//! Deterministic acceptance for the shipped Lua configuration package.
//!
//! Every scenario drives the ordinary file-backed configuration package
//! through the public kernel transaction. The host contributes an immutable
//! environment snapshot, path arithmetic, bounded filesystem effects, the
//! append-only record store, and package composition; every directory name,
//! precedence rule, trust decision, merge rule, and default lives in the Lua
//! under `crates/pi-rs-builtins/config/`.
//!
//! The matrices below are the PLAN 4.2 acceptance: precedence, trust,
//! rollback, and idempotence, plus complete inspectability of the effective
//! configuration.

#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use pi_rs_host::kernel::{DispatchBatch, DispatchRequest, RootKind};
use pi_rs_host::{Host, HostConfig, PackageSource};
use serde_json::{Value, json};

// ---------------------------------------------------------------------------
// Harness
// ---------------------------------------------------------------------------

/// The configuration package in load order. `init.lua` requires the other
/// modules, so they are declared first, exactly as a distribution manifest
/// would list them.
fn package_files() -> Vec<PathBuf> {
    let root = pi_rs_builtins::package_root().join("config");
    [
        "json.lua",
        "paths.lua",
        "schema.lua",
        "trust.lua",
        "defaults.lua",
        "apply.lua",
        "init.lua",
    ]
    .into_iter()
    .map(|file| root.join(file))
    .collect()
}

/// Driver root: performs one optional command per dispatch and republishes
/// everything the configuration package makes inspectable. Snapshot values are
/// read-only proxies, so anything sent onward is deep-copied into a plain
/// table first.
const DRIVER: &str = r#"
local pi = ...
local roots = pi.roots.v1
local fs = pi.effects.v1.fs
local settings = pi.kernel.v1.module.require("pi.config.settings", "1")

local function clone(value)
  if type(value) ~= "table" then
    return value
  end
  local out = {}
  for key, item in pairs(value) do
    out[key] = clone(item)
  end
  return out
end

local function packages()
  local out = {}
  for index, row in ipairs(pi.packages.v1.list()) do
    out[index] = { source = row.source, scope = row.scope }
  end
  return out
end

-- Declarations from the consumer's side: nothing here knows a configuration
-- package exists, only that the kernel carries declarations of these kinds.
local function registered(kind)
  local out = {}
  for index, entry in ipairs(pi.kernel.v1.registered(kind)) do
    out[index] = clone(entry)
  end
  return out
end

roots.register({
  kind = "application",
  id = "config-driver",
  dispatch = function(snapshot)
    local event = snapshot.event
    if event.kind == "trust" then
      local ok, changed = pcall(settings.trust, event.directory, event.decision)
      roots.action("trust_result", {
        ok = ok,
        changed = changed,
        error = not ok and tostring(changed) or nil,
      })
    elseif event.kind == "reload" then
      local report = settings.reload()
      roots.action("reload_result", {
        ok = report.ok,
        changed = report.changed,
        revision = report.revision,
        errors = clone(report.errors),
      })
    elseif event.kind == "write" then
      fs.write(event.path, event.contents)
    elseif event.kind == "remove" then
      fs.remove_file(event.path)
    end

    roots.action("report", {
      revision = settings.revision(),
      loaded = settings.loaded(),
      effective = settings.effective(),
      provenance = settings.provenance(),
      sources = settings.sources(),
      leaves = settings.leaves(),
      errors = settings.errors(),
      packages = packages(),
      resources = settings.resources(),
      roots = settings.roots(),
      trust_list = settings.trust_list(),
      declarations = settings.declarations(),
      modules = settings.modules(),
      themes = registered("theme"),
      keymaps = registered("keymap"),
      providers = registered("provider"),
      event_config = clone(event.config),
      event_config_revision = event.config_revision,
      event_model = event.model and event.model.id or nil,
    })
  end,
})
"#;

/// A package the configuration may select. It defines a module, so its
/// presence is observable without any privileged hook.
const SELECTED_PACKAGE: &str = r#"
local pi = ...
pi.kernel.v1.module.define({
  name = "__NAME__",
  version = "1",
  factory = function()
    return { selected = true }
  end,
})
"#;

struct Fixture {
    root: tempfile::TempDir,
}

impl Fixture {
    fn new() -> Self {
        let fixture = Self {
            root: tempfile::tempdir().expect("temporary root"),
        };
        for directory in [
            fixture.home(),
            fixture.config_root(),
            fixture.data_root(),
            fixture.state_root(),
            fixture.cache_root(),
            fixture.packages_directory(),
            fixture.project(),
        ] {
            std::fs::create_dir_all(directory).expect("fixture directory");
        }
        fixture
    }

    fn path(&self, relative: &str) -> PathBuf {
        self.root.path().join(relative)
    }

    fn home(&self) -> PathBuf {
        self.path("home")
    }

    fn config_root(&self) -> PathBuf {
        self.path("xdg/config")
    }

    fn data_root(&self) -> PathBuf {
        self.path("xdg/data")
    }

    fn state_root(&self) -> PathBuf {
        self.path("xdg/state")
    }

    fn cache_root(&self) -> PathBuf {
        self.path("xdg/cache")
    }

    fn config_file(&self) -> PathBuf {
        self.config_root().join("pi/config.lua")
    }

    fn legacy_settings(&self) -> PathBuf {
        self.home().join(".pi/agent/settings.json")
    }

