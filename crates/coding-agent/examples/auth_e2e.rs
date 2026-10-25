//! `pi-rs auth ...` on the built binary: the credential it prints, the status
//! it reports, the errors it refuses with, and the refresh it writes back, against a fake token
//! server on a real socket.

use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::thread;

const ACCESS: &str = "anthropic-access";
const REFRESHED: &str = "refreshed-access";

fn binary() -> PathBuf {
    match std::env::var("PI_RS_BIN") {
        Ok(value) if !value.is_empty() => PathBuf::from(value),
        _ => panic!("set PI_RS_BIN to the built pi-rs binary"),
    }
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64
}

fn write_auth(agent_dir: &str, value: &str) {
    std::fs::create_dir_all(agent_dir).unwrap();
    std::fs::write(Path::new(agent_dir).join("auth.json"), value).unwrap();
}

fn read_auth(agent_dir: &str) -> String {
    std::fs::read_to_string(Path::new(agent_dir).join("auth.json")).unwrap_or_default()
}

/// The token endpoint every refresh in this check goes to.
fn fake_token_server() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            let mut buffer = [0u8; 4096];
            let _ = stream.read(&mut buffer);
            let body = format!(
                "{{\"access_token\":\"{REFRESHED}\",\"refresh_token\":\"refreshed-refresh\",\"expires_in\":3600}}"
            );
            let reply = format!(
                "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = stream.write_all(reply.as_bytes());
        }
    });
    format!("http://127.0.0.1:{port}/token")
}

struct Output {
    stdout: String,
    stderr: String,
    code: i32,
}

fn run(bin: &Path, home: &str, agent_dir: &str, token_url: &str, args: &[&str]) -> Output {
    let output = Command::new(bin)
        .args(args)
        .env_clear()
        .env("HOME", home)
        .env("PI_CODING_AGENT_DIR", agent_dir)
        .env("PI_OAUTH_ANTHROPIC_TOKEN_URL", token_url)
        .env("PI_OAUTH_ANTHROPIC_AUTHORIZE_URL", token_url)
        .stdin(std::process::Stdio::null())
        .output()
        .expect("pi-rs runs");
    Output {
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        code: output.status.code().unwrap_or(-1),
    }
}

fn check<T: PartialEq + std::fmt::Debug>(label: &str, got: T, want: T) {
    assert!(got == want, "{label}: got {got:?}, want {want:?}");
}

fn contains(label: &str, haystack: &str, needle: &str) {
    assert!(
        haystack.contains(needle),
        "{label}: {needle:?} not in {haystack:?}"
    );
}

