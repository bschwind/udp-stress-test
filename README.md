# udp-stress-test
A UDP client/server stress test based on Tokio

## Build
`$ cargo build --release`

## Run Server
`$ cargo run --release -- s`

## Run Clients
`$ cargo run --release`

## Run Clients with random start time
`$ cargo run --release -r `

## Usage
```
UDP benchmark 1.0
Tests UDP throughput with Tokio UDPSockets

USAGE:
    udp-fun [FLAGS] [OPTIONS]

FLAGS:
    -s, --as-server    If set, runs the UDP server instead of the clients
        --help         Prints help information
    -r, --randomize    If set, randomizes the start times of the clients
    -V, --version      Prints version information

OPTIONS:
    -b, --bind <bind_addr>               Adddress to bind the server to (default 0.0.0.0)
    -t, --tickrate <client_tick_rate>    Frequency in Hz for clients to send packets (default 10)
    -d, --duration <duration>            Number of seconds to run the clients (default 5)
    -h, --host <host>                    Host IP for clients to connect to (default 127.0.0.1)
    -n, --num <num-clients>              Number of clients to serve or create (default 128)
    -p, --port <port>                    Port the server runs on (and the client connects to (default 55777)
```
