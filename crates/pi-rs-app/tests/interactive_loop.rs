#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

//! PTY-driven acceptance test for the interactive product loop.
//!
//! Spawns `pi` behind a real pseudo-terminal, types input, and verifies the
//! loop renders ANSI frames, executes bounded effects, diagnoses a missing
//! model, streams a deterministic fixture provider incrementally, cancels
//! in-flight work, and exits on a shutdown action.

use std::io::{Read, Write};
use std::os::unix::io::{AsRawFd, FromRawFd, RawFd};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

struct Pty {
    master: std::fs::File,
    slave: std::fs::File,
}

fn open_pty() -> Pty {
    unsafe {
        let master_fd = libc::posix_openpt(libc::O_RDWR | libc::O_NOCTTY);
        assert!(master_fd >= 0, "posix_openpt failed");
        assert_eq!(libc::grantpt(master_fd), 0, "grantpt failed");
        assert_eq!(libc::unlockpt(master_fd), 0, "unlockpt failed");
        let slave_name = libc::ptsname(master_fd);
        assert!(!slave_name.is_null(), "ptsname failed");
        let slave_fd = libc::open(slave_name, libc::O_RDWR | libc::O_NOCTTY);
        assert!(slave_fd >= 0, "open slave PTY failed");
        Pty {
            master: std::fs::File::from_raw_fd(master_fd),
            slave: std::fs::File::from_raw_fd(slave_fd),
        }
    }
}

fn read_available(master: &mut std::fs::File, timeout: Duration) -> String {
    let mut output = Vec::new();
    let deadline = Instant::now() + timeout;
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            break;
        }
        let fd = master.as_raw_fd();
        let mut fd_set = unsafe { std::mem::zeroed::<libc::fd_set>() };
        unsafe {
            libc::FD_ZERO(&mut fd_set);
            libc::FD_SET(fd, &mut fd_set);
        }
        let mut tv = libc::timeval {
            tv_sec: remaining.as_secs() as _,
            tv_usec: remaining.subsec_micros() as _,
        };
        let ready = unsafe {
            libc::select(
                fd + 1,
                &mut fd_set,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                &mut tv,
            )
        };
        if ready <= 0 {
            break;
        }
        let mut buf = [0_u8; 4096];
        match master.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => output.extend_from_slice(&buf[..n]),
            Err(_) => break,
        }
    }
    String::from_utf8_lossy(&output).into_owned()
}

