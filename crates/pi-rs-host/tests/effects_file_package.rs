#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

use std::io::{Read as _, Write as _};
use std::net::TcpListener;
use std::time::Duration;

use pi_rs_host::{Host, HostConfig, PackageSource};

const PACKAGE: &str = r#"
local pi = ...

pi.register_command("effects-all", { handler = function(args)
  local opts = pi.json.decode(args)
  pi.fs.write_file(opts.path, "effect-data")
  pi.fs.append_file(opts.path, "!")
  local file = pi.fs.read_file(opts.path)
  local process = pi.exec("sh", {"-c", "printf process-ok"})
  local chunks = {}
  local response = pi.http.stream(opts.url, {
    timeout_ms = 1000, stream_capacity = 2, max_body_bytes = 64,
  }, function(chunk) chunks[#chunks + 1] = chunk end)
  pi.sleep(1)
  pi.clipboard.write_text("effect-clipboard", {
    platform = "other", env = { SSH_CONNECTION = "test" },
  })
  return {
    file = file,
    process = process.stdout,
    status = response.status,
    body = table.concat(chunks),
    hash = pi.crypto.sha256("abc"),
    random_len = #pi.crypto.random_bytes(24),
    uuid = pi.random_uuid(),
  }
end })

pi.register_command("effects-cancel", { handler = function()
  local timer_signal = pi.abort_signal()
  pi.spawn(function() pi.sleep(5); timer_signal:abort() end)
  local timer_ok, timer_error = pcall(pi.sleep, 10000, timer_signal)

  local process_signal = pi.abort_signal()
  pi.spawn(function() pi.sleep(5); process_signal:abort() end)
  local process = pi.exec("sh", {"-c", "printf partial; sleep 10"}, {signal=process_signal})
  return {
    timer_cancelled = not timer_ok and tostring(timer_error):find("aborted") ~= nil,
    process_killed = process.killed,
    process_output = process.stdout,
  }
end })

pi.register_command("effects-bounds", { handler = function(args)
  local output_ok, output_error = pcall(pi.exec, "sh", {"-c", "head -c 4096 /dev/zero"}, {
    max_output_bytes = 64,
  })
  local timeout_ok, timeout_error = pcall(pi.http.get, args, {timeout_ms=20})
  return {
    output_bounded = not output_ok and tostring(output_error):find("exceeded 64") ~= nil,
    timed_out = not timeout_ok and tostring(timeout_error):find("timed out") ~= nil,
  }
end })
"#;

fn serve_once(body: &'static [u8]) -> (String, std::thread::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let join = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut request = [0_u8; 2048];
        let _ = stream.read(&mut request).unwrap();
        write!(
            stream,
            "HTTP/1.1 200 OK\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
            body.len()
        )
        .unwrap();
        for chunk in body.chunks(3) {
            stream.write_all(chunk).unwrap();
            stream.flush().unwrap();
            std::thread::sleep(Duration::from_millis(2));
        }
    });
    (format!("http://{address}/effects"), join)
}

fn file_package() -> (tempfile::TempDir, std::path::PathBuf) {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("effects-package.lua");
    std::fs::write(&path, PACKAGE).unwrap();
    (directory, path)
}

#[test]
fn ordinary_file_package_exercises_every_effect_and_bounds() {
    let (directory, package) = file_package();
    let output = directory.path().join("effect.txt");
    let (url, server) = serve_once(b"hello-stream");
    let host = Host::new(HostConfig::default()).unwrap();
    let handle = host
        .load_package(PackageSource::File { path: &package })
        .unwrap();

    let result = host
        .call_command(
            "effects-all",
            &serde_json::json!({"path": output, "url": url}).to_string(),
        )
        .unwrap()
        .unwrap();
    assert_eq!(result["file"], "effect-data!");
    assert_eq!(result["process"], "process-ok");
    assert_eq!(result["status"], 200);
    assert_eq!(result["body"], "hello-stream");
    assert_eq!(
        result["hash"],
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
    assert_eq!(result["random_len"], 24);
    let uuid = result["uuid"].as_str().unwrap();
    assert_eq!(uuid.len(), 36);
    server.join().unwrap();

    let cancelled = host.call_command("effects-cancel", "").unwrap().unwrap();
    assert_eq!(cancelled["timer_cancelled"], true);
    assert_eq!(cancelled["process_killed"], true);
    assert_eq!(cancelled["process_output"], "partial");

    let stalled = TcpListener::bind("127.0.0.1:0").unwrap();
    let stalled_address = stalled.local_addr().unwrap();
    let stalled_server = std::thread::spawn(move || {
        let (mut stream, _) = stalled.accept().unwrap();
        let mut request = [0_u8; 1024];
        let _ = stream.read(&mut request);
        std::thread::sleep(Duration::from_millis(100));
    });
    let bounded = host
        .call_command("effects-bounds", &format!("http://{stalled_address}/stall"))
        .unwrap()
        .unwrap();
    assert_eq!(bounded["output_bounded"], true);
    assert_eq!(bounded["timed_out"], true);
    stalled_server.join().unwrap();

    assert_eq!(host.effect_stats().active, 0);
    assert_eq!(host.scope_stats(&handle).unwrap().resources, 0);
    host.dispose_package(&handle).unwrap();
}
