#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

use std::io::{Read as _, Write as _};
use std::net::TcpListener;
use std::sync::mpsc::{Receiver, sync_channel};
use std::time::{Duration, Instant};

use pi_rs_host::{Host, HostConfig, PackageSource};

const PACKAGE: &str = r#"
local pi = ...
local held_stream

pi.register_command("long-process", { handler = function(pid_file)
  local script = "sh -c \"trap '' TERM; while :; do sleep 1; done\" & echo $! > " .. pid_file .. "; wait"
  return pi.exec("sh", {"-c", script})
end })

pi.register_command("long-timer", { handler = function()
  pi.sleep(10000)
  return {completed=true}
end })

pi.register_command("long-socket", { handler = function(url)
  return pi.http.stream(url, {timeout_ms=10000, stream_capacity=1}, function() end)
end })

pi.register_command("hold-socket", { handler = function(url)
  held_stream = pi.http.open(url, {timeout_ms=10000, stream_capacity=1})
  return {status=held_stream.status}
end })

pi.register_command("quick", { handler = function()
  pi.sleep(1)
  return {hash=pi.crypto.sha256("reload")}
end })
"#;

fn package_file() -> (tempfile::TempDir, std::path::PathBuf) {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("cleanup-effects.lua");
    std::fs::write(&path, PACKAGE).unwrap();
    (directory, path)
}

fn wait_until(mut condition: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(3);
    while !condition() {
        assert!(Instant::now() < deadline, "condition timed out");
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[cfg(unix)]
fn process_is_gone(pid: i32) -> bool {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).unwrap_or_default();
    stat.is_empty() || stat.split_whitespace().nth(2) == Some("Z")
}

fn hold_socket() -> (
    String,
    Receiver<()>,
    Receiver<()>,
    std::thread::JoinHandle<()>,
) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let (accepted_tx, accepted_rx) = sync_channel(1);
    let (closed_tx, closed_rx) = sync_channel(1);
    let join = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut request = [0_u8; 2048];
        let _ = stream.read(&mut request).unwrap();
        stream
            .write_all(
                b"HTTP/1.1 200 OK\r\ntransfer-encoding: chunked\r\nconnection: close\r\n\r\n5\r\nhello\r\n",
            )
            .unwrap();
        stream.flush().unwrap();
        accepted_tx.send(()).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        let mut byte = [0_u8; 1];
        loop {
            match stream.read(&mut byte) {
                Ok(0) | Err(_) => break,
                Ok(_) => {}
            }
        }
        closed_tx.send(()).unwrap();
    });
    (
        format!("http://{address}/hold"),
        accepted_rx,
        closed_rx,
        join,
    )
}

fn loaded_host(path: &std::path::Path) -> (Host, pi_rs_host::kernel::PackageHandle) {
    let host = Host::new(HostConfig::default()).unwrap();
    let handle = host.load_package(PackageSource::File { path }).unwrap();
    (host, handle)
}

#[cfg(unix)]
#[test]
fn scope_disposal_kills_process_tree_and_all_effect_tasks() {
    let (directory, path) = package_file();
    let pid_file = directory.path().join("descendant.pid");
    let (host, handle) = loaded_host(&path);
    let caller = host.clone();
    let pid_path = pid_file.to_string_lossy().into_owned();
    let call = std::thread::spawn(move || caller.call_command("long-process", &pid_path));
    wait_until(|| pid_file.exists());
    let pid: i32 = std::fs::read_to_string(&pid_file)
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    assert!(host.effect_stats().active > 0);

    host.dispose_package(&handle).unwrap();
    let result = call.join().unwrap().unwrap().unwrap();
    assert_eq!(result["killed"], true);
    wait_until(|| process_is_gone(pid));
    wait_until(|| host.effect_stats().active == 0);
    assert_eq!(host.scope_stats(&handle).unwrap().resources, 0);
}

#[test]
fn timer_and_loopback_socket_are_cancelled_on_disposal_then_reload() {
    let (_directory, path) = package_file();

    let (host, timer_handle) = loaded_host(&path);
    let caller = host.clone();
    let started = Instant::now();
    let timer = std::thread::spawn(move || caller.call_command("long-timer", ""));
    wait_until(|| host.effect_stats().active > 0);
    host.dispose_package(&timer_handle).unwrap();
    assert!(timer.join().unwrap().is_err());
    assert!(started.elapsed() < Duration::from_secs(2));
    wait_until(|| host.effect_stats().active == 0);

    let reloaded = host
        .load_package(PackageSource::File { path: &path })
        .unwrap();
    let result = host.call_command("quick", "").unwrap().unwrap();
    assert_eq!(
        result["hash"],
        "4027f515418b77482d033970fe4e6cc2c9b6dbd8d19201c31afd06d080a07f41"
    );
    host.dispose_package(&reloaded).unwrap();

    let (host, socket_handle) = loaded_host(&path);
    let (url, accepted, closed, server) = hold_socket();
    let caller = host.clone();
    let socket = std::thread::spawn(move || caller.call_command("long-socket", &url));
    accepted.recv_timeout(Duration::from_secs(2)).unwrap();
    wait_until(|| host.effect_stats().active > 0);
    host.dispose_package(&socket_handle).unwrap();
    let _ = socket.join().unwrap();
    closed.recv_timeout(Duration::from_secs(2)).unwrap();
    server.join().unwrap();
    wait_until(|| host.effect_stats().active == 0);
}

#[test]
fn final_host_shutdown_closes_retained_loopback_stream() {
    let (_directory, path) = package_file();
    let (url, accepted, closed, server) = hold_socket();
    let (host, _handle) = loaded_host(&path);
    let result = host.call_command("hold-socket", &url).unwrap().unwrap();
    assert_eq!(result["status"], 200);
    accepted.recv_timeout(Duration::from_secs(2)).unwrap();
    assert_eq!(host.effect_stats().active, 1);

    drop(host);
    closed.recv_timeout(Duration::from_secs(2)).unwrap();
    server.join().unwrap();
}
