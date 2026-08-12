#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

//! File-backed exercisers for every PLAN 9.9 mechanism, plus disposal/leak
//! contracts: a process, socket, or file watcher disposed through its handle
//! leaves nothing alive, and no timer survives being cleared or its dispatch.

use pi_rs_host::{Host, HostConfig};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::thread;

fn host() -> Host {
    Host::new(HostConfig::default()).unwrap()
}

#[test]
fn every_mechanism_example_loads_and_runs() {
    let host = host();
    // Load every new file-backed exerciser as an ordinary extension.
    for name in [
        "crypto-demo.lua",
        "http-stream-demo.lua",
        "process-demo.lua",
        "tcp-demo.lua",
        "fs-advanced.lua",
        "timers-demo.lua",
    ] {
        host.load(
            name,
            &std::fs::read_to_string(format!(
                "{}/../../examples/extensions/{}",
                env!("CARGO_MANIFEST_DIR"),
                name
            ))
            .unwrap(),
        )
        .unwrap();
    }
    // Registering all six at once must not conflict.
    assert!(!host.commands().unwrap().is_empty());
}

fn start_echo_server() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut line = String::new();
        {
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            if reader.read_line(&mut line).is_ok() {
                stream.write_all(line.as_bytes()).unwrap();
                stream.flush().unwrap();
            }
        }
        let mut raw = [0_u8; 5];
        let _ = stream.read(&mut raw);
    });
    format!("{addr}")
}

#[test]
fn disposed_socket_and_timer_leave_nothing_alive() {
    let addr = start_echo_server();
    let host = host();
    host.load(
        "leak-check.lua",
        r#"
        local pi = ...
        pi.register_command("leak", {
            handler = function(arg)
                local host, port = arg:match("^(%S+):(%d+)$")
                local socket = pi.tcp.connect(host, tonumber(port), { timeout_ms = 2000 })
                socket:write("hi\n")
                socket:read_line()
                socket:close()
                -- After close, reads are empty and the socket reports closed.
                local closed = socket:is_closed()
                local post = socket:read(2)
                -- A cleared timer never fires and reports nothing further.
                local fired = false
                local t = pi.set_timeout(5, function() fired = true end)
                pi.clear_timeout(t)
                pi.sleep(30)
                return { closed = closed, post_len = #post, cleared_fired = fired }
            end,
        })
        "#,
    )
    .unwrap();
    let out = host.call_command("leak", &addr).unwrap().unwrap();
    assert_eq!(out["closed"], true);
    assert_eq!(out["post_len"], 0);
    assert_eq!(out["cleared_fired"], false);
}

#[test]
fn disposed_process_is_reaped() {
    let host = host();
    host.load(
        "process-leak.lua",
        r#"
        local pi = ...
        pi.register_command("leak", {
            handler = function()
                local p = pi.process.spawn("sh", { "-c", "sleep 60" })
                local pid = p:pid()
                p:dispose()
                local code = p:wait()
                return { pid = pid, terminated = code == nil }
            end,
        })
        "#,
    )
    .unwrap();
    let out = host.call_command("leak", "").unwrap().unwrap();
    assert!(out["pid"].as_u64().unwrap() > 0);
    assert_eq!(out["terminated"], true);
}

#[test]
fn disposed_file_watcher_stops() {
    let dir = std::env::temp_dir().join(format!("pi-rs-watch-stop-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("w.txt");
    std::fs::write(&file, "v1").unwrap();
    let host = host();
    host.load(
        "fs-watcher-leak.lua",
        &format!(
            r#"
            local pi = ...
            pi.register_command("leak", {{
                handler = function()
                    local events = 0
                    local watch = pi.fs.watch_file("{}", function() events = events + 1 end)
                    watch:close()
                    -- A closed watcher must not fire even if the file changes.
                    pi.fs.write_file("{}", "v2")
                    pi.sleep(350)
                    local still = watch:poll()
                    return {{ events = events, after_close_fired = still }}
                end,
            }})
            "#,
            file.display(),
            file.display()
        ),
    )
    .unwrap();
    let out = host.call_command("leak", "").unwrap().unwrap();
    // The change was made after close; the poller must not report it as new.
    assert_eq!(out["events"], 0);
    assert_eq!(out["after_close_fired"], false);
    let _ = std::fs::remove_dir_all(&dir);
}
