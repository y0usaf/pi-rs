//! PLAN 9.9 mechanism supersurface acceptance.
//!
//! Proves the Lua-native mechanism bindings through the file-backed
//! exerciser (examples/extensions/mechanism-demo.lua) plus direct Rust
//! assertions for the leak contracts:
//! - pi.crypto: reviewed hashes (sha256 vector, xxhash32, random_uuid,
//!   streaming create_hash) and pi.buffer binary ops
//! - pi.timer: set/clear timeout+interval; dispose_all leaves none
//! - pi.fs: watch/atomic/symlink/lstat/chmod/rename/rm/mkdtemp/access/
//!   constants/copy/open
//! - pi.net: TCP framed client against a real local server; pi.url.parse
//! - pi.process: pipe roundtrip, kill, process-tree cancellation,
//!   spawn_sync, exec_file_sync, dispose
//! - pi.resources: nothing survives disposal
//! - shutdown: a child left behind at VM drop is killed (no zombie)

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use pi_rs_host::{Host, HostConfig};
use std::io::{Read, Write};
use std::net::TcpListener;
use std::time::Duration;

fn host() -> Host {
    Host::new(HostConfig {
        dispatch_timeout_ms: 20_000,
        cwd: None,
        project_trusted: true,
    })
    .expect("host starts")
}

fn load_demo(host: &Host) {
    host.load(
        "mechanism-demo.lua",
        include_str!("../../../examples/extensions/mechanism-demo.lua"),
    )
    .expect("exerciser loads");
}

fn call(host: &Host, command: &str, args: &str) -> serde_json::Value {
    host.call_command(command, args)
        .expect("command runs")
        .expect("command returns a value")
}

#[test]
fn crypto_and_buffer_mechanisms() {
    let host = host();
    load_demo(&host);
    let out = call(&host, "mechanism-crypto", "");
    assert_eq!(out["sha256"], "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
    let uuid: String = out["uuid"].as_str().unwrap().to_owned();
    assert_eq!(uuid.len(), 36);
    assert_eq!(&uuid[8..9], "-");
    assert_eq!(&uuid[13..14], "-");
    assert_eq!(&uuid[18..19], "-");
    assert_eq!(&uuid[23..24], "-");
    // version nibble at index 14
    assert_eq!(&uuid[14..15], "4");
    let xx: f64 = out["xxhash32"].as_f64().unwrap();
    assert!((0.0_f64..=4_294_967_295.0_f64).contains(&xx));
}

#[test]
fn timer_mechanisms_and_dispose() {
    let host = host();
    load_demo(&host);
    let out = call(&host, "mechanism-timer", "");
    assert!(out["interval_ticks"].as_u64().unwrap() >= 2);
    assert_eq!(out["timeout_fired"], 1);
    assert_eq!(out["cancelled_fired"], 0);
}

#[test]
fn fs_mechanisms() {
    let host = host();
    load_demo(&host);
    let out = call(&host, "mechanism-fs", "");
    assert!(out["watcher_fired"].as_u64().unwrap() >= 1);
    assert_eq!(out["lstat"], "symlink");
}

#[test]
fn net_mechanisms_framed_tcp_and_url() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        stream.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        let mut line = String::new();
        let mut byte = [0u8; 1];
        loop {
            let n = stream.read(&mut byte).unwrap();
            if n == 0 {
                break;
            }
            if byte[0] == b'\n' {
                break;
            }
            line.push(byte[0] as char);
        }
        if line == "PING" {
            stream.write_all(b"PONG\n").unwrap();
        }
        // drain BYE then close
        let mut buf = [0u8; 16];
        let _ = stream.read(&mut buf).unwrap();
    });

    let host = host();
    load_demo(&host);
    let out = call(&host, "mechanism-net", &format!("{} {}", address.ip(), address.port()));
    assert_eq!(out["line"], "PONG");
    server.join().unwrap();
}

#[test]
fn process_mechanisms_pipes_kill_tree_capture() {
    let host = host();
    load_demo(&host);
    let out = call(&host, "mechanism-process", "");
    assert_eq!(out["roundtrip"], "pipe-roundtrip");
    assert_eq!(out["sync_code"], 7);
}

#[test]
fn resources_no_leak_after_disposal() {
    let host = host();
    load_demo(&host);
    let out = call(&host, "mechanism-resources", "");
    assert_eq!(out["remaining"], 0, "no resource survives dispose_all");
}

/// A child left running when the host drops must be killed by the VM
/// shutdown path (no process survives its owner).
#[test]
fn shutdown_disposes_leftover_subprocess() {
    let (pid, host) = {
        let host = host();
        load_demo(&host);
        let out = call(&host, "mechanism-shutdown", "");
        (out["pid"].as_u64().unwrap() as i32, host)
    };
    // Drop the host: the VM thread exits and disposes every resource.
    drop(host);
    std::thread::sleep(Duration::from_millis(300));
    let alive = unsafe { libc::kill(pid, 0) } == 0;
    assert!(!alive, "child {pid} survived host shutdown");
}
