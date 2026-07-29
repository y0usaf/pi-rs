//! Credential storage driven by an ordinary file-backed package.
//!
//! Rust contributes storage mechanics only: canonical-first selection with a
//! read-only legacy fallback, locking, atomic replacement, private modes,
//! stored-value expansion, and OAuth refresh. Every location, every provider
//! name, and the entire precedence policy are authored in Lua and arrive
//! through the public environment/path effects, so nothing private links this
//! test to the package it drives.

#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

use pi_rs_host::kernel::{DispatchBatch, DispatchRequest, RootKind};
use pi_rs_host::{Host, HostConfig, PackageSource};

/// One package owning the whole credential journey: legacy fallback, canonical
/// promotion, api-key and OAuth rows, resolution, and removal.
const CREDENTIAL_PACKAGE: &str = r#"
local pi = ...
local auth = pi.auth.v1
local effects = pi.effects.v1
local roots = pi.roots.v1

-- Both locations are the package's own decision.
local home = effects.env.get("PI_TEST_HOME")
local canonical = effects.path.join(home, "state", "credentials.json")
local legacy = effects.path.join(home, "legacy", "auth.json")

roots.register({
  kind = "application",
  id = "credential-store",
  dispatch = function()
    local store = auth.store({ canonical = canonical, legacy = legacy })

    -- The legacy file is the only one on disk: it is selected, readable, and
    -- resolvable, and the package never writes to it.
    local legacy_snapshot = store:snapshot()
    local legacy_described = store:describe("legacy-provider")
    local legacy_resolved = store:resolve("legacy-provider")

    -- Writing promotes storage to the canonical location. The stored value is
    -- a command-backed expression, expanded only when resolved.
    store:set_api_key("fixture-provider", "!printf resolved-fixture-secret")
    local promoted = store:snapshot()
    local api_key_described = store:describe("fixture-provider")
    local resolved = store:resolve("fixture-provider")

    -- An OAuth row keeps provider-defined extra fields verbatim and reports
    -- expiry without exposing a token.
    store:set_oauth("anthropic", {
      refresh = "refresh-token",
      access = "access-token",
      expires = 4102444800000,
      account = "fixture-account",
    })
    local oauth_described = store:describe("anthropic")
    local oauth_resolved = store:resolve("anthropic")

    store:remove("fixture-provider")
    local after_removal = store:snapshot()
    local removed_described = store:describe("fixture-provider")

    -- Subscription inventory is mechanism data, like the model catalog.
    local subscription_ids = {}
    local callback_server = {}
    for _, provider in ipairs(auth.providers()) do
      subscription_ids[#subscription_ids + 1] = provider.id
      callback_server[provider.id] = provider.uses_callback_server
    end

    roots.action("credentials", {
      legacy_source = legacy_snapshot.source,
      legacy_providers = legacy_snapshot.providers,
      legacy_kind = legacy_described.kind,
      legacy_key = legacy_resolved.api_key,
      legacy_refreshed = legacy_resolved.refreshed,
      promoted_source = promoted.source,
      promoted_providers = promoted.providers,
      api_key_kind = api_key_described.kind,
      resolved_key = resolved.api_key,
      resolved_refreshed = resolved.refreshed,
      oauth_kind = oauth_described.kind,
      oauth_expires = oauth_described.expires,
      oauth_expired = oauth_described.expired,
      oauth_extra_fields = oauth_described.extra_fields,
      oauth_key = oauth_resolved.api_key,
      oauth_refreshed = oauth_resolved.refreshed,
      after_removal = after_removal.providers,
      removed_described = removed_described == nil,
      subscription_ids = subscription_ids,
      anthropic_callback_server = callback_server["anthropic"],
      max_secret_bytes = auth.max_secret_bytes,
      max_providers = auth.max_providers,
    })
  end,
})
"#;

/// Every refusal a login/configuration package meets before a secret exists.
const REFUSING_PACKAGE: &str = r#"
local pi = ...
local auth = pi.auth.v1
local effects = pi.effects.v1
local roots = pi.roots.v1

local home = effects.env.get("PI_TEST_HOME")
local canonical = effects.path.join(home, "state", "credentials.json")

local function refusal(fn)
  local ok, error_value = pcall(fn)
  return { ok = ok, message = tostring(error_value) }
end

roots.register({
  kind = "application",
  id = "credential-refusals",
  dispatch = function()
    local store = auth.store({ canonical = canonical })
    local empty = store:snapshot()

    roots.action("refused", {
      absent_source = empty.source,
      absent_providers = #empty.providers,
      absent_described = store:describe("fixture-provider") == nil,
      absent_resolved = store:resolve("fixture-provider") == nil,
      relative_canonical = refusal(function()
        return auth.store({ canonical = "state/credentials.json" })
      end),
      same_paths = refusal(function()
        return auth.store({ canonical = canonical, legacy = canonical })
      end),
      blank_provider = refusal(function() return store:set_api_key("  ", "value") end),
      oversize_secret = refusal(function()
        return store:set_api_key("fixture-provider", string.rep("x", auth.max_secret_bytes + 1))
      end),
      unknown_oauth_provider = refusal(function()
        store:set_oauth("not-a-subscription", {
          refresh = "refresh-token",
          access = "access-token",
          expires = 4102444800000,
        })
        return store:resolve("not-a-subscription")
      end),
      incomplete_oauth = refusal(function()
        return store:set_oauth("anthropic", { access = "access-token" })
      end),
    })
  end,
})
"#;

fn host_with(home: &std::path::Path) -> Host {
    let environment = [(
        "PI_TEST_HOME".to_owned(),
        home.to_string_lossy().into_owned(),
    )]
    .into_iter()
    .collect();
    Host::new(HostConfig {
        environment: Some(environment),
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

const LEGACY_FILE: &str = r#"{"legacy-provider":{"type":"api_key","key":"legacy-fixture-secret"}}"#;

#[test]
fn file_backed_package_owns_credential_locations_and_resolution() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let home = directory.path().join("home");
    let legacy = home.join("legacy").join("auth.json");
    std::fs::create_dir_all(legacy.parent().expect("legacy parent")).expect("legacy directory");
    std::fs::write(&legacy, LEGACY_FILE).expect("write legacy credentials");

    let host = host_with(&home);
    load(&host, directory.path(), "credentials", CREDENTIAL_PACKAGE);
    let batch = dispatch(&host);
    let payload = &batch.actions[0].payload;

    // Legacy is a read-only fallback: selected while canonical is absent.
    assert_eq!(payload["legacy_source"], serde_json::json!("legacy"));
    assert_eq!(
        payload["legacy_providers"],
        serde_json::json!(["legacy-provider"])
    );
    assert_eq!(payload["legacy_kind"], serde_json::json!("api_key"));
    assert_eq!(
        payload["legacy_key"],
        serde_json::json!("legacy-fixture-secret")
    );
    assert_eq!(payload["legacy_refreshed"], serde_json::json!(false));

    // The first write promotes storage to the canonical file: the selected
    // legacy rows migrate forward with it, so nothing a package could already
    // resolve disappears once canonical exists.
    assert_eq!(payload["promoted_source"], serde_json::json!("canonical"));
    assert_eq!(
        payload["promoted_providers"],
        serde_json::json!(["fixture-provider", "legacy-provider"])
    );
    // The legacy file itself stays byte-identical: it is never written.

    assert_eq!(
        std::fs::read_to_string(&legacy).expect("legacy still readable"),
        LEGACY_FILE
    );

    // A stored api-key row is an expression; resolution expands it.
    assert_eq!(payload["api_key_kind"], serde_json::json!("api_key"));
    assert_eq!(
        payload["resolved_key"],
        serde_json::json!("resolved-fixture-secret")
    );
    assert_eq!(payload["resolved_refreshed"], serde_json::json!(false));

    // An OAuth row keeps its provider-defined extra data and reports expiry
    // without ever exposing a token through `describe`.
    assert_eq!(payload["oauth_kind"], serde_json::json!("oauth"));
    assert_eq!(
        payload["oauth_expires"],
        serde_json::json!(4_102_444_800_000_i64)
    );
    assert_eq!(payload["oauth_expired"], serde_json::json!(false));
    assert_eq!(
        payload["oauth_extra_fields"],
        serde_json::json!(["account"])
    );
    assert_eq!(payload["oauth_key"], serde_json::json!("access-token"));
    assert_eq!(payload["oauth_refreshed"], serde_json::json!(false));

    // Removal leaves the other rows alone.
    assert_eq!(
        payload["after_removal"],
        serde_json::json!(["anthropic", "legacy-provider"])
    );
    assert_eq!(payload["removed_described"], serde_json::json!(true));

    // The canonical file is private to its owner and never world-readable.
    let canonical = home.join("state").join("credentials.json");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&canonical)
            .expect("canonical metadata")
            .permissions()
            .mode();
        assert_eq!(mode & 0o077, 0, "canonical credentials stay private");
    }
    let stored: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&canonical).expect("canonical readable"))
            .expect("canonical json");
    assert_eq!(stored["anthropic"]["type"], serde_json::json!("oauth"));
    assert_eq!(
        stored["anthropic"]["account"],
        serde_json::json!("fixture-account")
    );

    // Subscription inventory is reported, not chosen.
    assert_eq!(
        payload["subscription_ids"],
        serde_json::json!(["anthropic", "github-copilot", "openai-codex"])
    );
    assert_eq!(
        payload["anthropic_callback_server"],
        serde_json::json!(true)
    );

    // The bounds are part of the surface, not folklore.
    assert_eq!(payload["max_secret_bytes"], serde_json::json!(65536));
    assert_eq!(payload["max_providers"], serde_json::json!(256));
}

#[test]
fn invalid_locations_providers_and_secrets_are_refused() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let home = directory.path().join("home");
    let host = host_with(&home);
    load(&host, directory.path(), "refusing", REFUSING_PACKAGE);
    let batch = dispatch(&host);
    let payload = &batch.actions[0].payload;

    // Absence is data: no file, no providers, no error.
    assert_eq!(payload["absent_source"], serde_json::json!("absent"));
    assert_eq!(payload["absent_providers"], serde_json::json!(0));
    assert_eq!(payload["absent_described"], serde_json::json!(true));
    assert_eq!(payload["absent_resolved"], serde_json::json!(true));

    for (field, fragment) in [
        ("relative_canonical", "must be absolute"),
        ("same_paths", "must differ"),
        ("blank_provider", "invalid credential provider id"),
        ("oversize_secret", "exceeds 65536 bytes"),
        ("unknown_oauth_provider", "Unknown OAuth provider"),
        ("incomplete_oauth", "invalid oauth credential"),
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