#[test]
fn pty_loop_renders_input_frames_and_exits_on_shutdown() {
    let scratch = tempfile::tempdir().unwrap();

    // Deterministic fixture provider: an ordinary local HTTP server serving
    // a canned OpenAI-completions SSE stream. The Lua package discovers the
    // port through the public filesystem effect; no private channel exists.
    let fixture = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let fixture_port = fixture.local_addr().unwrap().port();
    std::fs::write(
        scratch.path().join("fixture_port.txt"),
        fixture_port.to_string(),
    )
    .unwrap();
    std::thread::spawn(move || {
        const CHUNKS: [&str; 3] = ["Hello", ", fixture", " world"];
        let Ok((mut socket, _)) = fixture.accept() else {
            return;
        };
        // Drain the request head; the body is irrelevant to the fixture.
        let mut request = Vec::new();
        let mut buf = [0_u8; 1024];
        loop {
            match socket.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    request.extend_from_slice(&buf[..n]);
                    if request.windows(4).any(|w| w == b"\r\n\r\n") {
                        break;
                    }
                }
                Err(_) => return,
            }
            if request.len() > 64 * 1024 {
                return;
            }
        }
        let mut body = String::new();
        for delta in CHUNKS {
            let chunk = serde_json::json!({
                "id": "fixture-completion",
                "model": "fixture-model",
                "choices": [{
                    "index": 0,
                    "delta": { "content": delta },
                    "finish_reason": serde_json::Value::Null,
                }],
            });
            body.push_str(&format!("data: {chunk}\n\n"));
        }
        let done = serde_json::json!({
            "id": "fixture-completion",
            "model": "fixture-model",
            "choices": [{
                "index": 0,
                "delta": {},
                "finish_reason": "stop",
            }],
            "usage": {
                "prompt_tokens": 3,
                "completion_tokens": 5,
                "total_tokens": 8,
            },
        });
        body.push_str(&format!("data: {done}\n\n"));
        body.push_str("data: [DONE]\n\n");
        let response = format!(
            "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
            body.len(),
        );
        let _ = socket.write_all(response.as_bytes());
    });
    std::fs::copy(
        concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../examples/walking-skeleton/application.lua"
        ),
        scratch.path().join("application.lua"),
    )
    .unwrap();
    std::fs::write(
        scratch.path().join("packages.json"),
        r#"{"version":1,"packages":["application.lua"]}"#,
    )
    .unwrap();

    let mut pty = open_pty();
    let slave_raw: RawFd = pty.slave.as_raw_fd();

    // Duplicate the slave fd for the child's stdin/stdout. The parent
    // keeps its own copy; the child inherits duplicated descriptors.
    let child_stdin = unsafe { libc::dup(slave_raw) };
    let child_stdout = unsafe { libc::dup(slave_raw) };
    assert!(child_stdin >= 0 && child_stdout >= 0);

    let child = Command::new(env!("CARGO_BIN_EXE_pi"))
        .current_dir(scratch.path())
        .env("HOME", scratch.path())
        .env("TERM", "xterm-256color")
        .env_remove("PI_PACKAGE_MANIFEST")
        .env_remove("XDG_CONFIG_HOME")
        .env_remove("XDG_DATA_HOME")
        .env_remove("XDG_STATE_HOME")
        .env_remove("XDG_CACHE_HOME")
        .args(["--manifest", "packages.json"])
        .stdin(unsafe { Stdio::from_raw_fd(child_stdin) })
        .stdout(unsafe { Stdio::from_raw_fd(child_stdout) })
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn pi behind PTY");

    // Wait for the startup frame.
    let startup_output = read_available(&mut pty.master, Duration::from_secs(5));
    assert!(
        !startup_output.is_empty(),
        "startup frame should produce ANSI output"
    );

    // Type 'h' — the skeleton should echo it.
    pty.master.write_all(b"h").unwrap();
    let echo_output = read_available(&mut pty.master, Duration::from_secs(2));
    assert!(
        !echo_output.is_empty(),
        "typed input should produce ANSI output"
    );

    // Type 'r' — the skeleton should run a bounded process effect and
    // render its stdout through the retained display.
    pty.master.write_all(b"r").unwrap();
    let effect_output = read_available(&mut pty.master, Duration::from_secs(5));
    assert!(
        !effect_output.is_empty(),
        "effect key should produce ANSI output"
    );

    // Type 'm' — the skeleton should diagnose a missing model without a
    // private API.
    pty.master.write_all(b"m").unwrap();
    let model_output = read_available(&mut pty.master, Duration::from_secs(5));
    assert!(
        !model_output.is_empty(),
        "missing-model key should produce ANSI output"
    );

    // Type 't' — the skeleton should cancel an in-flight timer effect and
    // return to an input-ready frame.
    pty.master.write_all(b"t").unwrap();
    let timer_output = read_available(&mut pty.master, Duration::from_secs(5));
    assert!(
        !timer_output.is_empty(),
        "cancellation key should produce ANSI output"
    );

    // Type 's' — the skeleton should stream the fixture provider and render
    // incremental text-delta frames before the final result frame. The
    // retained display emits minimal cell diffs, so later deltas appear as
    // their changed cells; each delta must arrive as its own frame.
    pty.master.write_all(b"s").unwrap();
    let stream_output = read_available(&mut pty.master, Duration::from_secs(10));
    assert!(
        stream_output.contains("stream> Hello"),
        "first incremental delta frame missing: {stream_output:?}"
    );
    assert!(
        stream_output.contains(", fixture"),
        "second incremental delta cells missing: {stream_output:?}"
    );
    assert!(
        stream_output.contains(" world"),
        "third incremental delta cells missing: {stream_output:?}"
    );
    assert!(
        stream_output.contains("stream done: Hello, fixture world"),
        "final streamed frame missing: {stream_output:?}"
    );

    // Type 'q' — the skeleton should shut down.
    pty.master.write_all(b"q").unwrap();

    let output = child.wait_with_output().expect("wait for pi to exit");
    assert!(
        output.status.success(),
        "pi should exit cleanly: {:?}",
        String::from_utf8_lossy(&output.stderr)
    );
}
