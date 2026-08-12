#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use pi_rs_host::{Host, HostConfig};
use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::mpsc;
use std::thread;

/// A tiny HTTP server that serves a body in two chunks (deliberately
/// Transfer-Encoding: chunked so the client sees incremental chunks).
fn serve_chunked(body: &'static str) -> (String, mpsc::Receiver<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut request = [0_u8; 4096];
        let _ = stream.read(&mut request);
        tx.send(()).unwrap();
        // Chunked response: two chunks so stream() yields two on_chunk calls.
        let half = body.len() / 2;
        let head = &body[..half];
        let tail = &body[half..];
        let _ = write!(
            stream,
            "HTTP/1.1 200 OK\r\ncontent-type: text/plain\r\ntransfer-encoding: chunked\r\nconnection: close\r\n\r\n{:x}\r\n{}\r\n{:x}\r\n{}\r\n0\r\n\r\n",
            head.len(),
            head,
            tail.len(),
            tail
        );
        let _ = stream.flush();
    });
    (format!("http://{address}/stream"), rx)
}

#[test]
fn http_stream_yields_chunks_and_returns_headers() {
    let (url, _server_ready) = serve_chunked("hello world of streaming bodies");
    let host = Host::new(HostConfig::default()).unwrap();
    host.load(
        "http-stream-demo.lua",
        include_str!("../../../examples/extensions/http-stream-demo.lua"),
    )
    .unwrap();
    let result = host
        .call_command("http-stream-demo", &url)
        .unwrap()
        .unwrap();
    assert_eq!(result["status"], 200);
    assert_eq!(result["ok"], true);
    assert_eq!(result["body"], "hello world of streaming bodies");
    assert!(result["chunk_count"].as_u64().unwrap() >= 2);
}

#[test]
fn http_fetch_posts_body_and_returns_response() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let got = std::sync::Arc::new(std::sync::Mutex::new(String::new()));
    let got2 = got.clone();
    thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut request = [0_u8; 4096];
        let count = stream.read(&mut request).unwrap();
        let req = String::from_utf8_lossy(&request[..count]).into_owned();
        *got2.lock().unwrap() = req;
        stream
            .write_all(b"HTTP/1.1 201 Created\r\ncontent-type: application/json\r\ncontent-length: 2\r\nconnection: close\r\n\r\nok")
            .unwrap();
    });
    let host = Host::new(HostConfig::default()).unwrap();
    host.load(
        "http-stream-demo.lua",
        include_str!("../../../examples/extensions/http-stream-demo.lua"),
    )
    .unwrap();
    let result = host
        .call_command("http-fetch-demo", &format!("http://{address}/x"))
        .unwrap()
        .unwrap();
    assert_eq!(result["status"], 201);
    assert_eq!(result["ok"], true);
    assert_eq!(result["body"], "ok");
    let request = got.lock().unwrap();
    assert!(request.starts_with("POST /x HTTP/1.1\r\n"));
    assert!(
        request
            .to_ascii_lowercase()
            .contains("content-type: application/json\r\n")
    );
    assert!(request.ends_with("\r\n\r\n{\"hello\":\"world\"}"));
}

/// Serve a leading chunk then stall (never finish the body) so an abort
/// mid-stream is observable.
fn serve_stalling() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    thread::spawn(move || {
        if let Ok((mut stream, _)) = listener.accept() {
            let mut request = [0_u8; 4096];
            let _ = stream.read(&mut request);
            let _ = write!(
                stream,
                "HTTP/1.1 200 OK\r\ncontent-type: text/plain\r\ntransfer-encoding: chunked\r\n\r\n5\r\nearly\r\n"
            );
            let _ = stream.flush();
            // Stall forever; the client must abort.
            thread::park();
        }
    });
    format!("http://{address}/stall")
}

#[test]
fn http_stream_aborts_in_flight_request() {
    let url = serve_stalling();
    let host = Host::new(HostConfig::default()).unwrap();
    host.load(
        "http-stream-demo.lua",
        include_str!("../../../examples/extensions/http-stream-demo.lua"),
    )
    .unwrap();
    let result = host
        .call_command("http-stream-abort", &url)
        .unwrap()
        .unwrap();
    assert_eq!(result["aborted"], true);
    assert_eq!(result["received"], "early");
}