    fn packages_directory(&self) -> PathBuf {
        self.data_root().join("pi/packages")
    }

    fn trust_store(&self) -> PathBuf {
        self.state_root().join("pi/trust/trust.jsonl")
    }

    fn project(&self) -> PathBuf {
        self.path("project")
    }

    fn project_config(&self) -> PathBuf {
        self.project().join(".pi/config.lua")
    }

    fn write(&self, path: &Path, contents: &str) {
        std::fs::create_dir_all(path.parent().expect("parent directory")).expect("parent");
        std::fs::write(path, contents).expect("fixture file");
    }

    fn write_config(&self, contents: &str) {
        self.write(&self.config_file(), contents);
    }

    fn write_package(&self, name: &str) -> PathBuf {
        let path = self.packages_directory().join(name);
        let module = format!("pi.test.{}", name.trim_end_matches(".lua"));
        self.write(&path, &SELECTED_PACKAGE.replace("__NAME__", &module));
        path
    }

    fn environment(&self) -> BTreeMap<String, String> {
        let mut environment = BTreeMap::new();
        environment.insert("HOME".to_owned(), text(&self.home()));
        environment.insert("XDG_CONFIG_HOME".to_owned(), text(&self.config_root()));
        environment.insert("XDG_DATA_HOME".to_owned(), text(&self.data_root()));
        environment.insert("XDG_STATE_HOME".to_owned(), text(&self.state_root()));
        environment.insert("XDG_CACHE_HOME".to_owned(), text(&self.cache_root()));
        environment
    }
}

fn text(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

fn start(environment: BTreeMap<String, String>) -> Host {
    let host = Host::new(HostConfig {
        environment: Some(environment),
        ..HostConfig::default()
    })
    .expect("host starts");
    for file in package_files() {
        host.load_package(PackageSource::File { path: &file })
            .unwrap_or_else(|error| panic!("configuration package {}: {error}", file.display()));
    }
    let directory = tempfile::tempdir().expect("driver directory");
    let driver = directory.path().join("config-driver.lua");
    std::fs::write(&driver, DRIVER).expect("driver package");
    host.load_package(PackageSource::File { path: &driver })
        .expect("driver package loads");
    std::mem::forget(directory);
    host
}

/// Every dispatch carries the project root the launcher publishes, which is
/// what makes the project layer discoverable.
fn dispatch(host: &Host, fixture: &Fixture, event: Value) -> DispatchBatch {
    host.dispatch(DispatchRequest::new(
        RootKind::Application,
        event,
        json!({ "root": text(&fixture.project()) }),
    ))
    .expect("dispatch succeeds")
}

fn report(batch: &DispatchBatch) -> &Value {
    action(batch, "report")
}

fn action<'a>(batch: &'a DispatchBatch, kind: &str) -> &'a Value {
    &batch
        .actions
        .iter()
        .find(|action| action.kind == kind)
        .unwrap_or_else(|| panic!("action {kind} is published"))
        .payload
}

/// An empty Lua table crosses as an empty JSON object, so a list accessor has
/// to accept both shapes.
fn rows(value: &Value) -> Vec<Value> {
    value.as_array().cloned().unwrap_or_default()
}

fn source_row(payload: &Value, layer: &str) -> Value {
    rows(&payload["sources"])
        .into_iter()
        .find(|row| row["layer"] == layer)
        .unwrap_or_else(|| panic!("source row for layer {layer}"))
}

/// Source key of the package the configuration loads to carry its own
/// declarations. It is an ordinary Lua-loaded package, so it appears in the
/// live package list beside the ones the configuration selected.
const DECLARATION_PACKAGE: &str = "pi.config.declarations";

fn live_packages(payload: &Value) -> Vec<String> {
    rows(&payload["packages"])
        .into_iter()
        .map(|row| row["source"].as_str().unwrap_or_default().to_owned())
        .collect()
}

/// The packages the *configuration file* selected, which is what every
/// selection, rollback, and idempotence assertion is about.
fn package_sources(payload: &Value) -> Vec<String> {
    live_packages(payload)
        .into_iter()
        .filter(|source| source != DECLARATION_PACKAGE)
        .collect()
}

fn file_state(path: &Path) -> (Vec<u8>, std::time::SystemTime) {
    let metadata = std::fs::metadata(path).expect("legacy metadata");
    (
        std::fs::read(path).expect("legacy contents"),
        metadata.modified().expect("legacy mtime"),
    )
}

// ---------------------------------------------------------------------------
// Precedence
// ---------------------------------------------------------------------------

