#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

use pi_rs_host::{Host, HostConfig};
use std::io::{Read, Write};
use std::net::TcpListener;

#[test]
fn http_demo_performs_awaitable_get_through_public_surface() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut request = [0_u8; 4096];
        let count = stream.read(&mut request).unwrap();
        let request = String::from_utf8_lossy(&request[..count]);
        assert!(request.starts_with("GET /demo HTTP/1.1\r\n"));
        assert!(
            request
                .to_ascii_lowercase()
                .contains("accept: text/plain\r\n")
        );
        assert!(request.to_ascii_lowercase().contains("x-pi-demo: 1\r\n"));
        stream
            .write_all(b"HTTP/1.1 202 Accepted\r\ncontent-type: text/plain\r\ncontent-length: 5\r\nconnection: close\r\n\r\nhello")
            .unwrap();
    });

    let host = Host::new(HostConfig::default()).unwrap();
    host.load(
        "http-demo.lua",
        include_str!("../../../examples/extensions/http-demo.lua"),
    )
    .unwrap();
    let result = host
        .call_command("http-demo", &format!("http://{address}/demo"))
        .unwrap()
        .unwrap();

    assert_eq!(result["status"], 202);
    assert_eq!(result["ok"], true);
    assert_eq!(result["body"], "hello");
    assert_eq!(result["headers"]["content-type"], "text/plain");
    server.join().unwrap();
}
