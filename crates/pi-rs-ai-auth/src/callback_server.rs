//! Loopback callback server for authorization-code redirects.
//!
//! Spec: `startCallbackServer` in `utils/oauth/anthropic.ts` (Node
//! `http.createServer`); extracted here as shared PKCE-engine machinery
//! (locked `pi-rs-ai` row: one PKCE engine, flows as data). Response
//! behavior is 1:1: wrong path → 404 error page and keep serving;
//! provider `error` / missing params / state mismatch → 400 error page
//! and keep serving; matching `code`+`state` → 200 success page and
//! settle. The spec's `cancelWait` has no analog — callers race the
//! wait future with `tokio::select!`, which drops it.
//!
//! Resurrected from the attic (`rebuild` @ `e8cb418`,
//! `pi-rs-ai-auth/src/auth/callback_server.rs`) and reshaped to the
//! spec's pages and per-request semantics.

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use crate::error::AuthError;
use crate::oauth_page::{oauth_error_html, oauth_success_html};

/// Spec: the `PI_OAUTH_CALLBACK_HOST` override (default loopback).
const CALLBACK_HOST_ENV: &str = "PI_OAUTH_CALLBACK_HOST";

/// The settled `{ code, state }` pair.
pub struct CallbackCode {
    pub code: String,
    pub state: String,
}

/// Messages a flow customizes on the served pages (flow data).
#[derive(Clone)]
pub struct CallbackPages {
    /// Success-page body, e.g. "… authentication completed. You can
    /// close this window."
    pub success: String,
    /// Error-page body when the provider redirects with `error=…`,
    /// e.g. "… authentication did not complete."
    pub denied: String,
}

pub struct CallbackServer {
    rx: mpsc::Receiver<CallbackCode>,
    handle: JoinHandle<()>,
}

impl CallbackServer {
    /// Bind `PI_OAUTH_CALLBACK_HOST` (default `127.0.0.1`) on `port` and
    /// serve until a request to `path` carries a `code` with the
    /// expected `state`.
    pub async fn start(
        port: u16,
        path: &str,
        expected_state: String,
        pages: CallbackPages,
    ) -> Result<Self, AuthError> {
        let host = std::env::var(CALLBACK_HOST_ENV).unwrap_or_else(|_| "127.0.0.1".into());
        let listener = TcpListener::bind((host.as_str(), port)).await?;
        let (tx, rx) = mpsc::channel(1);
        let path = path.to_owned();
        let handle = tokio::spawn(async move {
            loop {
                let Ok((mut stream, _)) = listener.accept().await else {
                    return;
                };
                if let Some(settled) =
                    handle_request(&mut stream, &path, &expected_state, &pages).await
                {
                    let _ = tx.send(settled).await;
                    return;
                }
            }
        });
        Ok(Self { rx, handle })
    }

    /// Wait for the authorization code; `None` if the server died.
    /// Spec: `waitForCode()`.
    pub async fn wait_for_code(&mut self) -> Option<CallbackCode> {
        self.rx.recv().await
    }
}

impl Drop for CallbackServer {
    // Spec: `server.close()` in the login flow's `finally`.
    fn drop(&mut self) {
        self.handle.abort();
    }
}

/// Serve one request. Returns the settled code when login completed.
async fn handle_request(
    stream: &mut TcpStream,
    path: &str,
    expected_state: &str,
    pages: &CallbackPages,
) -> Option<CallbackCode> {
    let target = read_request_target(stream).await?;
    let url = url::Url::parse(&format!("http://localhost{target}")).ok()?;

    if url.path() != path {
        respond_html(
            stream,
            404,
            &oauth_error_html("Callback route not found.", None),
        )
        .await;
        return None;
    }

    let mut code = None;
    let mut state = None;
    let mut error = None;
    for (key, value) in url.query_pairs() {
        match key.as_ref() {
            "code" => code = Some(value.into_owned()),
            "state" => state = Some(value.into_owned()),
            "error" => error = Some(value.into_owned()),
            _ => {}
        }
    }

    if let Some(error) = error {
        let details = format!("Error: {error}");
        respond_html(
            stream,
            400,
            &oauth_error_html(&pages.denied, Some(&details)),
        )
        .await;
        return None;
    }

    let (Some(code), Some(state)) = (code, state) else {
        respond_html(
            stream,
            400,
            &oauth_error_html("Missing code or state parameter.", None),
        )
        .await;
        return None;
    };

    if state != expected_state {
        respond_html(stream, 400, &oauth_error_html("State mismatch.", None)).await;
        return None;
    }

    respond_html(stream, 200, &oauth_success_html(&pages.success)).await;
    Some(CallbackCode { code, state })
}

/// Read request headers and return the request-target of the first line.
async fn read_request_target(stream: &mut TcpStream) -> Option<String> {
    let mut buf = vec![0u8; 16 * 1024];
    let mut len = 0;
    while len < buf.len() {
        match stream.read(&mut buf[len..]).await {
            Ok(0) | Err(_) => break,
            Ok(n) => {
                len += n;
                if buf[..len].windows(4).any(|w| w == b"\r\n\r\n") {
                    break;
                }
            }
        }
    }
    let request = String::from_utf8_lossy(&buf[..len]).into_owned();
    request
        .lines()
        .next()?
        .split_whitespace()
        .nth(1)
        .map(str::to_owned)
}

async fn respond_html(stream: &mut TcpStream, status: u16, body: &str) {
    let reason = match status {
        200 => "OK",
        400 => "Bad Request",
        404 => "Not Found",
        _ => "Internal Server Error",
    };
    let response = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let _ = stream.write_all(response.as_bytes()).await;
    let _ = stream.shutdown().await;
}
