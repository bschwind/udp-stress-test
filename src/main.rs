extern crate bytes;
extern crate futures;
extern crate tokio_core;
extern crate tokio_io;
extern crate tokio_proto;
extern crate tokio_service;
extern crate tokio_timer;
extern crate byteorder;
extern crate clap;
extern crate rand;

use std::io;
use std::net::IpAddr;
use std::net::SocketAddr;
use std::str::FromStr;
use std::rc::Rc;
use std::time::Duration;

use clap::{Arg, App};

use rand::{thread_rng, Rng};

use byteorder::{ByteOrder, LittleEndian};

use tokio_core::reactor::{Core, Handle};
use tokio_core::net::{UdpSocket, UdpCodec};

use tokio_timer::Timer;

use futures::Future;
use futures::{Sink, Stream};
use futures::future::ok;
use futures::sync::oneshot;

const MAX_PACKET_BYTES: usize = 1220;
const MAX_CLIENTS: usize = 128;
const SERVER_IP: &str = "127.0.0.1";
const SERVER_PORT: u16 = 55777;

mod my_adapters {
	use futures::{Async, Stream, Poll};

	pub struct CompletionPact<S, C>
		where S: Stream,
			  C: Stream, 
	{
		stream: S,
		completer: C,
	}

	pub fn stream_completion_pact<S, C>(s: S, c: C) -> CompletionPact<S, C>
		where S: Stream,
			  C: Stream,
	{
		CompletionPact {
			stream: s,
			completer: c,
		}
	}

	impl<S, C> Stream for CompletionPact<S, C>
		where S: Stream,
			  C: Stream,
	{
		type Item = S::Item;
		type Error = S::Error;

		fn poll(&mut self) -> Poll<Option<S::Item>, S::Error> {
			match self.completer.poll() {
				Ok(Async::Ready(None)) |
				Err(_) |
				Ok(Async::Ready(Some(_))) => {
					// We are done, forget us
					Ok(Async::Ready(None))
				},
				Ok(Async::NotReady) => {
					self.stream.poll()
				},
			}
		}
	}
}

pub struct ServerCodec;
pub struct ClientCodec;

impl UdpCodec for ServerCodec {
	type In = (SocketAddr, Vec<u8>);
	type Out = (SocketAddr, Vec<u8>);

	fn decode(&mut self, addr: &SocketAddr, buf: &[u8]) -> Result<Self::In, io::Error> {
		Ok((*addr, buf.to_vec()))
	}

	fn encode(&mut self, (addr, buf): Self::Out, into: &mut Vec<u8>) -> SocketAddr {
		into.extend(buf);
		addr
	}
}

impl UdpCodec for ClientCodec {
	type In = (SocketAddr, Vec<u8>);
	type Out = Rc<(SocketAddr, Vec<u8>)>;

	fn decode(&mut self, addr: &SocketAddr, buf: &[u8]) -> Result<Self::In, io::Error> {
		Ok((*addr, buf.to_vec()))
	}

	fn encode(&mut self, stuff: Self::Out, into: &mut Vec<u8>) -> SocketAddr {
		let addr = stuff.0;
		let buf = &stuff.1;
		into.extend(buf);
		addr
	}
}

fn run_server(buf: Vec<u8>) {
	let mut recv_counts = vec![0 as u64; MAX_CLIENTS];
	let addr = SocketAddr::new(IpAddr::from_str(SERVER_IP).unwrap(), SERVER_PORT);

	let mut core = Core::new().unwrap();
	let handle = core.handle();

	let socket = UdpSocket::bind(&addr, &handle).unwrap();
	println!("Listening on: {}", addr);

	let (sink, stream) = socket.framed(ServerCodec).split();

	let print_addr_stream = stream.map(|(addr, msg)| {
		let client_id = LittleEndian::read_u16(&msg);
		recv_counts[client_id as usize] += 1;

		(addr, msg) // TODO - send buf instead of echoing
	});

	let echo_stream = print_addr_stream.forward(sink).and_then(|_| Ok(()));
	let server = core.run(echo_stream);

	println!("Result: {:?}", server);
}

