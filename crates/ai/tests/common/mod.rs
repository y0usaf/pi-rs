#![allow(dead_code)]

//! A fake provider server: it records each request body and answers with scripted server-sent events.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};

use serde_json::Value;

pub struct FakeServer {
    pub base_url: String,
    bodies: Arc<Mutex<Vec<Value>>>,
}

impl FakeServer {
    /// Serve `events`, a list of `(event name, JSON data)`; an empty name writes a data-only event.
    pub fn start(events: Vec<(&str, &str)>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("the fake server binds");
        let port = listener.local_addr().expect("the fake server has an address").port();
        let bodies: Arc<Mutex<Vec<Value>>> = Arc::new(Mutex::new(Vec::new()));
        let script: Vec<(String, String)> = events
            .into_iter()
            .map(|(name, data)| (name.to_owned(), data.to_owned()))
            .collect();
        let served = Arc::clone(&bodies);
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let bodies = Arc::clone(&served);
                let script = script.clone();
                std::thread::spawn(move || {
                    if let Err(error) = serve(stream, &bodies, &script) {
                        eprintln!("fake server: {error}");
                    }
                });
            }
        });
        Self {
            base_url: format!("http://127.0.0.1:{port}/v1"),
            bodies,
        }
    }

    pub fn bodies(&self) -> Vec<Value> {
        self.bodies.lock().expect("the fake server records bodies").clone()
    }
}

fn serve(
    mut stream: TcpStream,
    bodies: &Mutex<Vec<Value>>,
    script: &[(String, String)],
) -> std::io::Result<()> {
    let body = read_request(&mut stream)?;
    bodies.lock().expect("the fake server records bodies").push(body);
    let mut events = String::new();
    for (name, data) in script {
        if !name.is_empty() {
            events.push_str(&format!("event: {name}\n"));
        }
        events.push_str(&format!("data: {data}\n\n"));
    }
    write!(
        stream,
        "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ncontent-length: {}\r\n\r\n{events}",
        events.len()
    )?;
    stream.flush()
}

fn read_request(stream: &mut TcpStream) -> std::io::Result<Value> {
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut line = String::new();
    reader.read_line(&mut line)?;
    let mut headers: Vec<(String, String)> = Vec::new();
    loop {
        line.clear();
        reader.read_line(&mut line)?;
        let trimmed = line.trim_end();
        if trimmed.is_empty() {
            break;
        }
        if let Some((name, value)) = trimmed.split_once(':') {
            headers.push((name.trim().to_ascii_lowercase(), value.trim().to_owned()));
        }
    }
    let length = headers
        .iter()
        .find(|(name, _)| name == "content-length")
        .and_then(|(_, value)| value.parse::<usize>().ok())
        .unwrap_or(0);
    let mut body = vec![0; length];
    reader.read_exact(&mut body)?;
    Ok(serde_json::from_slice(&body).unwrap_or(Value::Null))
}
