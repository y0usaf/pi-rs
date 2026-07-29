//! Subscription login driven by an ordinary file-backed package.
//!
//! Rust contributes the OAuth wire flow only: PKCE, the loopback callback
//! server, authorization-code exchange, and RFC 8628 device polling. Every
//! user-visible step — the login-method selector, the authorization URL, the
//! device code, prompts, progress, and manual code entry — is an ordinary Lua
//! function, and the credential row comes back for Lua to store through the
//! same `pi.auth.v1.store` it already owns.
//!
//! The provider rows are fixture flows pointed at a local HTTP socket, so the
//! whole journey runs offline. They are wire configuration, not product policy:
//! the package names only the provider id it logs into.

#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

use std::io::{Read, Write};
use std::sync::{Arc, Mutex};

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use pi_rs_ai_auth::{
    GitHubCopilotEndpoints, GitHubCopilotFlow, OpenAiCodexEndpoints, OpenAiCodexFlow,
};
use pi_rs_host::kernel::{DispatchBatch, DispatchRequest, RootKind};
use pi_rs_host::{Host, HostConfig, PackageSource};

/// A browser login: the package picks the method, shows the URL, pastes the
/// code back, then stores the returned row itself.
const BROWSER_LOGIN_PACKAGE: &str = r#"
local pi = ...
local auth = pi.auth.v1
local effects = pi.effects.v1
local roots = pi.roots.v1

local canonical = effects.path.join(effects.env.get("PI_TEST_HOME"), "credentials.json")