fn main() {
    let bin = binary();
    let root = std::env::temp_dir().join(format!("pi-auth-e2e-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let home = root.join("home").to_string_lossy().into_owned();
    let agent_dir = root.join("home/agent").to_string_lossy().into_owned();
    std::fs::create_dir_all(&agent_dir).unwrap();
    let token_url = fake_token_server();

    let oauth = format!(
        "{{\"anthropic\":{{\"type\":\"oauth\",\"access\":\"{ACCESS}\",\"refresh\":\"r1\",\"expires\":{}}}}}",
        now_ms() + 3_600_000
    );
    write_auth(&agent_dir, &oauth);

    let out = run(&bin, &home, &agent_dir, &token_url, &["auth"]);
    check("help exit", out.code, 0);
    contains("help usage", &out.stdout, "Usage:");
    contains("help bearer", &out.stdout, "pi-rs auth print-bearer-token");

    let out = run(&bin, &home, &agent_dir, &token_url, &["auth", "--help"]);
    check("help flag exit", out.code, 0);

    let out = run(&bin, &home, &agent_dir, &token_url, &["auth", "nope"]);
    check("unknown exit", out.code, 1);
    contains(
        "unknown message",
        &out.stderr,
        "Unknown auth command \"nope\"",
    );

    let out = run(
        &bin,
        &home,
        &agent_dir,
        &token_url,
        &["auth", "check", "--provider", "anthropic"],
    );
    check("check exit", out.code, 0);
    check("check status", out.stdout.trim(), "ready");

    let out = run(
        &bin,
        &home,
        &agent_dir,
        &token_url,
        &["auth", "check", "--provider", "anthropic", "--json"],
    );
    check("check json exit", out.code, 0);
    let json: serde_json::Value = serde_json::from_str(out.stdout.trim()).expect("json output");
    check("json status", json["status"].as_str(), Some("ready"));
    check(
        "json provider",
        json["provider"].as_str(),
        Some("anthropic"),
    );
    check("json authType", json["authType"].as_str(), Some("oauth"));
    check(
        "json has no credentials",
        json.get("credentials").is_none(),
        true,
    );

    let out = run(
        &bin,
        &home,
        &agent_dir,
        &token_url,
        &[
            "auth",
            "check",
            "--provider",
            "anthropic",
            "--json",
            "--credentials",
        ],
    );
    check("check credentials exit", out.code, 0);
    let json: serde_json::Value = serde_json::from_str(out.stdout.trim()).expect("json output");
    check(
        "json credential",
        json["credentials"].as_str(),
        Some(ACCESS),
    );

    let out = run(
        &bin,
        &home,
        &agent_dir,
        &token_url,
        &["auth", "check", "--provider", "nope"],
    );
    check("unknown provider check exit", out.code, 1);
    check("unknown provider status", out.stdout.trim(), "not_ready");

    let out = run(
        &bin,
        &home,
        &agent_dir,
        &token_url,
        &["auth", "print-bearer-token", "--provider", "anthropic"],
    );
    check("bearer exit", out.code, 0);
    check("bearer token", out.stdout.trim(), ACCESS);

    let out = run(
        &bin,
        &home,
        &agent_dir,
        &token_url,
        &["auth", "print-api-key", "--provider", "anthropic"],
    );
    check("api key on oauth exit", out.code, 1);
    contains(
        "api key on oauth message",
        &out.stderr,
        "Provider \"anthropic\" is configured with OAuth, not an API key",
    );

    write_auth(
        &agent_dir,
        "{\"openrouter\":{\"type\":\"api_key\",\"key\":\"sk-or-1\"}}",
    );
    let out = run(
        &bin,
        &home,
        &agent_dir,
        &token_url,
        &["auth", "print-api-key", "--provider", "openrouter"],
    );
    check("api key exit", out.code, 0);
    check("api key", out.stdout.trim(), "sk-or-1");

    let out = run(
        &bin,
        &home,
        &agent_dir,
        &token_url,
        &["auth", "print-bearer-token", "--provider", "openrouter"],
    );
    check("bearer on api key exit", out.code, 1);
    contains(
        "bearer on api key message",
        &out.stderr,
        "Provider \"openrouter\" is not configured with an OAuth bearer token",
    );

    let out = run(&bin, &home, &agent_dir, &token_url, &["auth", "check"]);
    check("check without provider exit", out.code, 1);
    contains(
        "check without provider message",
        &out.stderr,
        "Auth checks require --provider <provider> or --model <model>",
    );

    let out = run(
        &bin,
        &home,
        &agent_dir,
        &token_url,
        &["auth", "check", "--provider", "anthropic", "--bogus"],
    );
    check("unknown flag exit", out.code, 1);
    contains(
        "unknown flag message",
        &out.stderr,
        "Unknown option --bogus for \"auth check\"",
    );

    let expired = format!(
        "{{\"anthropic\":{{\"type\":\"oauth\",\"access\":\"stale\",\"refresh\":\"r1\",\"expires\":{}}}}}",
        now_ms() - 60_000
    );
    write_auth(&agent_dir, &expired);
    let out = run(
        &bin,
        &home,
        &agent_dir,
        &token_url,
        &["auth", "print-bearer-token", "--provider", "anthropic"],
    );
    check("refresh exit", out.code, 0);
    check("refreshed token", out.stdout.trim(), REFRESHED);
    contains("refresh written back", &read_auth(&agent_dir), REFRESHED);

    let soon = format!(
        "{{\"anthropic\":{{\"type\":\"oauth\",\"access\":\"{ACCESS}\",\"refresh\":\"r1\",\"expires\":{}}}}}",
        now_ms() + 10 * 60_000
    );
    write_auth(&agent_dir, &soon);
    let out = run(
        &bin,
        &home,
        &agent_dir,
        &token_url,
        &[
            "auth",
            "print-bearer-token",
            "--provider",
            "anthropic",
            "--min-expiry",
            "5m",
        ],
    );
    check("min expiry exit", out.code, 0);
    check("min expiry keeps token", out.stdout.trim(), ACCESS);

    let out = run(
        &bin,
        &home,
        &agent_dir,
        &token_url,
        &[
            "auth",
            "print-bearer-token",
            "--provider",
            "anthropic",
            "--min-expiry",
            "1h",
        ],
    );
    check("default min expiry refreshes", out.stdout.trim(), REFRESHED);

    let out = run(
        &bin,
        &home,
        &agent_dir,
        &token_url,
        &[
            "auth",
            "print-bearer-token",
            "--provider",
            "anthropic",
            "--min-expiry",
            "soon",
        ],
    );
    check("bad duration exit", out.code, 1);

    let _ = std::fs::remove_dir_all(&root);
    println!(
        "auth ok: check, json, credentials, print-api-key, print-bearer-token, refresh, errors"
    );
}
