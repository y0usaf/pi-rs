//! Parity tests for the callback server (spec: `startCallbackServer` in
//! `utils/oauth/anthropic.ts`) over real loopback connections.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use pi_rs_ai_auth::{CallbackPages, CallbackServer};

fn pages() -> CallbackPages {
    CallbackPages {
        success: "Test authentication completed. You can close this window.".into(),
        denied: "Test authentication did not complete.".into(),
    }
}

/// Reserve a loopback port (bind :0, read, release).
fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

async fn get(port: u16, path_query: &str) -> (u16, String) {
    let response = reqwest::get(format!("http://127.0.0.1:{port}{path_query}"))
        .await
        .unwrap();
    let status = response.status().as_u16();
    let body = response.text().await.unwrap();
    (status, body)
}

#[tokio::test]
async fn serves_until_matching_code_arrives() {
    let port = free_port();
    let mut server = CallbackServer::start(port, "/callback", "expected".into(), pages())
        .await
        .unwrap();

    // Wrong path → 404 error page, server keeps serving.
    let (status, body) = get(port, "/nope").await;
    assert_eq!(status, 404);
    assert!(body.contains("Callback route not found."));

    // Provider error → 400 with the flow's denied message + details.
    let (status, body) = get(port, "/callback?error=access_denied").await;
    assert_eq!(status, 400);
    assert!(body.contains("Test authentication did not complete."));
    assert!(body.contains("Error: access_denied"));

    // Missing params → 400.
    let (status, body) = get(port, "/callback?code=abc").await;
    assert_eq!(status, 400);
    assert!(body.contains("Missing code or state parameter."));

    // State mismatch → 400, still not settled.
    let (status, body) = get(port, "/callback?code=abc&state=wrong").await;
    assert_eq!(status, 400);
    assert!(body.contains("State mismatch."));

    // Matching code + state → 200 success page, wait settles.
    let (status, body) = get(port, "/callback?code=the-code&state=expected").await;
    assert_eq!(status, 200);
    assert!(body.contains("Test authentication completed. You can close this window."));

    let settled = server.wait_for_code().await.unwrap();
    assert_eq!(settled.code, "the-code");
    assert_eq!(settled.state, "expected");
}

#[tokio::test]
async fn query_values_are_percent_decoded() {
    let port = free_port();
    let mut server = CallbackServer::start(port, "/callback", "st ate".into(), pages())
        .await
        .unwrap();
    let (status, _) = get(port, "/callback?code=a%2Bb&state=st%20ate").await;
    assert_eq!(status, 200);
    let settled = server.wait_for_code().await.unwrap();
    assert_eq!(settled.code, "a+b");
    assert_eq!(settled.state, "st ate");
}
