//! Environment, path, and filesystem-metadata effects from a file-backed
//! package.
//!
//! The XDG-first/legacy-fallback matrix below is entirely Lua policy: the host
//! contributes an immutable environment snapshot, pure path arithmetic, and
//! bounded filesystem mechanism. No directory name, precedence rule, or
//! fallback lives in Rust.

#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

use pi_rs_host::kernel::{DispatchBatch, DispatchRequest, RootKind};
use pi_rs_host::{Host, HostConfig, PackageSource};

const RESOLVER: &str = r#"
local pi = ...
local effects = pi.effects.v1
local env, path, fs = effects.env, effects.path, effects.fs
local roots = pi.roots.v1

local DEFAULTS = { theme = "plain", model = "none" }

local function parse(text)
  local settings = {}
  for line in string.gmatch(text, "[^\n]+") do
    local key, value = string.match(line, "^(%w+)%s*=%s*(.+)$")
    if key then settings[key] = value end
  end
  return settings
end

-- Product policy: XDG location first (explicit variable, else the conventional
-- default under HOME), then the inherited legacy directory as read-only
-- fallback, then Lua constants.
local function candidates()
  local list = {}
  local xdg = env.get("XDG_CONFIG_HOME")
  local home = env.get("HOME")
  if xdg and xdg ~= "" then
    list[#list + 1] = { source = "xdg", file = path.join(xdg, "pi-rs", "config.conf") }
  elseif home then
    list[#list + 1] = {
      source = "xdg-default",
      file = path.join(home, ".config", "pi-rs", "config.conf"),
    }
  end
  if home then
    list[#list + 1] = { source = "legacy", file = path.join(home, ".pi", "config.conf") }
  end
  return list
end

roots.register({
  kind = "application",
  id = "configuration-resolver",
  dispatch = function()
    local resolved = { source = "defaults", theme = DEFAULTS.theme, model = DEFAULTS.model }
    local probed = {}
    for _, candidate in ipairs(candidates()) do
      probed[#probed + 1] = candidate.source
      if fs.exists(candidate.file) then
        local settings = parse(fs.read(candidate.file))
        resolved = {
          source = candidate.source,
          file = candidate.file,
          theme = settings.theme or DEFAULTS.theme,
          model = settings.model or DEFAULTS.model,
        }
        break
      end
    end

    -- Writes target a state location the package computed itself.
    local state_home = env.get("XDG_STATE_HOME")
      or path.join(env.get("HOME") or "/tmp", ".local", "state")
    local state_directory = path.join(state_home, "pi-rs")
    fs.make_directory(state_directory)
    local marker = path.join(state_directory, "resolved.conf")
    fs.write(marker, "source = " .. resolved.source .. "\n")
    local entries = fs.list(state_directory)
    local info = fs.stat(marker)
    local reread = parse(fs.read(marker))
    fs.remove_file(marker)
    local remaining = fs.list(state_directory)
    local bounded_ok, bounded_error = pcall(function() return fs.list(state_directory, 0) end)
    local missing_ok, missing_error = pcall(function() return fs.stat(marker) end)

    local names = env.names()
    local seen_home = false
    for _, name in ipairs(names) do
      if name == "HOME" then seen_home = true end
    end

    roots.action("configured", {
      source = resolved.source,
      file = resolved.file,
      theme = resolved.theme,
      model = resolved.model,
      probed = probed,
      relative = resolved.file and path.relative(state_home, resolved.file) or "",
      separator = path.separator,
      absolute = path.is_absolute(state_directory),
      state_directory = state_directory,
      state_type = info.type,
      marker_source = reread.source,
      entries = #entries,
      first_entry = entries[1],
      remaining = #remaining,
      default_max_entries = fs.default_max_entries,
      max_entries = fs.max_entries,
      bounded_ok = bounded_ok,
      bounded_error = tostring(bounded_error),
      missing_ok = missing_ok,
      env_names = #names,
      seen_home = seen_home,
      absent = env.get("PI_RS_ABSENT_VARIABLE") == nil,
      writable = env.set == nil and env.all == nil,
    })
  end,
})
"#;

const PROCESS_ENVIRONMENT: &str = r#"
local pi = ...
local env = pi.effects.v1.env
local roots = pi.roots.v1

roots.register({
  kind = "application",
  id = "process-environment",
  dispatch = function()
    local names = env.names()
    local seen = false
    for _, name in ipairs(names) do
      if name == "PATH" then seen = true end
    end
    roots.action("inherited", { path = env.get("PATH") or "", seen = seen })
  end,
})
"#;

struct Fixture {
    _root: tempfile::TempDir,
    home: std::path::PathBuf,
    xdg_config: std::path::PathBuf,
    xdg_state: std::path::PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let root = tempfile::tempdir().expect("temporary root");
        let home = root.path().join("home");
        let xdg_config = root.path().join("xdg/config");
        let xdg_state = root.path().join("xdg/state");
        std::fs::create_dir_all(&home).expect("home directory");
        Self {
            _root: root,
            home,
            xdg_config,
            xdg_state,
        }
    }

    fn write(&self, path: &std::path::Path, contents: &str) {
        std::fs::create_dir_all(path.parent().expect("parent directory")).expect("config parent");
        std::fs::write(path, contents).expect("config file");
    }

    fn xdg_file(&self) -> std::path::PathBuf {
        self.xdg_config.join("pi-rs/config.conf")
    }

    fn home_file(&self) -> std::path::PathBuf {
        self.home.join(".config/pi-rs/config.conf")
    }

    fn legacy_file(&self) -> std::path::PathBuf {
        self.home.join(".pi/config.conf")
    }

    fn environment(&self, xdg_config: bool) -> std::collections::BTreeMap<String, String> {
        let mut environment = std::collections::BTreeMap::new();
        environment.insert("HOME".to_owned(), self.home.to_string_lossy().into_owned());
        environment.insert(
            "XDG_STATE_HOME".to_owned(),
            self.xdg_state.to_string_lossy().into_owned(),
        );
        if xdg_config {
            environment.insert(
                "XDG_CONFIG_HOME".to_owned(),
                self.xdg_config.to_string_lossy().into_owned(),
            );
        }
        environment
    }
}

fn load(host: &Host, name: &str, source: &str) {
    let directory = tempfile::tempdir().expect("temporary package directory");
    let path = directory.path().join(format!("{name}.lua"));
    std::fs::write(&path, source).expect("write file-backed package");
    host.load_package(PackageSource::File { path: &path })
        .expect("file-backed package loads");
    std::mem::forget(directory);
}

fn resolve(environment: std::collections::BTreeMap<String, String>) -> DispatchBatch {
    let host = Host::new(HostConfig {
        environment: Some(environment),
        ..HostConfig::default()
    })
    .expect("host starts");
    load(&host, "configuration-resolver", RESOLVER);
    host.dispatch(DispatchRequest::new(
        RootKind::Application,
        serde_json::json!({ "kind": "resolve" }),
        serde_json::json!({}),
    ))
    .expect("dispatch succeeds")
}

#[test]
fn explicit_xdg_configuration_wins_over_the_legacy_location() {
    let fixture = Fixture::new();
    fixture.write(&fixture.xdg_file(), "theme = solar\nmodel = alpha\n");
    fixture.write(&fixture.legacy_file(), "theme = inherited\n");

    let batch = resolve(fixture.environment(true));
    assert_eq!(batch.actions.len(), 1);
    let payload = &batch.actions[0].payload;

    assert_eq!(payload["source"], "xdg");
    assert_eq!(payload["theme"], "solar");
    assert_eq!(payload["model"], "alpha");
    assert_eq!(payload["probed"], serde_json::json!(["xdg"]));
    assert_eq!(
        payload["file"],
        fixture.xdg_file().to_string_lossy().into_owned()
    );

    // Metadata mechanism: the package created its own state directory, wrote,
    // listed, described, re-read, and removed a file inside it.
    assert_eq!(payload["state_type"], "file");
    assert_eq!(payload["marker_source"], "xdg");
    assert_eq!(payload["entries"], 1);
    assert_eq!(payload["first_entry"], "resolved.conf");
    assert_eq!(payload["remaining"], 0);
    assert_eq!(payload["absolute"], true);
    assert_eq!(payload["separator"], "/");
    assert_eq!(
        payload["state_directory"],
        fixture
            .xdg_state
            .join("pi-rs")
            .to_string_lossy()
            .into_owned()
    );

    // Bounds and honest failures.
    assert_eq!(payload["default_max_entries"], 1024);
    assert_eq!(payload["max_entries"], 16384);
    assert_eq!(payload["bounded_ok"], false);
    assert!(
        payload["bounded_error"]
            .as_str()
            .expect("listing bound error text")
            .contains("16384"),
        "expected the listing bound in {}",
        payload["bounded_error"]
    );
    assert_eq!(payload["missing_ok"], false);

    // The environment is a read-by-name snapshot with no bulk value dump.
    assert_eq!(payload["env_names"], 3);
    assert_eq!(payload["seen_home"], true);
    assert_eq!(payload["absent"], true);
    assert_eq!(payload["writable"], true);
}

#[test]
fn unset_xdg_variable_falls_back_to_the_conventional_home_location() {
    let fixture = Fixture::new();
    fixture.write(&fixture.home_file(), "theme = home\n");
    fixture.write(&fixture.legacy_file(), "theme = inherited\n");

    let payload = &resolve(fixture.environment(false)).actions[0].payload;
    assert_eq!(payload["source"], "xdg-default");
    assert_eq!(payload["theme"], "home");
    assert_eq!(payload["model"], "none");
    assert_eq!(payload["probed"], serde_json::json!(["xdg-default"]));
}

#[test]
fn legacy_configuration_is_used_only_as_an_untouched_fallback() {
    let fixture = Fixture::new();
    fixture.write(&fixture.legacy_file(), "theme = inherited\nmodel = old\n");

    let payload = &resolve(fixture.environment(true)).actions[0].payload;
    assert_eq!(payload["source"], "legacy");
    assert_eq!(payload["theme"], "inherited");
    assert_eq!(payload["model"], "old");
    assert_eq!(payload["probed"], serde_json::json!(["xdg", "legacy"]));

    assert_eq!(
        std::fs::read_to_string(fixture.legacy_file()).expect("legacy configuration"),
        "theme = inherited\nmodel = old\n"
    );
    assert!(
        !fixture.xdg_file().exists(),
        "probing must not create the preferred location"
    );
}

#[test]
fn absent_configuration_leaves_lua_defaults_and_creates_nothing() {
    let fixture = Fixture::new();

    let payload = &resolve(fixture.environment(true)).actions[0].payload;
    assert_eq!(payload["source"], "defaults");
    assert_eq!(payload["theme"], "plain");
    assert_eq!(payload["model"], "none");
    assert_eq!(payload["file"], serde_json::Value::Null);
    assert_eq!(payload["relative"], "");

    assert!(!fixture.xdg_config.join("pi-rs").exists());
    assert!(!fixture.home.join(".config").exists());
}

#[test]
fn the_default_host_snapshots_the_process_environment_once() {
    let host = Host::new(HostConfig::default()).expect("host starts");
    load(&host, "process-environment", PROCESS_ENVIRONMENT);
    let batch = host
        .dispatch(DispatchRequest::new(
            RootKind::Application,
            serde_json::json!({ "kind": "inherit" }),
            serde_json::json!({}),
        ))
        .expect("dispatch succeeds");

    let payload = &batch.actions[0].payload;
    let expected = std::env::var("PATH").unwrap_or_default();
    assert_eq!(payload["path"], expected);
    assert_eq!(payload["seen"], !expected.is_empty());
}