#[test]
fn the_canonical_configuration_wins_and_the_legacy_file_is_never_read() {
    let fixture = Fixture::new();
    fixture.write_config(r#"return { theme = "canonical" }"#);
    fixture.write(
        &fixture.legacy_settings(),
        r#"{ "theme": "legacy", "packages": ["never.lua"] }"#,
    );
    let before = file_state(&fixture.legacy_settings());

    let host = start(fixture.environment());
    let batch = dispatch(&host, &fixture, json!({ "kind": "report" }));
    let payload = report(&batch);

    assert_eq!(payload["effective"]["theme"], "canonical");
    assert_eq!(payload["effective"]["packages"], json!({}));
    let user = source_row(payload, "user");
    assert_eq!(user["outcome"], "selected");
    assert_eq!(user["kind"], "lua");
    assert_eq!(user["source"], text(&fixture.config_file()));
    assert_eq!(
        payload["provenance"]["theme"]["source"],
        text(&fixture.config_file())
    );

    // A resource that lost only reports its counterpart; nothing touches it.
    let config = &payload["resources"]["config"];
    assert_eq!(config["source"], "canonical");
    assert_eq!(config["selected"], text(&fixture.config_file()));
    assert_eq!(config["legacy"], text(&fixture.legacy_settings()));
    assert_eq!(config["destination"], text(&fixture.config_file()));
    assert_eq!(before, file_state(&fixture.legacy_settings()));
}

#[test]
fn the_legacy_settings_file_is_read_only_when_the_canonical_file_is_absent() {
    let fixture = Fixture::new();
    fixture.write(
        &fixture.legacy_settings(),
        r#"{ "theme": "legacy", "keymaps": { "ctrl+k": "clear" }, "editor": "vim", "model": null }"#,
    );
    let before = file_state(&fixture.legacy_settings());

    let host = start(fixture.environment());
    let batch = dispatch(&host, &fixture, json!({ "kind": "report" }));
    let payload = report(&batch);

    assert_eq!(payload["effective"]["theme"], "legacy");
    assert_eq!(payload["effective"]["keymaps"]["ctrl+k"], "clear");
    assert_eq!(payload["effective"]["model"], Value::Null);
    let user = source_row(payload, "user");
    assert_eq!(user["kind"], "json");
    assert_eq!(user["source"], text(&fixture.legacy_settings()));
    // A key pi-rs does not define is reported, not fatal: the promise for this
    // file is storage provenance, not format compatibility.
    let diagnostic = user["diagnostic"].as_str().expect("legacy diagnostic");
    assert!(diagnostic.contains("editor"), "{diagnostic}");
    assert!(diagnostic.contains("model"), "{diagnostic}");
    assert_eq!(payload["resources"]["config"]["source"], "legacy");
    assert_eq!(before, file_state(&fixture.legacy_settings()));
}

#[test]
fn a_broken_canonical_file_never_falls_through_to_the_legacy_file() {
    let fixture = Fixture::new();
    fixture.write_config("return { theme = }");
    fixture.write(&fixture.legacy_settings(), r#"{ "theme": "legacy" }"#);

    let host = start(fixture.environment());
    let batch = dispatch(&host, &fixture, json!({ "kind": "report" }));
    let payload = report(&batch);

    assert_eq!(payload["loaded"], false);
    assert_eq!(payload["revision"], 0);
    assert_eq!(payload["effective"], json!({}));
    let user = source_row(payload, "user");
    assert_eq!(user["outcome"], "invalid");
    assert_eq!(user["source"], text(&fixture.config_file()));
    let errors = rows(&payload["errors"]);
    assert_eq!(errors.len(), 1);
    let message = errors[0].as_str().expect("error text");
    assert!(message.starts_with("user: "), "{message}");
    assert!(message.contains("config.lua"), "{message}");
}

// ---------------------------------------------------------------------------
// Trust
// ---------------------------------------------------------------------------

#[test]
fn project_configuration_is_inert_until_its_directory_is_trusted() {
    let fixture = Fixture::new();
    fixture.write_config(
        r#"return { theme = "user", model = { provider = "openai", id = "gpt-5.1" } }"#,
    );
    fixture.write(&fixture.project_config(), r#"return { theme = "project" }"#);

    let host = start(fixture.environment());
    let undecided = report(&dispatch(&host, &fixture, json!({ "kind": "report" }))).clone();
    assert_eq!(undecided["effective"]["theme"], "user");
    let project = source_row(&undecided, "project");
    assert_eq!(project["outcome"], "untrusted");
    assert_eq!(project["source"], text(&fixture.project_config()));
    assert!(!fixture.trust_store().exists(), "no decision, no store");

    // Trusting is one durable decision; the next composition uses it.
    let trusted = dispatch(
        &host,
        &fixture,
        json!({ "kind": "trust", "directory": text(&fixture.project()), "decision": "trust" }),
    );
    assert_eq!(action(&trusted, "trust_result")["changed"], true);
    let after = report(&dispatch(&host, &fixture, json!({ "kind": "reload" }))).clone();
    assert_eq!(after["effective"]["theme"], "project");
    assert_eq!(source_row(&after, "project")["outcome"], "selected");
    // The higher layer replaced only what it declared.
    assert_eq!(after["effective"]["model"]["id"], "gpt-5.1");
    assert_eq!(
        after["provenance"]["theme"]["source"],
        text(&fixture.project_config())
    );
    assert_eq!(after["provenance"]["model.id"]["layer"], "user");

    // Repeating a decision writes nothing: the store is append-only, so
    // idempotence has to be a property of the decision, not of the file.
    let repeated = dispatch(
        &host,
        &fixture,
        json!({ "kind": "trust", "directory": text(&fixture.project()), "decision": "trust" }),
    );
    assert_eq!(action(&repeated, "trust_result")["changed"], false);
    let records = std::fs::read_to_string(fixture.trust_store()).expect("trust store");
    assert_eq!(records.lines().count(), 2, "header plus one decision");

    // Revoking is an ordinary later record and takes the layer away again.
    let denied = dispatch(
        &host,
        &fixture,
        json!({ "kind": "trust", "directory": text(&fixture.project()), "decision": "deny" }),
    );
    assert_eq!(action(&denied, "trust_result")["changed"], true);
    let revoked = report(&dispatch(&host, &fixture, json!({ "kind": "reload" }))).clone();
    assert_eq!(revoked["effective"]["theme"], "user");
    assert_eq!(source_row(&revoked, "project")["outcome"], "denied");
    let history = std::fs::read_to_string(fixture.trust_store()).expect("trust store");
    assert_eq!(history.lines().count(), 3, "header plus two decisions");
    assert_eq!(
        rows(&revoked["trust_list"])[0]["directory"],
        text(&fixture.project())
    );
}

#[test]
fn a_trust_decision_covers_only_the_directory_it_names() {
    let fixture = Fixture::new();
    fixture.write_config(r#"return { theme = "user" }"#);
    fixture.write(&fixture.project_config(), r#"return { theme = "project" }"#);

    let host = start(fixture.environment());
    dispatch(
        &host,
        &fixture,
        json!({ "kind": "trust", "directory": text(&fixture.home()), "decision": "trust" }),
    );
    let payload = report(&dispatch(&host, &fixture, json!({ "kind": "reload" }))).clone();
    assert_eq!(payload["effective"]["theme"], "user");
    assert_eq!(source_row(&payload, "project")["outcome"], "untrusted");
}

// ---------------------------------------------------------------------------
// Rollback and idempotence
// ---------------------------------------------------------------------------

#[test]
fn a_failed_reload_keeps_the_published_configuration_and_its_packages() {
    let fixture = Fixture::new();
    let selected = fixture.write_package("extra.lua");
    fixture.write_config(r#"return { theme = "one", packages = { "extra.lua" } }"#);

    let host = start(fixture.environment());
    let first = report(&dispatch(&host, &fixture, json!({ "kind": "report" }))).clone();
    assert_eq!(first["revision"], 1);
    assert_eq!(package_sources(&first), vec![text(&selected)]);

    // Each failure mode leaves settings, packages, and revision untouched and
    // reports why.
    for (contents, fragment) in [
        ("return { theme = }", "config.lua"),
        (r#"return { theme = 3 }"#, "theme must be a string"),
        (r#"return { tehme = "typo" }"#, "unknown key tehme"),
        (
            r#"return { theme = "two", packages = { "absent.lua" } }"#,
            "cannot load",
        ),
    ] {
        dispatch(
            &host,
            &fixture,
            json!({ "kind": "write", "path": text(&fixture.config_file()), "contents": contents }),
        );
        let batch = dispatch(&host, &fixture, json!({ "kind": "reload" }));
        let result = action(&batch, "reload_result");
        assert_eq!(result["ok"], false, "{contents}");
        assert_eq!(result["revision"], 1, "{contents}");
        let message = rows(&result["errors"])
            .first()
            .and_then(|value| value.as_str().map(str::to_owned))
            .unwrap_or_default();
        assert!(message.contains(fragment), "{contents}: {message}");

        let payload = report(&batch);
        assert_eq!(payload["revision"], 1, "{contents}");
        assert_eq!(payload["effective"]["theme"], "one", "{contents}");
        assert_eq!(
            package_sources(payload),
            vec![text(&selected)],
            "{contents}"
        );
    }

    // A valid file publishes again from the same live state.
    dispatch(
        &host,
        &fixture,
        json!({
            "kind": "write",
            "path": text(&fixture.config_file()),
            "contents": r#"return { theme = "two", packages = { "extra.lua" } }"#,
        }),
    );
    let payload = report(&dispatch(&host, &fixture, json!({ "kind": "reload" }))).clone();
    assert_eq!(payload["revision"], 2);
    assert_eq!(payload["effective"]["theme"], "two");
    assert_eq!(package_sources(&payload), vec![text(&selected)]);
}

#[test]
fn recomposing_an_unchanged_configuration_publishes_nothing() {
    let fixture = Fixture::new();
    let selected = fixture.write_package("extra.lua");
    fixture.write_config(r#"return { theme = "one", packages = { "extra.lua" } }"#);

    let host = start(fixture.environment());
    let first = report(&dispatch(&host, &fixture, json!({ "kind": "report" }))).clone();
    let scopes = rows(&first["packages"]);

    for _ in 0..2 {
        let batch = dispatch(&host, &fixture, json!({ "kind": "reload" }));
        let result = action(&batch, "reload_result");
        assert_eq!(result["ok"], true);
        assert_eq!(result["changed"], false);
        assert_eq!(result["revision"], 1);
        // Nothing was disposed and reloaded: the same package scope is live.
        assert_eq!(rows(&report(&batch)["packages"]), scopes);
    }

    // A changed selection swaps the generation: the replacement loads before
    // the retired package is disposed, and only the new one stays.
    let replacement = fixture.write_package("second.lua");
    dispatch(
        &host,
        &fixture,
        json!({
            "kind": "write",
            "path": text(&fixture.config_file()),
            "contents": r#"return { theme = "one", packages = { "second.lua" } }"#,
        }),
    );
    let payload = report(&dispatch(&host, &fixture, json!({ "kind": "reload" }))).clone();
    assert_eq!(payload["revision"], 2);
    assert_eq!(package_sources(&payload), vec![text(&replacement)]);
    assert_ne!(package_sources(&payload), vec![text(&selected)]);
}

#[test]
fn a_duplicate_package_entry_is_refused_before_anything_loads() {
    let fixture = Fixture::new();
    fixture.write_package("extra.lua");
    fixture.write_config(r#"return { packages = { "extra.lua", "extra.lua" } }"#);

    let host = start(fixture.environment());
    let payload = report(&dispatch(&host, &fixture, json!({ "kind": "report" }))).clone();
    assert_eq!(payload["loaded"], false);
    let message = rows(&payload["errors"])[0]
        .as_str()
        .expect("error")
        .to_owned();
    assert!(message.contains("duplicate entry"), "{message}");
    assert!(package_sources(&payload).is_empty());
}

// ---------------------------------------------------------------------------
// Inspectability
// ---------------------------------------------------------------------------

#[test]
fn every_effective_key_reports_the_layer_and_file_that_produced_it() {
    let fixture = Fixture::new();
    fixture.write_config(
        r#"return {
  theme = "user",
  model = { provider = "anthropic", id = "claude-sonnet-4-5" },
  keymaps = { ["ctrl+k"] = "clear" },
}"#,
    );
    fixture.write(
        &fixture.project_config(),
        r#"return { theme = "project", tools = { root = "/workspace" } }"#,
    );

    let host = start(fixture.environment());
    dispatch(
        &host,
        &fixture,
        json!({ "kind": "trust", "directory": text(&fixture.project()), "decision": "trust" }),
    );
    let payload = report(&dispatch(&host, &fixture, json!({ "kind": "reload" }))).clone();

    let leaves: Vec<String> = rows(&payload["leaves"])
        .into_iter()
        .map(|value| value.as_str().unwrap_or_default().to_owned())
        .collect();
    let provenance = payload["provenance"]
        .as_object()
        .expect("provenance object");

    // Fail closed in both directions: a leaf without an origin, or an origin
    // without a leaf, is a gap in the inspection surface.
    for leaf in &leaves {
        assert!(provenance.contains_key(leaf), "no provenance for {leaf}");
    }
    for key in provenance.keys() {
        assert!(leaves.contains(key), "provenance {key} has no value");
    }
    assert!(leaves.contains(&"theme".to_owned()));
    assert!(leaves.contains(&"model.provider".to_owned()));

    // Each layer is visible where it won.
    assert_eq!(provenance["theme"]["layer"], "project");
    assert_eq!(
        provenance["theme"]["source"],
        text(&fixture.project_config())
    );
    assert_eq!(provenance["model.id"]["layer"], "user");
    assert_eq!(provenance["keymaps.ctrl+k"]["layer"], "user");
    assert_eq!(provenance["tools.root"]["layer"], "project");
    assert_eq!(provenance["tools.suppress"]["layer"], "defaults");
    assert_eq!(provenance["providers"]["source"], "<builtins>");

    // Every considered source is reported, including the ones that lost.
    let layers: Vec<String> = rows(&payload["sources"])
        .into_iter()
        .map(|row| row["layer"].as_str().unwrap_or_default().to_owned())
        .collect();
    assert_eq!(layers, vec!["defaults", "user", "project"]);
}

// ---------------------------------------------------------------------------
// Resource paths
// ---------------------------------------------------------------------------

#[test]
fn the_resource_matrix_is_xdg_first_with_a_per_resource_legacy_fallback() {
    let fixture = Fixture::new();
    // Present canonically: config. Present only legacy: sessions. Absent
    // everywhere: credentials.
    fixture.write_config(r#"return { theme = "user" }"#);
    std::fs::create_dir_all(fixture.home().join(".pi/agent/sessions")).expect("legacy sessions");

    let host = start(fixture.environment());
    let payload = report(&dispatch(&host, &fixture, json!({ "kind": "report" }))).clone();
    let resources = &payload["resources"];

    assert_eq!(resources["config"]["source"], "canonical");
    assert_eq!(resources["sessions"]["source"], "legacy");
    assert_eq!(
        resources["sessions"]["selected"],
        text(&fixture.home().join(".pi/agent/sessions"))
    );
    assert_eq!(resources["credentials"]["source"], "absent");
    assert_eq!(resources["credentials"]["selected"], Value::Null);

    // Every destination is canonical, whatever supplied the value.
    assert_eq!(
        resources["sessions"]["destination"],
        text(&fixture.state_root().join("pi/sessions"))
    );
    assert_eq!(
        resources["credentials"]["destination"],
        text(&fixture.state_root().join("pi/credentials.json"))
    );
    assert_eq!(
        resources["packages"]["destination"],
        text(&fixture.data_root().join("pi/packages"))
    );
    assert_eq!(
        resources["cache"]["destination"],
        text(&fixture.cache_root().join("pi"))
    );
    // Trust is a pi-rs concept, so it has no inherited counterpart at all.
    assert_eq!(resources["trust"]["legacy"], Value::Null);

    assert_eq!(
        payload["roots"]["config"],
        text(&fixture.config_root().join("pi"))
    );
    assert_eq!(
        payload["roots"]["legacy"],
        text(&fixture.home().join(".pi/agent"))
    );
}

#[test]
fn absent_xdg_variables_use_the_home_defaults_and_a_relative_one_is_refused() {
    let fixture = Fixture::new();
    let home_config = fixture.home().join(".config/pi/config.lua");
    fixture.write(&home_config, r#"return { theme = "home" }"#);

    // Only HOME is set, plus one deliberately invalid relative override.
    let mut environment = BTreeMap::new();
    environment.insert("HOME".to_owned(), text(&fixture.home()));
    environment.insert("XDG_DATA_HOME".to_owned(), "relative/data".to_owned());
    environment.insert("XDG_STATE_HOME".to_owned(), String::new());

    let host = start(environment);
    let payload = report(&dispatch(&host, &fixture, json!({ "kind": "report" }))).clone();

    assert_eq!(payload["effective"]["theme"], "home");
    assert_eq!(
        payload["roots"]["config"],
        text(&fixture.home().join(".config/pi"))
    );
    // A relative variable is ignored, not accepted and not resolved against
    // the working directory.
    assert_eq!(
        payload["roots"]["data"],
        text(&fixture.home().join(".local/share/pi"))
    );
    // An empty variable has the XDG meaning of "unset".
    assert_eq!(
        payload["roots"]["state"],
        text(&fixture.home().join(".local/state/pi"))
    );
}

// ---------------------------------------------------------------------------
// Policy replacement
// ---------------------------------------------------------------------------

#[test]
fn replacing_the_configuration_file_changes_the_published_model() {
    let fixture = Fixture::new();
    fixture
        .write_config(r#"return { model = { provider = "anthropic", id = "claude-sonnet-4-5" } }"#);

    let host = start(fixture.environment());
    let first = report(&dispatch(&host, &fixture, json!({ "kind": "startup" }))).clone();
    assert_eq!(first["event_model"], "claude-sonnet-4-5");
    assert_eq!(first["event_config"]["model"]["provider"], "anthropic");
    assert_eq!(first["event_config_revision"], 1);

    // The same journey with a different file selects a different model, and
    // the reload event is the one that republishes it.
    dispatch(
        &host,
        &fixture,
        json!({
            "kind": "write",
            "path": text(&fixture.config_file()),
            "contents": r#"return { model = { provider = "openai", id = "gpt-5.1" } }"#,
        }),
    );
    let second = report(&dispatch(
        &host,
        &fixture,
        json!({ "kind": "config_reload" }),
    ))
    .clone();
    assert_eq!(second["event_model"], "gpt-5.1");
    assert_eq!(second["event_config_revision"], 2);

    // A model no catalog offers is a diagnostic-free no-op: the event keeps no
    // model and a later package may still choose one.
    dispatch(
        &host,
        &fixture,
        json!({
            "kind": "write",
            "path": text(&fixture.config_file()),
            "contents": r#"return { model = { provider = "nowhere", id = "imaginary" } }"#,
        }),
    );
    let third = report(&dispatch(
        &host,
        &fixture,
        json!({ "kind": "config_reload" }),
    ))
    .clone();
    assert_eq!(third["event_model"], Value::Null);
    assert_eq!(third["effective"]["model"]["provider"], "nowhere");
}

#[test]
fn a_configuration_file_receives_no_host_capability() {
    let fixture = Fixture::new();
    fixture.write_config(
        r#"local context = ...
return { theme = string.format("%s:%s", context.layer, context.paths.config ~= nil) }"#,
    );

    let host = start(fixture.environment());
    let payload = report(&dispatch(&host, &fixture, json!({ "kind": "report" }))).clone();
    // Pure libraries and the passed context work.
    assert_eq!(payload["effective"]["theme"], "user:true");

    // Reaching for host capability fails as an ordinary Lua error, and the
    // layer is refused rather than partially applied.
    for probe in [
        r#"return { theme = pi.kernel.v1.api_version and "x" or "y" }"#,
        r#"return { theme = io.open("/etc/passwd") and "x" or "y" }"#,
        r#"return { theme = tostring(os.getenv("HOME")) }"#,
        r#"local chunk = load("return 1") return { theme = "x" }"#,
    ] {
        dispatch(
            &host,
            &fixture,
            json!({ "kind": "write", "path": text(&fixture.config_file()), "contents": probe }),
        );
        let batch = dispatch(&host, &fixture, json!({ "kind": "reload" }));
        let result = action(&batch, "reload_result");
        assert_eq!(result["ok"], false, "{probe}");
        let message = rows(&result["errors"])
            .first()
            .and_then(|value| value.as_str().map(str::to_owned))
            .unwrap_or_default();
        assert!(message.contains("nil value"), "{probe}: {message}");
    }
}

// ---------------------------------------------------------------------------
// Applying the sections
// ---------------------------------------------------------------------------

#[test]
fn theme_keymaps_and_providers_become_declarations_the_product_reads() {
    let fixture = Fixture::new();
    fixture.write_config(
        r#"
return {
  theme = "solar",
  keymaps = { ["ctrl+k"] = "clear", ["ctrl+j"] = "newline" },
  providers = {
    openai = { base_url = "http://127.0.0.1:9/v1", models = { "gpt-5.1" } },
  },
}
"#,
    );

    let host = start(fixture.environment());
    let payload = report(&dispatch(&host, &fixture, json!({ "kind": "report" }))).clone();

    // One theme declaration, in the configuration's own id namespace so a
    // configured theme never silently collides with a package's, carrying the
    // layer and file that produced it.
    let themes = rows(&payload["themes"]);
    assert_eq!(themes.len(), 1);
    assert_eq!(themes[0]["declaration_id"], "pi.config.theme");
    assert_eq!(themes[0]["name"], "solar");
    assert_eq!(themes[0]["layer"], "user");
    assert_eq!(themes[0]["origin"], text(&fixture.config_file()));

    // One keymap declaration per binding, in sorted binding order, so the
    // declaration order is a property of the file's content rather than of
    // Lua's table iteration order.
    let keymaps = rows(&payload["keymaps"]);
    assert_eq!(keymaps.len(), 2);
    assert_eq!(keymaps[0]["declaration_id"], "pi.config.keymap:ctrl+j");
    assert_eq!(keymaps[0]["binding"], "ctrl+j");
    assert_eq!(keymaps[0]["action"], "newline");
    assert_eq!(keymaps[1]["declaration_id"], "pi.config.keymap:ctrl+k");
    assert_eq!(keymaps[1]["action"], "clear");

    // A configured endpoint rides on the reviewed catalog row, so nothing here
    // invents a cost, a context window, or a token budget.
    let providers = rows(&payload["providers"]);
    assert_eq!(providers.len(), 1);
    assert_eq!(providers[0]["declaration_id"], "pi.config.provider:openai");
    assert_eq!(providers[0]["base_url"], "http://127.0.0.1:9/v1");
    let models = rows(&providers[0]["models"]);
    assert_eq!(models.len(), 1);
    assert_eq!(models[0]["id"], "gpt-5.1");
    assert_eq!(models[0]["baseUrl"], "http://127.0.0.1:9/v1");
    assert!(models[0]["contextWindow"].as_u64().unwrap_or_default() > 0);

    // The configuration's own account of what it applied agrees with the
    // kernel's, and the declarations arrive in one package the configuration
    // owns rather than from the configuration package's permanent scope.
    let planned = rows(&payload["declarations"]);
    assert_eq!(planned.len(), 4);
    assert_eq!(planned[0]["kind"], "theme");
    assert_eq!(planned[3]["kind"], "provider");
    assert_eq!(
        live_packages(&payload),
        vec![DECLARATION_PACKAGE.to_owned()]
    );
}

#[test]
fn changing_the_configuration_replaces_the_declarations_it_produced() {
    let fixture = Fixture::new();
    fixture.write_config(
        r#"return { theme = "solar", keymaps = { ["ctrl+k"] = "clear", ["ctrl+j"] = "newline" } }"#,
    );

    let host = start(fixture.environment());
    let first = report(&dispatch(&host, &fixture, json!({ "kind": "report" }))).clone();
    assert_eq!(rows(&first["themes"])[0]["name"], "solar");
    assert_eq!(rows(&first["keymaps"]).len(), 2);

    dispatch(
        &host,
        &fixture,
        json!({
            "kind": "write",
            "path": text(&fixture.config_file()),
            "contents": r#"return { theme = "mono", keymaps = { ["ctrl+k"] = "submit" } }"#,
        }),
    );
    let batch = dispatch(&host, &fixture, json!({ "kind": "reload" }));
    assert_eq!(action(&batch, "reload_result")["ok"], true);
    let second = report(&batch);

    // The previous declarations are gone rather than shadowed: the same ids
    // are declared again, which the kernel would refuse if the package that
    // made them were still alive.
    let themes = rows(&second["themes"]);
    assert_eq!(themes.len(), 1);
    assert_eq!(themes[0]["declaration_id"], "pi.config.theme");
    assert_eq!(themes[0]["name"], "mono");
    let keymaps = rows(&second["keymaps"]);
    assert_eq!(keymaps.len(), 1);
    assert_eq!(keymaps[0]["declaration_id"], "pi.config.keymap:ctrl+k");
    assert_eq!(keymaps[0]["action"], "submit");
    // One declaration package, not one per revision.
    assert_eq!(live_packages(second), vec![DECLARATION_PACKAGE.to_owned()]);

    // Removing every applied section retracts the declarations with it.
    dispatch(
        &host,
        &fixture,
        json!({
            "kind": "write",
            "path": text(&fixture.config_file()),
            "contents": "return {}",
        }),
    );
    let cleared = report(&dispatch(&host, &fixture, json!({ "kind": "reload" }))).clone();
    assert!(rows(&cleared["themes"]).is_empty());
    assert!(rows(&cleared["keymaps"]).is_empty());
    assert!(live_packages(&cleared).is_empty());
}

#[test]
fn a_provider_naming_an_unknown_model_fails_the_reload_and_keeps_the_declarations() {
    let fixture = Fixture::new();
    fixture.write_config(
        r#"return { theme = "solar", providers = { openai = { models = { "gpt-5.1" } } } }"#,
    );

    let host = start(fixture.environment());
    let first = report(&dispatch(&host, &fixture, json!({ "kind": "report" }))).clone();
    assert_eq!(first["revision"], 1);
    assert_eq!(rows(&first["providers"]).len(), 1);

    // The plan is built during composition, before anything is published, so a
    // model the reviewed catalog does not carry fails the whole reload instead
    // of publishing settings the product cannot act on.
    dispatch(
        &host,
        &fixture,
        json!({
            "kind": "write",
            "path": text(&fixture.config_file()),
            "contents": r#"return { theme = "mono", providers = { openai = { models = { "no-such-model" } } } }"#,
        }),
    );
    let batch = dispatch(&host, &fixture, json!({ "kind": "reload" }));
    let result = action(&batch, "reload_result");
    assert_eq!(result["ok"], false);
    assert_eq!(result["revision"], 1);
    let message = rows(&result["errors"])
        .first()
        .and_then(|value| value.as_str().map(str::to_owned))
        .unwrap_or_default();
    assert!(message.contains("providers.openai.models[1]"), "{message}");

    let payload = report(&batch);
    assert_eq!(payload["revision"], 1);
    assert_eq!(payload["effective"]["theme"], "solar");
    assert_eq!(rows(&payload["themes"])[0]["name"], "solar");
    assert_eq!(rows(&payload["providers"]).len(), 1);
}

#[test]
fn the_configuration_pins_the_module_identities_it_names() {
    let fixture = Fixture::new();
    let selected = fixture.write_package("extra.lua");
    fixture.write_config(
        r#"return {
  packages = { "extra.lua" },
  modules = { { name = "pi.test.extra", version = "1" } },
}"#,
    );

    let host = start(fixture.environment());
    let first = report(&dispatch(&host, &fixture, json!({ "kind": "report" }))).clone();
    assert_eq!(first["revision"], 1);
    assert_eq!(rows(&first["modules"]), vec![json!("pi.test.extra@1")]);
    assert_eq!(package_sources(&first), vec![text(&selected)]);

    // Pinning a version no selected package provides is a configuration error,
    // not a missing dependency discovered later by whoever needed it.
    dispatch(
        &host,
        &fixture,
        json!({
            "kind": "write",
            "path": text(&fixture.config_file()),
            "contents": r#"return {
  packages = { "extra.lua" },
  modules = { { name = "pi.test.extra", version = "2" } },
}"#,
        }),
    );
    let batch = dispatch(&host, &fixture, json!({ "kind": "reload" }));
    let result = action(&batch, "reload_result");
    assert_eq!(result["ok"], false);
    assert_eq!(result["revision"], 1);
    let message = rows(&result["errors"])
        .first()
        .and_then(|value| value.as_str().map(str::to_owned))
        .unwrap_or_default();
    assert!(message.starts_with("modules[1]: "), "{message}");

    let payload = report(&batch);
    assert_eq!(payload["revision"], 1);
    assert_eq!(rows(&payload["modules"]), vec![json!("pi.test.extra@1")]);
    assert_eq!(package_sources(payload), vec![text(&selected)]);
}

// ---------------------------------------------------------------------------
// Zero configuration
// ---------------------------------------------------------------------------

#[test]
fn without_any_configuration_file_only_the_shipped_defaults_are_published() {
    let fixture = Fixture::new();

    let host = start(fixture.environment());
    let payload = report(&dispatch(&host, &fixture, json!({ "kind": "report" }))).clone();

    assert_eq!(payload["loaded"], true);
    assert_eq!(payload["revision"], 1);
    // The shipped layer carries shape, not visible policy: no theme, no model,
    // no keymap, so suppressing this package removes configurability rather
    // than the product's appearance.
    assert_eq!(payload["effective"]["theme"], Value::Null);
    assert_eq!(payload["effective"]["model"], Value::Null);
    for (_, origin) in payload["provenance"]
        .as_object()
        .expect("provenance")
        .iter()
    {
        assert_eq!(origin["layer"], "defaults");
        assert_eq!(origin["source"], "<builtins>");
    }
    assert_eq!(source_row(&payload, "user")["outcome"], "absent");
    assert!(package_sources(&payload).is_empty());
    // Nothing to apply, so nothing is declared and no declaration package is
    // loaded at all.
    assert!(live_packages(&payload).is_empty());
    assert!(rows(&payload["themes"]).is_empty());
    assert!(rows(&payload["keymaps"]).is_empty());
    assert!(rows(&payload["providers"]).is_empty());
    assert!(rows(&payload["declarations"]).is_empty());

    // Reading a configuration writes nothing: no file is created under any
    // root, and no trust store exists without a decision.
    assert!(!fixture.config_file().exists());
    assert!(!fixture.trust_store().exists());
    assert!(!fixture.state_root().join("pi").exists());
}

#[test]
fn the_host_supplies_no_configuration_module_of_its_own() {
    let fixture = Fixture::new();
    let host = Host::new(HostConfig {
        environment: Some(fixture.environment()),
        ..HostConfig::default()
    })
    .expect("host starts");
    let directory = tempfile::tempdir().expect("probe directory");
    let probe = directory.path().join("probe.lua");
    std::fs::write(
        &probe,
        r#"
local pi = ...
local roots = pi.roots.v1
roots.register({
  kind = "application",
  id = "probe",
  dispatch = function()
    local ok = pcall(pi.kernel.v1.module.require, "pi.config.settings", "1")
    roots.action("probe", {
      settings_module = ok,
      config_member = pi.config ~= nil,
      settings_member = pi.settings ~= nil,
    })
  end,
})
"#,
    )
    .expect("probe package");
    host.load_package(PackageSource::File { path: &probe })
        .expect("probe loads");

    let batch = host
        .dispatch(DispatchRequest::new(
            RootKind::Application,
            json!({ "kind": "report" }),
            json!({}),
        ))
        .expect("dispatch succeeds");
    let payload = action(&batch, "probe");
    assert_eq!(payload["settings_module"], false);
    assert_eq!(payload["config_member"], false);
    assert_eq!(payload["settings_member"], false);
}
