extern crate bytes;
extern crate futures;
extern crate tokio_core;
extern crate tokio_io;
extern crate tokio_proto;
extern crate tokio_service;
extern crate tokio_timer;
extern crate byteorder;
#[macro_use]
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
use tokio_core::reactor::Timeout;

use tokio_timer::Timer;

use futures::Future;
use futures::{Sink, Stream};
use futures::future::ok;
use futures::sync::oneshot;
use futures::IntoFuture;
use futures::future::FutureResult;

const MAX_PACKET_BYTES: usize = 1220;

mod my_adapters {
	use futures::{Async, Stream, Poll};

	// From http://stackoverflow.com/a/42470352
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

fn delay_future(duration: Duration, handle: &Handle) -> futures::Flatten<FutureResult<tokio_core::reactor::Timeout, std::io::Error>> {
	Timeout::new(duration, &handle).into_future().flatten()
}

fn run_server(bind_addr: &str, server_port: u16, buf: Vec<u8>, num_clients: usize) {
	let mut recv_counts = vec![0 as u64; num_clients];
	let addr = SocketAddr::new(IpAddr::from_str(bind_addr).unwrap(), server_port);

	let mut core = Core::new().unwrap();
	let handle = core.handle();

	let socket = UdpSocket::bind(&addr, &handle).unwrap();
	println!("Listening on: {} with max of {} clients", addr, num_clients);

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

fn run_client(server_ip: &str, server_port: u16, buf: &Vec<u8>, index: u16, randomize_starts: bool, run_duration: Duration, tick_rate_hz: u32, timer: Timer, handle: Handle, result_tx: oneshot::Sender<(u16, u64, u64)>) {
	let addr = SocketAddr::new(IpAddr::from_str("127.0.0.1").unwrap(), 0);
	let server_addr = SocketAddr::new(IpAddr::from_str(server_ip).unwrap(), server_port);

	let socket = UdpSocket::bind(&addr, &handle).unwrap();

	let mut client_buf = buf.clone();
	LittleEndian::write_u16(&mut client_buf, index);

	let send_data = Rc::new((server_addr, client_buf));
	let (sink, stream) = socket.framed(ClientCodec).split();

	let send_interval_duration = Duration::from_millis(1000 / tick_rate_hz as u64);
	let wakeups = timer.interval(send_interval_duration);

	let send_stream = wakeups
		.map_err(|e| io::Error::new(io::ErrorKind::Other, e));

	let delay_ms: u64 = if randomize_starts {
		let mut rng = thread_rng();
		rng.gen_range(0, 1000)
	} else {
		0
	};

	let stop_signal = delay_future(run_duration + Duration::from_millis(delay_ms), &handle);
	let send_counter_future = my_adapters::stream_completion_pact(send_stream, stop_signal.into_stream())
		.fold((0 as u64, sink), move |(send_count, mut sink), _| {
			let _ = sink.start_send(send_data.clone()); // TODO - use this result

			// ok((send_count + 1, sink)) // This doesn't work, type inference fails
			ok::<_, io::Error>((send_count + 1, sink))
		})
		.map(|(send_count, mut sink)| {
			// Send any buffered output
			match sink.poll_complete() {
				Ok(_) => {}
				Err(e) => {
					println!("Error closing sink: {:?}", e);
				}
			}

			(send_count, sink)
		})
		.map_err(|_| ());

	let delayed_send_counter_future = delay_future(Duration::from_millis(delay_ms), &handle)
		.then(move |_| {
			send_counter_future
		});

	// Give the receive loop a bit of extra time to complete
	let read_timeout = delay_future(run_duration + Duration::from_millis(500) + Duration::from_millis(delay_ms), &handle);
	let read_counter_future = my_adapters::stream_completion_pact(stream, read_timeout.into_stream())
		.fold(0 as u64, move |recv_count, _| {
			// ok(recv_count + 1) // This doesn't work, type inference fails
			ok::<_, io::Error>(recv_count + 1)
		})
		.map_err(|_| ());

	let final_future = delayed_send_counter_future
		.join(read_counter_future)
		.then(move |result| {
			match result {
				Ok((send_count, read_count)) => {
					let _ = result_tx.send((index, send_count.0, read_count));
				}
				Err(e) => {
					println!("Error: {:?}", e);
					let _ = result_tx.send((index, 0, 0));
				}
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
		.arg(Arg::with_name("num-clients")
			.short("n")
			.long("num")
			.takes_value(true)
			.help("Number of clients to serve or create (default 128)"))
		.arg(Arg::with_name("duration")
			.short("d")
			.long("duration")
			.takes_value(true)
			.help("Number of seconds to run the clients (default 5)"))
		.arg(Arg::with_name("client_tick_rate")
			.short("t")
			.long("tickrate")
			.takes_value(true)
			.help("Frequency in Hz for clients to send packets (default 10)"))
		.arg(Arg::with_name("host")
			.short("h")
			.long("host")
			.takes_value(true)
			.help("Host IP for clients to connect to (default 127.0.0.1)"))
		.arg(Arg::with_name("port")
			.short("p")
			.long("port")
			.takes_value(true)
			.help("Port the server runs on (and the client connects to (default 55777)"))
		.arg(Arg::with_name("bind_addr")
			.short("b")
			.long("bind")
			.takes_value(true)
			.help("Adddress to bind the server to (default 0.0.0.0)"))
		.get_matches();

	let buf: Vec<u8> = (0..MAX_PACKET_BYTES).map(|n| n as u8).collect();

	let should_run_server = matches.is_present("as-server");
	let randomize_starts = matches.is_present("randomize-starts");
	let num_clients = value_t!(matches, "num-clients", usize).unwrap_or(128);
	let duration_seconds = value_t!(matches, "duration", u64).unwrap_or(5);
	let tick_rate_hz = value_t!(matches, "client_tick_rate", u32).unwrap_or(10);
	let server_host = value_t!(matches, "host", String).unwrap_or("127.0.0.1".to_string());
	let server_port = value_t!(matches, "port", u16).unwrap_or(55777);
	let bind_addr = value_t!(matches, "bind_addr", String).unwrap_or("0.0.0.0".to_string());

	let mut core = Core::new().unwrap();
	let handle = core.handle();

	if should_run_server {
		run_server(&bind_addr, server_port, buf, num_clients);
	} else {
		// This timer is shared so we don't create a thread per client
		let timer = tokio_timer::wheel().tick_duration(Duration::from_millis(10)).build();
		let run_duration = Duration::from_secs(duration_seconds);

		println!("Running {} clients at {} Hz for {} seconds with {} delay", num_clients, tick_rate_hz, duration_seconds, if randomize_starts {"randomized"} else {"no"});
		println!("Using server {}:{}", server_host, server_port);

		let mut client_chans = Vec::new();

		for n in 0..num_clients {
			let (tx, rx) = oneshot::channel::<_>(); // The client will send its result on this channel when finished
			client_chans.push(rx);

			run_client(&server_host, server_port, &buf, n as u16, randomize_starts, run_duration, tick_rate_hz, timer.clone(), handle.clone(), tx);
		}

		let client_results = client_chans.iter_mut().map(|rx| {
			rx.and_then(|(index, send_count, read_count)| {
				println!("Client {} done with result: Sent: {}, Received: {}", index, send_count, read_count);
				Ok(())
			})
		});

		let run_clients = futures::future::join_all(client_results);

		core.run(run_clients).unwrap();
	}
}
