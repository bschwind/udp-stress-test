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

use clap::{Arg, App, SubCommand};

use rand::{thread_rng, Rng};

use byteorder::{ByteOrder, LittleEndian};

use tokio_core::reactor::{Core, Handle};
use tokio_core::net::{UdpSocket, UdpCodec};

use tokio_timer::Timer;

use futures::{Future, Poll};
use futures::{Sink, Stream};
use futures::sync::oneshot;
use futures::sync::mpsc::UnboundedSender;

const MAX_PACKET_BYTES: usize = 1220;
const MAX_CLIENTS: usize = 128;
const SERVER_IP: &str = "127.0.0.1";
const SERVER_PORT: u16 = 55777;

struct NetcodeConn {
	socket: UdpSocket,
	buf: Vec<u8>
}

struct Server {
	conn: NetcodeConn,
	recv_counts: Vec<u64>,
	to_send: Option<SocketAddr>
}

struct Client {
	conn: NetcodeConn,
	recv_count: u64,
	server_addr: SocketAddr
}

impl Future for Server {
	type Item = ();
	type Error = io::Error;

	fn poll(&mut self) -> Poll<(), io::Error> {
		loop {
			if let Some(send_to_addr) = self.to_send {
				let amount = try_nb!(self.conn.socket.send_to(&self.conn.buf, &send_to_addr));
				println!("Sent {} bytes", amount);
				self.to_send = None;
			}

			let (count, from_addr) = try_nb!(self.conn.socket.recv_from(&mut self.conn.buf));

			let client_id = LittleEndian::read_u16(&self.conn.buf);
			println!("Client ID: {}", client_id);

			self.recv_counts[client_id as usize] += 1;
			self.to_send = Some(from_addr);

			// println!("{:?}", self.conn.buf);
			println!("{:?}", self.recv_counts);
		}
	}
}

impl Future for Client {
	type Item = ();
	type Error = io::Error;

	fn poll(&mut self) -> Poll<(), io::Error> {
		loop {
			let amount = try_nb!(self.conn.socket.send_to(&self.conn.buf, &self.server_addr));
			println!("Sent {} bytes", amount);

			let (count, from_addr) = try_nb!(self.conn.socket.recv_from(&mut self.conn.buf));

			// // println!("Client ID: {}", client_id);

			self.recv_count += 1;
			println!("Count is {}", self.recv_count);
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

	// Old impl
	// let conn = NetcodeConn {
	// 	socket: socket,
	// 	buf: buf
	// };

	// let server = Server {
	// 	conn: conn,
	// 	recv_counts: recv_counts,
	// 	to_send: None
	// };
	// End old impl

	// Streams
	// let (tx, rx) = futures::sync::mpsc::unbounded();
	let (sink, stream) = socket.framed(ServerCodec).split();

	let print_addr_stream = stream.map(|(addr, msg)| {
		// println!("lol: {:?}", addr);
		// println!("lol: {:?}", msg);

		// let client_id = msg.get_u16::<LittleEndian>();
		let client_id = LittleEndian::read_u16(&msg);
		recv_counts[client_id as usize] += 1;

		println!("Client ID: {}", client_id);
		println!("{:?}", recv_counts);

		// sink.send((addr, vec!(3, 2, 1)));
		// tx.send((addr, msg));

		// Ok(())
		(addr, msg)
	});

	// let echo_stream = sink.send_all(print_addr_stream);
	let echo_stream = print_addr_stream.forward(sink);

	core.run(echo_stream);
	// End Streams

	// core.run(server).unwrap();
}

fn run_client(buf: &Vec<u8>, index: u16, randomize_starts: bool, timer: Timer, handle: Handle, tx: oneshot::Sender<(u16, u64, u64)>) {
	let addr = SocketAddr::new(IpAddr::from_str("127.0.0.1").unwrap(), 0);
	let server_addr = SocketAddr::new(IpAddr::from_str(SERVER_IP).unwrap(), SERVER_PORT);

	// let mut core = Core::new().unwrap();
	// let handle = core.handle();

	let socket = UdpSocket::bind(&addr, &handle).unwrap();
	println!("Running on: {}", socket.local_addr().unwrap());

	let mut client_buf = buf.clone();
	LittleEndian::write_u16(&mut client_buf, index);
	let mut recv_count: u64 = 0;

	let ret_val = Rc::new((server_addr, client_buf));

	// Old impl
	// let conn = NetcodeConn {
	// 	socket: socket,
	// 	buf: client_buf
	// };

	// let client = Client {
	// 	conn: conn,
	// 	recv_count: 0,
	// 	server_addr: server_addr
	// };
	// let duration = time::Duration::from_millis(100); // 10 Hz
	// let wakeups = timer.interval(duration);

	// let whatever = wakeups
	// .and_then(|_| {
	// 	println!("WAKE UP!");

	// 	Ok(())
	// })
	// .for_each(|_| {
	// 	Ok(())
	// })
	// .then(|_| Ok(()));

	// handle.spawn(whatever);

	// let future_chain = client
	// 	.then(|_| Ok(()));
		// .map(|_| ())
		// .map_err(|e| io::Error::new(io::ErrorKind::Other, e))

	// println!("{:?}", buf);

	// core.run(client).unwrap();
	// handle.spawn(future_chain);
	// End Old impl

	// Streams
	let (sink, stream) = socket.framed(ClientCodec).split();

	// let duration = time::Duration::from_millis(100); // 10 Hz
	let duration = time::Duration::from_millis(101); // 10 Hz
	let wakeups = timer.interval(duration);

	let interval_send = wakeups
		.map(move |_| {
			ret_val.clone()
		})
		.map_err(|e| io::Error::new(io::ErrorKind::Other, e));

	let delay_ms = if randomize_starts {
		let mut rng = thread_rng();
		rng.gen_range(0, 1000)
	} else {
		0
	};

	let delay_future = timer.sleep(time::Duration::from_millis(delay_ms));

	let interval_send_future = delay_future.then(|_| {
		interval_send.forward(sink).map(|_| ()).map_err(|_| ())
	});

	let counter_future = stream
		.for_each(move |_| {
			recv_count += 1;

			println!("Client {}: {}", index, recv_count);

			Ok(())
		})
		.map_err(|_| {
			println!("OH NO");
		});

	let final_future = counter_future
		.select(interval_send_future)
		.then(move |_| -> Result<u64, ()> {
			println!("Done");
			// Ok(())
			tx.send((index, recv_count, 0));
			Ok(17)
		});

	let thing = timer.timeout(final_future, time::Duration::from_millis(1000)).then(move |hmm| {
		println!("Really done!");

		println!("hmm: {:?}", hmm);

		Err(())
	});

	handle.spawn(thing);
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
		let (tx, rx) = oneshot::channel::<i32>();

		// let timer = Timer::default(); // Share the timer or else you'll get a thread per client
		let timer = tokio_timer::wheel().tick_duration(time::Duration::from_millis(30)).build();

		let mut client_chans = Vec::new();

		for n in 0..MAX_CLIENTS {
			let (tx, rx) = oneshot::channel::<(u16, u64, u64)>(); // Channel of (client_index, receive_count, send_count)
			client_chans.push(rx);

			run_client(&buf, n as u16, randomize_starts, timer.clone(), core.handle().clone(), tx);
		}

		let client_delay_timeout = timer.sleep(time::Duration::from_millis(1000)).and_then(|_| {
			println!("Done!");
			
			for rx in client_chans {
				let thing = rx.wait();
				println!("{:?}", thing);
			}

			Ok(())
		});

		core.run(rx).unwrap();
	}
}
