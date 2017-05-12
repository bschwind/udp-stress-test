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
    udp-fun [FLAGS]

FLAGS:
    -s, --as-server    If set, runs the UDP server instead of the clients
    -h, --help         Prints help information
    -r, --randomize    If set, randomizes the start times of the clients
    -V, --version      Prints version information
```
