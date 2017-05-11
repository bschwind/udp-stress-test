extern crate bytes;
extern crate futures;
#[macro_use]
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
use std::{thread, time};
use std::rc::Rc;
use std::time::Duration;

use clap::{Arg, App, SubCommand};

use rand::{thread_rng, Rng};

use byteorder::{ByteOrder, LittleEndian};

use tokio_core::reactor::{Core, Handle};
use tokio_core::net::{UdpSocket, UdpCodec};

use tokio_timer::Timer;

use futures::{Future, Poll};
use futures::{Sink, Stream};
use futures::future::ok;
use futures::sync::oneshot;
use futures::sync::mpsc::UnboundedSender;
use futures::stream;
use futures::stream::Once;

const MAX_PACKET_BYTES: usize = 1220;
const MAX_CLIENTS: usize = 1024;
const SERVER_IP: &str = "127.0.0.1";
const SERVER_PORT: u16 = 55777;

mod CoolShit {
	use tokio_core::reactor::Core;
	use futures::sync::mpsc::unbounded;
	use tokio_core::net::TcpListener;
	use std::net::SocketAddr;
	use std::str::FromStr;
	use futures::{Async, Stream, Future, Poll};
	use std::thread;
	use std::time::Duration;

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
					Ok(Async::Ready(None)) // <<<<<< (3)
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

fn run_server(buf: Vec<u8>, core: Core) {
	let mut recv_counts = vec![0 as u64; MAX_CLIENTS];
	// let addr = SocketAddr::new(IpAddr::from_str("::1").unwrap(), 40000);
	let addr = SocketAddr::new(IpAddr::from_str(SERVER_IP).unwrap(), SERVER_PORT);

	let mut core = Core::new().unwrap();
	let handle = core.handle();

	let socket = UdpSocket::bind(&addr, &handle).unwrap();
	println!("Listening on: {}", addr);

	// Streams
	let (sink, stream) = socket.framed(ServerCodec).split();

	let print_addr_stream = stream.map(|(addr, msg)| {

		// let client_id = msg.get_u16::<LittleEndian>();
		let client_id = LittleEndian::read_u16(&msg);
		recv_counts[client_id as usize] += 1;

		println!("Client ID: {}", client_id);
		println!("{:?}", recv_counts);

		(addr, msg)
	});

	let echo_stream = print_addr_stream.forward(sink);

	core.run(echo_stream);
	// End Streams
}

fn run_client(buf: &Vec<u8>, index: u16, randomize_starts: bool, run_duration: Duration, timer: Timer, handle: Handle, stop_rx: oneshot::Receiver<()>) {
	let addr = SocketAddr::new(IpAddr::from_str("127.0.0.1").unwrap(), 0);
	let server_addr = SocketAddr::new(IpAddr::from_str(SERVER_IP).unwrap(), SERVER_PORT);

	let socket = UdpSocket::bind(&addr, &handle).unwrap();
	println!("Running on: {}", socket.local_addr().unwrap());

	let mut client_buf = buf.clone();
	LittleEndian::write_u16(&mut client_buf, index);

	let ret_val = Rc::new((server_addr, client_buf));

	// Streams
	let (sink, stream) = socket.framed(ClientCodec).split();

	let duration = Duration::from_millis(101); // 10 Hz
	let wakeups = timer.interval(duration);

	// let interval_send = wakeups
	// 	.map(move |_| {
	// 		ret_val.clone()
	// 	})
	// 	.map_err(|e| io::Error::new(io::ErrorKind::Other, e));

	let delay_ms = if randomize_starts {
		let mut rng = thread_rng();
		rng.gen_range(0, 1000)
	} else {
		0
	};

	let delay_future = timer.sleep(Duration::from_millis(delay_ms));

	// let interval_send_future = delay_future.then(|_| {
	// 	interval_send.forward(sink).map(|_| ()).map_err(|_| ())
	// });

	// let counter_future = stream
	// 	.for_each(move |_| {

	// 		// println!("Client {}: {}", index, recv_count);

	// 		Ok(())
	// 	})
	// 	.map_err(|_| {
	// 		println!("OH NO");
	// 	});

	// let final_future = counter_future
	// 	.select(interval_send_future)
	// 	.then(move |_| -> Result<(), ()> {
	// 		println!("Done");
	// 		Ok(())
	// 	});

	// let thing = timer.timeout(final_future, Duration::from_millis(1000)).then(move |hmm| {
	// 	println!("Really done!");

	// 	Err(())
	// });

	fn c(x: ()) {

	}

	// let mut dummy_stream_1: Once<u64, io::Error> = stream::once(Ok(17));
	// let mut dummy_stream_2: Once<u64, io::Error> = stream::once(Ok(17));

	let send_stream = wakeups
		.map_err(|e| io::Error::new(io::ErrorKind::Other, e));

	let send_counter_stream = CoolShit::stream_completion_pact(send_stream, stop_rx.into_stream())
	// let send_counter_stream = send_stream
		.fold((0 as u64, sink), move |(send_count, mut sink), _| {
			sink.start_send(ret_val.clone());
			ok::<_, io::Error>((send_count + 1, sink))
		})

		// .fold(0 as u64, move |send_count, _| {
		// 	println!("Adding!");
		// 	// ok(a + 1) // This doesn't work, type inference fails
		// 	ok::<_, io::Error>(send_count + 1)
		// })

		.map(|(send_count, mut sink)| {
			// c(sink)
			sink.poll_complete(); // Send any buffered output
			(send_count, sink)
		})
		.map_err(|_| ());
		// .then(move |result| -> Result<(), ()> {
		// 	// c(result);
		// 	println!("Client {} done with result: {:?}", index, result);
		// 	Ok(())
		// });

	let dummy_stream_2 = timer.sleep(run_duration + Duration::from_millis(500));
	let read_counter_stream = CoolShit::stream_completion_pact(stream, dummy_stream_2.into_stream())
	// let read_counter_stream = stream
		.fold(0 as u64, move |recv_count, _| {
			// ok(a + 1) // This doesn't work, type inference fails
			ok::<_, io::Error>(recv_count + 1)
		})
		// .map(|x| c(x))
		.map_err(|_| ());
		// .then(move |result| {
		// 	println!("Client {} done with result: {:?}", index, result);
		// 	Ok(())
		// });

		// .for_each(move |a| {
		// 	println!("{:?}", a);
		// 	Ok(())
		// })
		// .then(move |_| {
		// 	println!("Client {} done", index);
		// 	Ok(())
		// });


	let final_future = send_counter_stream
		.join(read_counter_stream)
		.then(move |result| {
			match result {
				Ok((send_count, read_count)) => println!("Client {} done with result: Sent: {}, Received: {}", index, send_count.0, read_count),
				Err(e) => println!("Error: {:?}", e)
			}

			Ok(())
		});

	handle.spawn(final_future);
	// End Streams
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
		run_server(buf, core);
	} else {
		let (forever_tx, forever_rx) = oneshot::channel::<i32>();

		let timer = tokio_timer::wheel().tick_duration(Duration::from_millis(30)).build();
		let run_duration = Duration::from_millis(1000);

		let mut client_chans = Vec::new();

		for n in 0..MAX_CLIENTS {
			let (tx, rx) = oneshot::channel::<()>(); // Channel of (client_index, receive_count, send_count)
			client_chans.push(tx);

			run_client(&buf, n as u16, randomize_starts, run_duration, timer.clone(), core.handle().clone(), rx);
		}

		let client_delay_timeout = timer.sleep(run_duration).and_then(|_| {
			println!("Done!");
			
			for transmitter in client_chans {
				transmitter.send(());
				// let thing = rx.wait();
				// println!("{:?}", thing);
			}

			Ok(())
		});

		core.run(client_delay_timeout).unwrap();

		core.run(forever_rx).unwrap();
	}
}
