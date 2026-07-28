//! Durable record persistence from ordinary file-backed packages.
//!
//! Every destination below is chosen by Lua from the immutable startup
//! context, so the host contributes storage mechanism only: no resource path,
//! record schema, or session meaning.

#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

use pi_rs_host::kernel::{DispatchBatch, DispatchRequest, RootKind};
use pi_rs_host::{Host, HostConfig, PackageSource};

const JOURNEY: &str = r#"
local pi = ...
local records = pi.records.v1
local roots = pi.roots.v1

roots.register({
  kind = "application",
  id = "record-journey",
  dispatch = function(snapshot)
    local destination = snapshot.context.storage.destination
    local store = records.create({ directory = destination, name = "journey" })
    local path = store:path()
    for index = 1, 3 do
      store:append({ schema = "journey/v1", index = index, note = "record " .. index })
    end
    local count = store:record_count()

    local copied = store:copy({ directory = destination, name = "prefix", record_count = 2 })
    local cursor = copied:cursor()
    local window = cursor:next({ max_records = 8, max_bytes = 4096 })

    local bounded = records.create({
      directory = destination,
      name = "bounded",
      limits = { max_record_bytes = 64 },
    })
    local oversize_ok, oversize_error = pcall(function()
      bounded:append({ blob = string.rep("x", 512) })
    end)
    bounded:close()
    local closed_ok, closed_error = pcall(function() bounded:append({ late = true }) end)

    local locked = records.list({ directory = destination })
    store:close()
    copied:close()

    local listing = records.list({ directory = destination })
    local names = {}
    for _, info in ipairs(listing.stores) do names[#names + 1] = info.name end
    table.sort(names)

    roots.action("recorded", {
      api_version = records.api_version,
      format_version = records.format_version,
      extension = records.extension,
      default_window_records = records.default_limits.max_window_records,
      path = path,
      count = count,
      copied_count = window.next_sequence,
      window_records = #window.records,
      first_index = window.records[1].index,
      first_note = window.records[1].note,
      last_index = window.records[#window.records].index,
      window_done = window.done,
      start_sequence = window.start_sequence,
      names = names,
      diagnostics = #listing.diagnostics,
      locked_stores = #locked.stores,
      locked_diagnostics = #locked.diagnostics,
      locked_kind = locked.diagnostics[1].kind,
      listed_count = listing.stores[2].record_count,
      oversize_ok = oversize_ok,
      oversize_error = tostring(oversize_error),
      closed_ok = closed_ok,
      closed_error = tostring(closed_error),
    })
  end,
})
"#;

const RETAIN: &str = r#"
local pi = ...
local records = pi.records.v1
local roots = pi.roots.v1
local kept = {}

roots.register({
  kind = "application",
  id = "record-retain",
  dispatch = function(snapshot)
    local destination = snapshot.context.storage.destination
    kept[#kept + 1] = records.create({ directory = destination, name = "retained" })
    kept[#kept + 1] = records.create({ directory = destination, name = "transient" })
    kept[2]:close()
    roots.action("retained", {
      path = kept[1]:path(),
      open = not kept[1]:closed(),
      released = kept[2]:closed(),
    })
  end,
})
"#;

const REOPEN: &str = r#"
local pi = ...
local records = pi.records.v1
local roots = pi.roots.v1

roots.register({
  kind = "application",
  id = "record-reopen",
  dispatch = function(snapshot)
    local store = records.open({ path = snapshot.event.path })
    local cursor = store:cursor()
    local window = cursor:next()
    store:close()
    roots.action("reopened", {
      count = #window.records,
      done = window.done,
    })
  end,
})
"#;

struct Fixture {
    _directory: tempfile::TempDir,
    destination: std::path::PathBuf,
    legacy: std::path::PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let directory = tempfile::tempdir().expect("temporary root");
        let destination = directory.path().join("xdg-state/pi/records");
        let legacy = directory.path().join("legacy/.pi/agent/records");
        std::fs::create_dir_all(&legacy).expect("legacy directory");
        std::fs::write(legacy.join("inherited.jsonl"), "legacy bytes\n").expect("legacy record");
        Self {
            _directory: directory,
            destination,
            legacy,
        }
    }

    fn context(&self) -> serde_json::Value {
        serde_json::json!({
            "storage": {
                "destination": self.destination.to_string_lossy(),
                "legacy": self.legacy.to_string_lossy(),
            }
        })
    }
}

fn load(host: &Host, name: &str, source: &str) -> pi_rs_host::kernel::PackageHandle {
    let directory = tempfile::tempdir().expect("temporary package directory");
    let path = directory.path().join(format!("{name}.lua"));
    std::fs::write(&path, source).expect("write file-backed package");
    let handle = host
        .load_package(PackageSource::File { path: &path })
        .expect("file-backed package loads");
    std::mem::forget(directory);
    handle
}

fn dispatch(host: &Host, event: serde_json::Value, context: serde_json::Value) -> DispatchBatch {
    host.dispatch(DispatchRequest::new(RootKind::Application, event, context))
        .expect("dispatch succeeds")
}

#[test]
fn file_backed_package_persists_copies_and_iterates_arbitrary_records() {
    let fixture = Fixture::new();
    let host = Host::new(HostConfig::default()).expect("host starts");
    load(&host, "record-journey", JOURNEY);

    let batch = dispatch(
        &host,
        serde_json::json!({ "kind": "record" }),
        fixture.context(),
    );
    assert_eq!(batch.actions.len(), 1);
    let payload = &batch.actions[0].payload;

    assert_eq!(payload["api_version"], 1);
    assert_eq!(payload["format_version"], 1);
    assert_eq!(payload["extension"], "jsonl");
    assert_eq!(payload["default_window_records"], 256);
    assert_eq!(payload["count"], 3);
    assert_eq!(payload["copied_count"], 2);
    assert_eq!(payload["window_records"], 2);
    assert_eq!(payload["start_sequence"], 0);
    assert_eq!(payload["first_index"], 1);
    assert_eq!(payload["first_note"], "record 1");
    assert_eq!(payload["last_index"], 2);
    assert_eq!(payload["window_done"], true);
    assert_eq!(payload["diagnostics"], 0);
    assert_eq!(payload["listed_count"], 3);

    // Open stores hold exclusive locks and are listed as explicit
    // diagnostics rather than silently omitted.
    assert_eq!(payload["locked_stores"], 1);
    assert_eq!(payload["locked_diagnostics"], 2);
    assert_eq!(payload["locked_kind"], "locked");
    assert_eq!(
        payload["names"],
        serde_json::json!(["bounded", "journey", "prefix"])
    );

    // Limits and closure are enforced by the mechanism, not by Lua discipline.
    assert_eq!(payload["oversize_ok"], false);
    assert!(
        payload["oversize_error"]
            .as_str()
            .expect("oversize error text")
            .contains("64"),
        "expected the record limit in {}",
        payload["oversize_error"]
    );
    assert_eq!(payload["closed_ok"], false);
    assert!(
        payload["closed_error"]
            .as_str()
            .expect("closed error text")
            .contains("record store is closed"),
        "expected a closed-store diagnostic in {}",
        payload["closed_error"]
    );

    // Records land only where Lua asked, in the documented on-disk format.
    let journey = fixture.destination.join("journey.jsonl");
    assert_eq!(
        payload["path"].as_str().expect("store path"),
        journey.to_string_lossy()
    );
    let written = std::fs::read_to_string(&journey).expect("read store");
    assert!(written.starts_with("{\"format\":\"pi-rs-records\",\"version\":1}\n"));
    assert!(written.contains("\"schema\":\"journey/v1\""));
    assert_eq!(written.lines().count(), 4);

    // The read-only legacy resource is never written, merged, or migrated.
    assert_eq!(
        std::fs::read_to_string(fixture.legacy.join("inherited.jsonl")).expect("legacy record"),
        "legacy bytes\n"
    );
    let legacy_entries = std::fs::read_dir(&fixture.legacy)
        .expect("legacy directory")
        .count();
    assert_eq!(legacy_entries, 1);
}

#[test]
fn open_stores_are_scope_resources_released_by_package_disposal() {
    let fixture = Fixture::new();
    let host = Host::new(HostConfig::default()).expect("host starts");
    let handle = load(&host, "record-retain", RETAIN);

    let batch = dispatch(
        &host,
        serde_json::json!({ "kind": "retain" }),
        fixture.context(),
    );
    let payload = &batch.actions[0].payload;
    assert_eq!(payload["open"], true);
    assert_eq!(payload["released"], true);
    let retained = payload["path"].as_str().expect("retained path").to_owned();

    // The closed store released its lease; the retained one still holds one.
    let stats = host.scope_stats(&handle).expect("scope stats");
    assert_eq!(stats.resources, 1);

    // A second host cannot take the retained store's exclusive lock.
    let observer = Host::new(HostConfig::default()).expect("observer host starts");
    load(&observer, "record-reopen", REOPEN);
    let locked = observer.dispatch(DispatchRequest::new(
        RootKind::Application,
        serde_json::json!({ "kind": "reopen", "path": retained.clone() }),
        serde_json::json!({}),
    ));
    assert!(locked.is_err(), "a locked store must not open twice");

    // Disposal runs the store's resource disposer, so the lock is released
    // without waiting for Lua garbage collection.
    host.dispose_package(&handle).expect("package disposes");
    let reopened = dispatch(
        &observer,
        serde_json::json!({ "kind": "reopen", "path": retained }),
        serde_json::json!({}),
    );
    assert_eq!(reopened.actions[0].payload["count"], 0);
    assert_eq!(reopened.actions[0].payload["done"], true);
}