fn run_client(buf: &Vec<u8>, index: u16, randomize_starts: bool, run_duration: Duration, timer: Timer, handle: Handle, stop_rx: oneshot::Receiver<()>) {
	let addr = SocketAddr::new(IpAddr::from_str("127.0.0.1").unwrap(), 0);
	let server_addr = SocketAddr::new(IpAddr::from_str(SERVER_IP).unwrap(), SERVER_PORT);

	let socket = UdpSocket::bind(&addr, &handle).unwrap();

	let mut client_buf = buf.clone();
	LittleEndian::write_u16(&mut client_buf, index);

	let ret_val = Rc::new((server_addr, client_buf));
	let (sink, stream) = socket.framed(ClientCodec).split();

	let duration = Duration::from_millis(40); // 10 Hz
	let wakeups = timer.interval(duration);

	let delay_ms = if randomize_starts {
		let mut rng = thread_rng();
		rng.gen_range(0, 1000)
	} else {
		0
	};

	let send_stream = wakeups
		.map_err(|e| io::Error::new(io::ErrorKind::Other, e));

	let send_counter_future = my_adapters::stream_completion_pact(send_stream, stop_rx.into_stream())
		.fold((0 as u64, sink), move |(send_count, mut sink), _| {
			let _ = sink.start_send(ret_val.clone()); // TODO - use this result

			// ok((send_count + 1, sink)) // This doesn't work, type inference fails
			ok::<_, io::Error>((send_count + 1, sink))
		})
		.map(|(send_count, mut sink)| {
			sink.poll_complete(); // Send any buffered output
			(send_count, sink)
		})
		.map_err(|_| ());


	let delayed_send_counter_future = timer.sleep(Duration::from_millis(delay_ms))
		.then(move |_| {
			send_counter_future
		});

	let dummy_stream_2 = timer.sleep(run_duration + Duration::from_millis(500));
	let read_counter_future = my_adapters::stream_completion_pact(stream, dummy_stream_2.into_stream())
		.fold(0 as u64, move |recv_count, _| {
			// ok(recv_count + 1) // This doesn't work, type inference fails
			ok::<_, io::Error>(recv_count + 1)
		})
		.map_err(|_| ());

	let final_future = delayed_send_counter_future
		.join(read_counter_future)
		.then(move |result| {
			match result {
				Ok((send_count, read_count)) => println!("Client {} done with result: Sent: {}, Received: {}", index, send_count.0, read_count),
				Err(e) => println!("Error: {:?}", e)
			}

			Ok(())
		});

	handle.spawn(final_future);
}

fn main() {
	let matches = App::new("UDP benchmark")
		.version("1.0")
		.about("Tests UDP throughput with Tokio UDPSockets")
		.arg(Arg::with_name("as-server")
			.short("s")
			.long("as-server")
			.help("If set, runs the UDP server instead of the clients"))
		.arg(Arg::with_name("randomize-starts")
			.short("r")
			.long("randomize")
			.help("If set, randomizes the start times of the clients"))
		.get_matches();

	let buf: Vec<u8> = (0..MAX_PACKET_BYTES).map(|n| n as u8).collect();

	let should_run_server = matches.is_present("as-server");
	let randomize_starts = matches.is_present("randomize-starts");

	let mut core = Core::new().unwrap();

	if should_run_server {
		run_server(buf);
	} else {
		let timer = tokio_timer::wheel().tick_duration(Duration::from_millis(30)).build();
		let run_duration = Duration::from_millis(3000);

		let mut client_chans = Vec::new();

		for n in 0..MAX_CLIENTS {
			let (tx, rx) = oneshot::channel::<()>(); // Channel of (client_index, receive_count, send_count)
			client_chans.push(tx);

			run_client(&buf, n as u16, randomize_starts, run_duration, timer.clone(), core.handle().clone(), rx);
		}

		let client_delay_timeout = timer.sleep(run_duration).and_then(|_| {
			println!("Done!");
			
			for transmitter in client_chans {
				let _ = transmitter.send(());
			}

			Ok(())
		});

		core.run(client_delay_timeout).unwrap();

		// Give the clients some time to print their results
		// TODO - make this more robust
		let client_delay_timeout = timer.sleep(Duration::from_millis(1000));

		core.run(client_delay_timeout).unwrap();
	}
}
