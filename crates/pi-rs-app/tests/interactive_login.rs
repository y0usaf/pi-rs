//! End-to-end `/login` and `/logout` behavior through the product wiring
//! (PLAN 3a.2): the `interactive-login-flow` exerciser routes the slash
//! commands through `handle_submit`, drives the real selectors and
//! login dialog with key input, and completes a PKCE flow against a
//! stubbed OAuth token endpoint. Asserts the persisted `auth.json` is
//! Pi-equivalent and that `/logout` clears it.
//!
//! This file is its own test binary: it owns the process-global
//! `PI_CODING_AGENT_DIR` and the OAuth provider registry.

#![allow(clippy::unwrap_used)]

use std::io::{Read, Write};
use std::sync::Arc;

use pi_rs_ai_auth::{CallbackPages, PkceFlow, register_oauth_provider};
use pi_rs_host::{Host, HostConfig};

/// Minimal one-shot HTTP token endpoint (the spec's `postJson` target).
fn spawn_token_server() -> (std::net::SocketAddr, std::thread::JoinHandle<()>) {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let handle = std::thread::spawn(move || {
        let Ok((mut stream, _)) = listener.accept() else {
            return;
        };
        let mut buf = Vec::new();
        let mut chunk = [0u8; 4096];
        loop {
            let n = stream.read(&mut chunk).unwrap_or(0);
            if n == 0 {
                break;
            }
            buf.extend_from_slice(&chunk[..n]);
            if let Some(header_end) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                let headers = String::from_utf8_lossy(&buf[..header_end]).to_ascii_lowercase();
                let content_length = headers
                    .lines()
                    .find_map(|line| line.strip_prefix("content-length:"))
                    .and_then(|v| v.trim().parse::<usize>().ok())
                    .unwrap_or(0);
                if buf.len() >= header_end + 4 + content_length {
                    break;
                }
            }
        }
        let body =
            serde_json::json!({"access_token": "a-1", "refresh_token": "r-1", "expires_in": 3600})
                .to_string();
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(),
            body
        );
        let _ = stream.write_all(response.as_bytes());
    });
    (addr, handle)
}

fn free_port() -> u16 {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    listener.local_addr().unwrap().port()
}

fn host() -> Host {
    let host = Host::new(HostConfig::default()).unwrap();
    let report = host.load_embedded(&[
        pi_rs_app::builtins::TOOLS_PACK,
        pi_rs_app::builtins::INTERACTIVE_PACK,
    ]);
    assert!(report.errors.is_empty(), "{:?}", report.errors);
    host
}

