#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use pi_rs_host::{Host, HostConfig};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::thread;

/// A tiny line-based echo + raw-bytes echo server.
fn start_echo_server() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(stream) = stream else { continue };
            thread::spawn(move || handle_echo(stream));
        }
    });
    format!("{addr}")
}

fn handle_echo(mut stream: TcpStream) {
    let mut reader_stream = stream.try_clone().unwrap();
    let mut reader = BufReader::new(reader_stream.try_clone().unwrap());
    let mut line = String::new();
    // Read one line, echo it.
    if reader.read_line(&mut line).is_ok() && !line.is_empty() {
        stream.write_all(line.as_bytes()).unwrap();
        stream.flush().unwrap();
        let _ = &mut reader_stream;
    }
    // Read one raw byte repeated 5 times and echo them verbatim.
    let mut raw = [0_u8; 5];
    if reader_stream.read_exact(&mut raw).is_ok() {
        stream.write_all(&raw).unwrap();
        stream.flush().unwrap();
    }
}

fn host() -> Host {
    Host::new(HostConfig::default()).unwrap()
}

fn load_demo(host: &Host) {
    host.load(
        "tcp-demo.lua",
        include_str!("../../../examples/extensions/tcp-demo.lua"),
    )
    .unwrap();
}

#[test]
fn tcp_connects_reads_lines_and_bytes() {
    let addr = start_echo_server();
    let host = host();
    load_demo(&host);
    let out = host.call_command("tcp-demo", &addr).unwrap().unwrap();
    assert_eq!(out["echo"], "ping");
    assert_eq!(out["bytes"], "frame");
    assert_eq!(out["closed_before"], false);
    assert_eq!(out["closed_after"], true);
}

#[test]
fn tcp_close_deterministic() {
    let addr = start_echo_server();
    let host = host();
    load_demo(&host);
    let out = host.call_command("tcp-dispose", &addr).unwrap().unwrap();
    assert_eq!(out["echoed"], "opencheck");
    assert_eq!(out["closed"], true);
    assert_eq!(out["post_read_len"], 0);
}