roots.register({
  kind = "application",
  id = "browser-login",
  dispatch = function()
    local seen = { auth = {}, device_codes = {}, progress = {}, options = {} }

    local credential = auth.login("openai-codex", {
      on_auth = function(info)
        seen.auth[#seen.auth + 1] = info.url
        seen.instructions = info.instructions
      end,
      on_device_code = function(info)
        seen.device_codes[#seen.device_codes + 1] = info.user_code
      end,
      on_prompt = function(prompt)
        seen.prompt = prompt.message
        return ""
      end,
      on_select = function(prompt)
        seen.select = prompt.message
        for _, option in ipairs(prompt.options) do
          seen.options[#seen.options + 1] = option.id .. "=" .. option.label
        end
        return "browser"
      end,
      on_progress = function(message)
        seen.progress[#seen.progress + 1] = message
      end,
      on_manual_code_input = function()
        return "browser-code"
      end,
    })

    -- Login returns a row; where it lives is still the package's decision.
    local store = auth.store({ canonical = canonical })
    store:set_oauth("openai-codex", credential)
    local described = store:describe("openai-codex")
    local resolved = store:resolve("openai-codex")

    roots.action("logged-in", {
      refresh = credential.refresh,
      access = credential.access,
      account = credential.accountId,
      expires_is_number = type(credential.expires) == "number",
      select_message = seen.select,
      select_options = seen.options,
      auth_url = seen.auth[1],
      auth_calls = #seen.auth,
      instructions = seen.instructions,
      device_code_calls = #seen.device_codes,
      prompt = seen.prompt,
      stored_providers = store:snapshot().providers,
      described_kind = described.kind,
      described_extra = described.extra_fields,
      resolved_key = resolved.api_key,
      resolved_refreshed = resolved.refreshed,
      default_login_timeout_ms = auth.default_login_timeout_ms,
      max_login_timeout_ms = auth.max_login_timeout_ms,
      max_login_models = auth.max_login_models,
    })
  end,
})
"#;

/// A headless device-code login: the package answers the enterprise prompt,
/// renders the code, and supplies the catalog rows to enable afterwards.
const DEVICE_LOGIN_PACKAGE: &str = r#"
local pi = ...
local auth = pi.auth.v1
local roots = pi.roots.v1

roots.register({
  kind = "application",
  id = "device-login",
  dispatch = function()
    local seen = { device_codes = {}, progress = {}, model_id_calls = {} }

    local credential = auth.login("github-copilot", {
      on_auth = function() error("browser login is not part of this journey") end,
      on_device_code = function(info)
        seen.device_codes[#seen.device_codes + 1] = info
      end,
      on_prompt = function(prompt)
        seen.prompt = prompt.message
        seen.placeholder = prompt.placeholder
        seen.allow_empty = prompt.allow_empty
        return ""
      end,
      on_select = function() return nil end,
      on_progress = function(message)
        seen.progress[#seen.progress + 1] = message
      end,
    }, {
      timeout_ms = 60000,
      model_ids = function(provider)
        seen.model_id_calls[#seen.model_id_calls + 1] = provider
        return { "gpt-test" }
      end,
    })

    local device = seen.device_codes[1]
    roots.action("logged-in", {
      refresh = credential.refresh,
      access = credential.access,
      enterprise = credential.enterpriseUrl,
      prompt = seen.prompt,
      placeholder = seen.placeholder,
      allow_empty = seen.allow_empty,
      user_code = device.user_code,
      verification_uri = device.verification_uri,
      interval_seconds = device.interval_seconds,
      expires_in_seconds = device.expires_in_seconds,
      progress = seen.progress,
      model_id_calls = seen.model_id_calls,
    })
  end,
})
"#;

/// Every refusal a login package meets before any network traffic happens.
const REFUSING_PACKAGE: &str = r#"
local pi = ...
local auth = pi.auth.v1
local roots = pi.roots.v1

local function refusal(fn)
  local ok, error_value = pcall(fn)
  return { ok = ok, message = tostring(error_value) }
end

local function callbacks(without)
  local base = {
    on_auth = function() end,
    on_device_code = function() end,
    on_prompt = function() return "" end,
    on_select = function() return nil end,
  }
  if without then base[without] = nil end
  return base
end

roots.register({
  kind = "application",
  id = "login-refusals",
  dispatch = function()
    roots.action("refused", {
      unknown_provider = refusal(function()
        return auth.login("not-a-subscription", callbacks())
      end),
      missing_callback = refusal(function()
        return auth.login("anthropic", callbacks("on_select"))
      end),
      zero_timeout = refusal(function()
        return auth.login("anthropic", callbacks(), { timeout_ms = 0 })
      end),
      oversize_timeout = refusal(function()
        return auth.login("anthropic", callbacks(), { timeout_ms = 7200000 })
      end),
      oversize_model_ids = refusal(function()
        return auth.login("anthropic", callbacks(), {
          model_ids = function()
            local ids = {}
            for index = 1, auth.max_login_models + 1 do ids[index] = "model-" .. index end
            return ids
          end,
        })
      end),
    })
  end,
})
"#;

/// A login that never completes on its own: the package pastes a code and the
/// token endpoint hangs, so only cancellation can end the dispatch.
const HANGING_LOGIN_PACKAGE: &str = r#"
local pi = ...
local auth = pi.auth.v1
local roots = pi.roots.v1

roots.register({
  kind = "application",
  id = "hanging-login",
  dispatch = function()
    local credential = auth.login("anthropic", {
      on_auth = function() end,
      on_device_code = function() end,
      on_prompt = function() return "pasted-code" end,
      on_select = function() return nil end,
      on_manual_code_input = function() return "pasted-code" end,
    })
    roots.action("logged-in", { refresh = credential.refresh })
  end,
})
"#;

/// Route body marking an endpoint that accepts the request and never answers.
const HANGING_RESPONSE: &str = "@hang";

/// Canned JSON served by an ordinary local HTTP socket, routed by request
/// target so concurrently issued requests cannot depend on arrival order.
fn fixture_endpoints(routes: Vec<(String, String)>) -> (String, Arc<Mutex<Vec<String>>>) {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind fixture endpoints");
    let port = listener.local_addr().expect("fixture address").port();
    let seen = Arc::new(Mutex::new(Vec::new()));
    let recorded = Arc::clone(&seen);
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut socket) = stream else { return };
            let routes = routes.clone();
            let recorded = Arc::clone(&recorded);
            std::thread::spawn(move || {
                let Some((target, _body)) = read_request(&mut socket) else {
                    return;
                };
                recorded.lock().expect("record target").push(target.clone());
                let response = routes
                    .iter()
                    .find(|(route, _)| *route == target)
                    .map(|(_, body)| (200, body.clone()))
                    .unwrap_or((404, "{}".to_owned()));
                if response.1 == HANGING_RESPONSE {
                    // Hold the connection open: the client can only leave by
                    // being cancelled.
                    std::thread::park();
                    return;
                }
                let payload = format!(
                    "HTTP/1.1 {} OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
                    response.0,
                    response.1.len(),
                    response.1,
                );
                let _ = socket.write_all(payload.as_bytes());
            });
        }
    });
    (format!("http://127.0.0.1:{port}"), seen)
}

/// Read one request, honouring `content-length` so a POST body is complete
/// before the canned response goes out.
fn read_request(socket: &mut std::net::TcpStream) -> Option<(String, String)> {
    let mut request = Vec::new();
    let mut buffer = [0_u8; 2048];
    loop {
        let read = socket.read(&mut buffer).ok()?;
        if read == 0 {
            return None;
        }
        request.extend_from_slice(&buffer[..read]);
        let Some(split) = request.windows(4).position(|window| window == b"\r\n\r\n") else {
            if request.len() > 64 * 1024 {
                return None;
            }
            continue;
        };
        let headers = String::from_utf8_lossy(&request[..split]).into_owned();
        let length = headers
            .lines()
            .find_map(|line| {
                line.to_ascii_lowercase()
                    .strip_prefix("content-length:")
                    .map(str::to_owned)
            })
            .and_then(|value| value.trim().parse::<usize>().ok())
            .unwrap_or(0);
        while request.len() < split + 4 + length {
            let read = socket.read(&mut buffer).ok()?;
            if read == 0 {
                break;
            }
            request.extend_from_slice(&buffer[..read]);
        }
        let target = headers
            .lines()
            .next()
            .and_then(|line| line.split_whitespace().nth(1))
            .map(str::to_owned)?;
        let body = String::from_utf8_lossy(&request[split + 4..]).into_owned();
        return Some((target, body));
    }
}

/// The Codex access token is a JWT whose payload carries the account id.
fn jwt(account_id: &str) -> String {
    let payload = serde_json::json!({
        "https://api.openai.com/auth": { "chatgpt_account_id": account_id }
    });
    format!("x.{}.y", URL_SAFE_NO_PAD.encode(payload.to_string()))
}

fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .expect("free port")
        .local_addr()
        .expect("free port address")
        .port()
}

fn host_with(home: &std::path::Path) -> Host {
    Host::new(HostConfig {
        environment: Some(
            [(
                "PI_TEST_HOME".to_owned(),
                home.to_string_lossy().into_owned(),
            )]
            .into_iter()
            .collect(),
        ),
        ..HostConfig::default()
    })
    .expect("host starts")
}

fn load(host: &Host, directory: &std::path::Path, name: &str, source: &str) {
    let path = directory.join(format!("{name}.lua"));
    std::fs::write(&path, source).expect("write file-backed package");
    host.load_package(PackageSource::File { path: &path })
        .expect("file-backed package loads");
}

fn dispatch(host: &Host) -> DispatchBatch {
    host.dispatch(DispatchRequest::new(
        RootKind::Application,
        serde_json::json!({ "kind": "startup" }),
        serde_json::json!({}),
    ))
    .expect("application dispatch")
}

#[test]
fn file_backed_package_drives_a_browser_login_and_stores_the_row() {
    let access = jwt("acct-browser");
    let (base, targets) = fixture_endpoints(vec![(
        "/oauth/token".to_owned(),
        serde_json::json!({
            "access_token": access,
            "refresh_token": "refresh-browser",
            "expires_in": 3600,
        })
        .to_string(),
    )]);
    pi_rs_ai_auth::register_oauth_provider(Arc::new(OpenAiCodexFlow {
        endpoints: OpenAiCodexEndpoints {
            authorize_url: format!("{base}/oauth/authorize"),
            token_url: format!("{base}/oauth/token"),
            device_user_code_url: format!("{base}/device/usercode"),
            device_token_url: format!("{base}/device/token"),
            device_verification_uri: format!("{base}/codex/device"),
            device_redirect_uri: format!("{base}/device/callback"),
            callback_port: free_port(),
        },
    }));

    let directory = tempfile::tempdir().expect("temporary directory");
    let host = host_with(directory.path());
    load(
        &host,
        directory.path(),
        "browser-login",
        BROWSER_LOGIN_PACKAGE,
    );
    let batch = dispatch(&host);
    let payload = &batch.actions[0].payload;

    // The login-method selector is a Lua decision over flow-supplied options.
    assert_eq!(
        payload["select_message"],
        serde_json::json!("Select OpenAI Codex login method:")
    );
    assert_eq!(
        payload["select_options"],
        serde_json::json!([
            "browser=Browser login (default)",
            "device_code=Device code login (headless)"
        ])
    );

    // The authorization URL reaches Lua once, with its instructions, and the
    // device-code callback is never used on this path.
    assert_eq!(payload["auth_calls"], serde_json::json!(1));
    assert_eq!(payload["device_code_calls"], serde_json::json!(0));
    let auth_url = payload["auth_url"].as_str().expect("authorization url");
    assert!(
        auth_url.starts_with(&format!("{base}/oauth/authorize?")),
        "authorization url {auth_url:?} should address the fixture endpoint"
    );
    assert!(auth_url.contains("code_challenge_method=S256"));
    assert!(
        payload["instructions"]
            .as_str()
            .is_some_and(|text| text.contains("browser"))
    );

    // The exchanged row comes back whole, including provider-defined extras.
    assert_eq!(payload["refresh"], serde_json::json!("refresh-browser"));
    assert_eq!(payload["access"], serde_json::json!(access));
    assert_eq!(payload["account"], serde_json::json!("acct-browser"));
    assert_eq!(payload["expires_is_number"], serde_json::json!(true));

    // Storing it is the package's own follow-up through the existing store.
    assert_eq!(
        payload["stored_providers"],
        serde_json::json!(["openai-codex"])
    );
    assert_eq!(payload["described_kind"], serde_json::json!("oauth"));
    assert_eq!(payload["described_extra"], serde_json::json!(["accountId"]));
    assert_eq!(payload["resolved_key"], serde_json::json!(access));
    assert_eq!(payload["resolved_refreshed"], serde_json::json!(false));

    // The bounds are part of the surface, not folklore.
    assert_eq!(
        payload["default_login_timeout_ms"],
        serde_json::json!(900_000)
    );
    assert_eq!(
        payload["max_login_timeout_ms"],
        serde_json::json!(3_600_000)
    );
    assert_eq!(payload["max_login_models"], serde_json::json!(128));

    // Manual code entry answered the flow, so exactly one exchange happened.
    let targets = targets.lock().expect("recorded targets").clone();
    assert_eq!(targets, vec!["/oauth/token".to_owned()]);
}

#[test]
fn file_backed_package_drives_a_device_code_login_and_enables_models() {
    let expires_at = pi_rs_ai_types::now_ms() / 1000 + 3600;
    let (base, targets) = fixture_endpoints(vec![
        (
            "/login/device/code".to_owned(),
            serde_json::json!({
                "device_code": "dev-1",
                "user_code": "WXYZ",
                "verification_uri": "https://github.com/login/device",
                "interval": 1,
                "expires_in": 900,
            })
            .to_string(),
        ),
        (
            "/login/oauth/access_token".to_owned(),
            serde_json::json!({ "access_token": "github-access" }).to_string(),
        ),
        (
            "/copilot/token".to_owned(),
            serde_json::json!({
                "token": "tid=1;proxy-ep=proxy.individual.githubcopilot.com;",
                "expires_at": expires_at,
            })
            .to_string(),
        ),
        ("/models/gpt-test/policy".to_owned(), "{}".to_owned()),
    ]);
    pi_rs_ai_auth::register_oauth_provider(Arc::new(GitHubCopilotFlow {
        endpoints_override: Some(GitHubCopilotEndpoints {
            device_code_url: format!("{base}/login/device/code"),
            access_token_url: format!("{base}/login/oauth/access_token"),
            copilot_token_url: format!("{base}/copilot/token"),
        }),
        policy_base_url_override: Some(base.clone()),
        // Empty on purpose: the catalog rows to enable come from Lua.
        model_ids: Vec::new(),
    }));

    let directory = tempfile::tempdir().expect("temporary directory");
    let host = host_with(directory.path());
    load(
        &host,
        directory.path(),
        "device-login",
        DEVICE_LOGIN_PACKAGE,
    );
    let batch = dispatch(&host);
    let payload = &batch.actions[0].payload;

    // The enterprise prompt is served by Lua, blank meaning github.com.
    assert!(
        payload["prompt"]
            .as_str()
            .is_some_and(|text| text.contains("GitHub Enterprise")),
        "prompt {:?} should name the enterprise question",
        payload["prompt"]
    );
    assert_eq!(payload["placeholder"], serde_json::json!("company.ghe.com"));
    assert_eq!(payload["allow_empty"], serde_json::json!(true));
    assert_eq!(payload["enterprise"], serde_json::Value::Null);

    // The device code reaches Lua with the polling parameters the flow used.
    assert_eq!(payload["user_code"], serde_json::json!("WXYZ"));
    assert_eq!(
        payload["verification_uri"],
        serde_json::json!("https://github.com/login/device")
    );
    assert_eq!(payload["interval_seconds"], serde_json::json!(1));
    assert_eq!(payload["expires_in_seconds"], serde_json::json!(900));

    // Progress and the post-login model list are ordinary Lua callbacks.
    assert_eq!(
        payload["progress"],
        serde_json::json!(["Enabling models..."])
    );
    assert_eq!(
        payload["model_id_calls"],
        serde_json::json!(["github-copilot"])
    );
    assert_eq!(payload["refresh"], serde_json::json!("github-access"));
    assert!(
        payload["access"]
            .as_str()
            .is_some_and(|token| token.starts_with("tid=1;")),
        "copilot token {:?} should be the exchanged short-lived token",
        payload["access"]
    );

    let targets = targets.lock().expect("recorded targets").clone();
    assert!(
        targets.contains(&"/models/gpt-test/policy".to_owned()),
        "recorded targets {targets:?} should include the Lua-chosen model policy"
    );
}

#[test]
fn unknown_providers_missing_callbacks_and_broken_bounds_are_refused() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let host = host_with(directory.path());
    load(&host, directory.path(), "refusing", REFUSING_PACKAGE);
    let batch = dispatch(&host);
    let payload = &batch.actions[0].payload;

    for (field, fragment) in [
        ("unknown_provider", "Unknown OAuth provider"),
        ("missing_callback", "requires callback on_select"),
        ("zero_timeout", "timeout_ms must be 1..=3600000"),
        ("oversize_timeout", "timeout_ms must be 1..=3600000"),
        ("oversize_model_ids", "more than 128 ids"),
    ] {
        assert_eq!(
            payload[field]["ok"],
            serde_json::json!(false),
            "{field} should be refused"
        );
        let message = payload[field]["message"].as_str().expect("diagnostic");
        assert!(
            message.contains(fragment),
            "{field} diagnostic {message:?} should name {fragment:?}"
        );
    }
}

#[test]
fn a_parked_login_is_cancelled_with_its_dispatch_scope() {
    let (base, targets) = fixture_endpoints(vec![(
        "/oauth/token".to_owned(),
        HANGING_RESPONSE.to_owned(),
    )]);
    let mut flow = pi_rs_ai_auth::anthropic_flow();
    flow.authorize_url = format!("{base}/oauth/authorize");
    flow.token_url = format!("{base}/oauth/token");
    flow.callback_port = free_port();
    pi_rs_ai_auth::register_oauth_provider(Arc::new(flow));

    let directory = tempfile::tempdir().expect("temporary directory");
    let host = host_with(directory.path());
    let path = directory.path().join("hanging-login.lua");
    std::fs::write(&path, HANGING_LOGIN_PACKAGE).expect("write file-backed package");
    let handle = host
        .load_package(PackageSource::File { path: &path })
        .expect("file-backed package loads");

    let caller = host.clone();
    let login = std::thread::spawn(move || {
        caller.dispatch(DispatchRequest::new(
            RootKind::Application,
            serde_json::json!({ "kind": "startup" }),
            serde_json::json!({}),
        ))
    });

    // The pasted code has been exchanged against an endpoint that never
    // answers, so the dispatch is parked on the token POST.
    let started = std::time::Instant::now();
    while targets.lock().expect("recorded targets").is_empty() {
        assert!(
            started.elapsed() < std::time::Duration::from_secs(10),
            "token exchange never reached the fixture endpoint"
        );
        std::thread::sleep(std::time::Duration::from_millis(10));
    }

    // Disposing the package cancels its dispatch scope; the parked login
    // observes the same token every other host mechanism does.
    host.dispose_package(&handle).expect("package disposes");
    let outcome = login.join().expect("login thread joins");
    assert!(
        matches!(outcome, Err(pi_rs_host::HostError::Cancelled)),
        "a cancelled login should report as a cancelled dispatch, got {outcome:?}"
    );
    assert!(
        started.elapsed() < std::time::Duration::from_secs(10),
        "cancellation should not wait for the login timeout"
    );
}
