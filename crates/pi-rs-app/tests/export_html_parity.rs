#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

//! Pi-derived differential for `core/export-html/index.ts`. The checked oracle
//! records the exact JSON text embedded in the HTML (including object order and
//! explicit nulls) and the SHA-256 of the complete document.

use pi_rs_app::builtins::{INTERACTIVE_PACK, TOOLS_PACK};
use pi_rs_host::{Host, HostConfig};
use sha2::{Digest, Sha256};

fn decode_base64(input: &str) -> Vec<u8> {
    let mut output = Vec::new();
    let mut chunk = [0u8; 4];
    let mut count = 0;
    for byte in input.bytes().filter(|byte| *byte != b'=') {
        chunk[count] = match byte {
            b'A'..=b'Z' => byte - b'A',
            b'a'..=b'z' => byte - b'a' + 26,
            b'0'..=b'9' => byte - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            _ => continue,
        };
        count += 1;
        if count == 4 {
            output.push(chunk[0] << 2 | chunk[1] >> 4);
            output.push(chunk[1] << 4 | chunk[2] >> 2);
            output.push(chunk[2] << 6 | chunk[3]);
            count = 0;
        }
    }
    if count >= 2 {
        output.push(chunk[0] << 2 | chunk[1] >> 4);
    }
    if count >= 3 {
        output.push(chunk[1] << 4 | chunk[2] >> 2);
    }
    output
}

#[test]
fn html_document_and_embedded_payload_match_pi() {
    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("../../../tests/export-html-parity/case.json")).unwrap();
    let oracle: serde_json::Value = serde_json::from_str(include_str!(
        "../../../tests/export-html-parity/oracle.json"
    ))
    .unwrap();
    let temp = tempfile::tempdir().unwrap();
    let session_file = temp.path().join("session.jsonl");
    let output_path = temp.path().join("session.html");
    std::fs::write(&session_file, fixture["session"].as_str().unwrap()).unwrap();

    let host = Host::new(HostConfig {
        cwd: Some(temp.path().to_string_lossy().into_owned()),
        ..HostConfig::default()
    })
    .unwrap();
    let report = host.load_embedded(&[pi_rs_agent::PACK, TOOLS_PACK, INTERACTIVE_PACK]);
    assert!(report.errors.is_empty(), "{:?}", report.errors);
    host.call_command(
        "export-html-parity",
        &serde_json::json!({
            "sessionFile": session_file,
            "outputPath": output_path,
            "systemPrompt": fixture["systemPrompt"],
            "tools": fixture["tools"],
            "theme": "dark",
            "colorMode": "truecolor",
            "appName": "pi"
        })
        .to_string(),
    )
    .expect("command")
    .expect("result");

    let html = std::fs::read_to_string(output_path).unwrap();
    if let Some(path) = std::env::var_os("EXPORT_HTML_DEBUG") {
        std::fs::write(path, &html).unwrap();
    }
    let encoded = html
        .split("<script id=\"session-data\" type=\"application/json\">")
        .nth(1)
        .and_then(|tail| tail.split("</script>").next())
        .expect("session payload");
    let payload = String::from_utf8(decode_base64(encoded)).unwrap();
    assert_eq!(payload, oracle["payload"].as_str().unwrap());
    assert!(payload.contains(r#""parentId":null"#));

    let digest = format!("{:x}", Sha256::digest(html.as_bytes()));
    assert_eq!(digest, oracle["htmlSha256"].as_str().unwrap());
}