#[test]
fn login_and_logout_complete_through_the_product_wiring() {
    let agent_dir = tempfile::tempdir().unwrap();
    // The VM's auth storage resolves PI_CODING_AGENT_DIR at Host::new.
    // SAFETY: this test binary has no other threads reading the env yet.
    unsafe { std::env::set_var("PI_CODING_AGENT_DIR", agent_dir.path()) };

    let (token_addr, token_server) = spawn_token_server();
    // Replace the built-in anthropic flow with the same row pointed at
    // the stub endpoint (spec: registerOAuthProvider replaces in place).
    register_oauth_provider(Arc::new(PkceFlow {
        id: "anthropic".into(),
        name: "Anthropic (Claude Pro/Max)".into(),
        error_label: "Anthropic".into(),
        client_id: "test-client".into(),
        authorize_url: "https://example.invalid/oauth/authorize".into(),
        token_url: format!("http://{token_addr}/oauth/token"),
        callback_port: free_port(),
        callback_path: "/callback".into(),
        scopes: "user:inference".into(),
        extra_auth_params: vec![("code".into(), "true".into())],
        instructions: "Complete login in your browser. If the browser is on another machine, paste the final redirect URL here.".into(),
        pages: CallbackPages {
            success: "ok".into(),
            denied: "no".into(),
        },
    }));

    let request = serde_json::json!({
        "model": { "id": "claude-opus-4-8", "provider": "anthropic", "api": "anthropic-messages" },
        "docsPath": "/opt/pi-rs/docs",
        "steps": [
            { "submit": "/login" },
            // "Use a subscription" -> oauth provider selector.
            { "input": ["\r"] },
            // Select Anthropic; pump the flow until the auth event mounts
            // the manual-input prompt.
            { "input": ["\r"], "pump": 5000 },
            // Paste a bare authorization code and submit; the flow
            // exchanges it at the stub endpoint and reports done.
            { "input": ["t", "e", "s", "t", "-", "c", "o", "d", "e", "\r"], "pump": 5000 },
            { "submit": "/logout" },
            { "input": ["\r"], "pump": 250 }
        ]
    });

    let result = host()
        .call_command("interactive-login-flow", &request.to_string())
        .unwrap()
        .unwrap();

    // The wiring ended back on the editor with no mounted overlay.
    assert_eq!(result["overlay"], false, "{result}");
    assert_eq!(result["editor_focused"], true);

    // Transcript: login status, the anthropic subscription warning
    // (stored oauth + anthropic model), then the logout status.
    let auth_path = agent_dir.path().join("auth.json");
    let rows = result["transcript"].as_array().unwrap();
    assert!(rows.len() >= 3, "transcript too short: {result}");
    let texts: Vec<(&str, &str)> = rows
        .iter()
        .map(|row| (row["kind"].as_str().unwrap(), row["text"].as_str().unwrap()))
        .collect();
    assert_eq!(
        texts[0],
        (
            "status",
            format!(
                "Logged in to Anthropic (Claude Pro/Max). Credentials saved to {}",
                auth_path.display()
            )
            .as_str()
        )
    );
    assert_eq!(texts[1].0, "warning");
    assert!(
        texts[1]
            .1
            .starts_with("Warning: Anthropic subscription auth is active."),
        "{:?}",
        texts[1]
    );
    assert_eq!(
        texts[2],
        ("status", "Logged out of Anthropic (Claude Pro/Max)")
    );

    // /logout removed the credential from storage and disk (an empty
    // Lua table crosses the seam as `{}`).
    assert_eq!(result["providers"], serde_json::json!({}));
    let disk: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&auth_path).unwrap()).unwrap();
    assert_eq!(disk, serde_json::json!({}));

    // The login itself persisted a Pi-shaped oauth credential before the
    // logout (the flow's respond -> exchange -> set path); replay the
    // exchange to pin the credential shape written to auth.json.
    token_server.join().unwrap();
    let (token_addr2, token_server2) = spawn_token_server();
    register_oauth_provider(Arc::new(PkceFlow {
        id: "anthropic".into(),
        name: "Anthropic (Claude Pro/Max)".into(),
        error_label: "Anthropic".into(),
        client_id: "test-client".into(),
        authorize_url: "https://example.invalid/oauth/authorize".into(),
        token_url: format!("http://{token_addr2}/oauth/token"),
        callback_port: free_port(),
        callback_path: "/callback".into(),
        scopes: "user:inference".into(),
        extra_auth_params: vec![],
        instructions: "instructions".into(),
        pages: CallbackPages {
            success: "ok".into(),
            denied: "no".into(),
        },
    }));
    let relogin = serde_json::json!({
        "model": { "id": "claude-opus-4-8", "provider": "anthropic", "api": "anthropic-messages" },
        "steps": [
            { "submit": "/login" },
            { "input": ["\r"] },
            { "input": ["\r"], "pump": 5000 },
            { "input": ["c", "-", "2", "\r"], "pump": 5000 }
        ]
    });
    let result = host()
        .call_command("interactive-login-flow", &relogin.to_string())
        .unwrap()
        .unwrap();
    assert_eq!(
        result["providers"],
        serde_json::json!(["anthropic"]),
        "{result}"
    );
    let disk: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&auth_path).unwrap()).unwrap();
    assert_eq!(disk["anthropic"]["type"], "oauth");
    assert_eq!(disk["anthropic"]["access"], "a-1");
    assert_eq!(disk["anthropic"]["refresh"], "r-1");
    assert!(disk["anthropic"]["expires"].as_i64().unwrap() > 0);
    token_server2.join().unwrap();
}
