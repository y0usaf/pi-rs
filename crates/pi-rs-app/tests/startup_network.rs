#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

use pi_rs_host::{Host, HostConfig};
use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::{Arc, Mutex};

#[test]
fn startup_network_policy_matches_pi_request_and_release_shapes() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let requests = Arc::new(Mutex::new(Vec::new()));
    let captured = Arc::clone(&requests);
    let server = std::thread::spawn(move || {
        for _ in 0..2 {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0_u8; 4096];
            let count = stream.read(&mut request).unwrap();
            let request = String::from_utf8_lossy(&request[..count]).to_string();
            captured.lock().unwrap().push(request.clone());
            let body = if request.starts_with("GET /latest HTTP/1.1") {
                r#"{"packageName":"@earendil-works/pi-coding-agent","version":" 0.80.0 ","note":" **Important** "}"#
            } else {
                ""
            };
            write!(
                stream,
                "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
                body.len(),
                body
            )
            .unwrap();
        }
    });

    let temp = tempfile::tempdir().unwrap();
    let host = Host::new(HostConfig {
        cwd: Some(temp.path().to_string_lossy().into_owned()),
        ..Default::default()
    })
    .unwrap();
    let report = host.load_embedded(&[
        pi_rs_agent::PACK,
        pi_rs_app::builtins::TOOLS_PACK,
        pi_rs_app::builtins::CODING_AGENT_PACK,
        pi_rs_app::builtins::INTERACTIVE_PACK,
    ]);
    assert!(report.errors.is_empty(), "{:?}", report.errors);
    let result = host
        .call_command(
            "startup-network-parity",
            &serde_json::json!({
                "version": "0.79.0",
                "userAgent": "pi/0.79.0 (linux; bun/1.2.0; x64)",
                "telemetryUrl": format!("http://{address}/report-install?version=0.79.0"),
                "versionCheckUrl": format!("http://{address}/latest"),
                "forceStartupNetwork": true,
                "telemetryEnabled": true
            })
            .to_string(),
        )
        .unwrap()
        .unwrap();
    server.join().unwrap();

    assert_eq!(result["release"]["version"], "0.80.0");
    assert_eq!(result["release"]["note"], "**Important**");
    assert_eq!(result["transcript"][0]["kind"], "update_available");
    assert_eq!(result["transcript"][0]["version"], "0.80.0");
    assert_eq!(result["comparisons"]["newer"], true);
    assert_eq!(result["comparisons"]["equal"], false);
    assert_eq!(result["comparisons"]["prerelease"], true);
    assert_eq!(result["comparisons"]["fallback"], true);

    let requests = requests.lock().unwrap();
    assert!(requests.iter().any(|request| {
        request.starts_with("GET /report-install?version=0.79.0 HTTP/1.1")
            && request.contains("user-agent: pi/0.79.0 (linux; bun/1.2.0; x64)")
    }));
    assert!(requests.iter().any(|request| {
        request.starts_with("GET /latest HTTP/1.1")
            && request.contains("accept: application/json")
            && request.contains("user-agent: pi/0.79.0 (linux; bun/1.2.0; x64)")
    }));
}
