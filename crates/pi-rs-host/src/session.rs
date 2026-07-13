//! Generic durable-record bindings installed as `pi.records`.
//!
//! This adapter exposes only opaque JSON append, bounded iteration, atomic
//! prefix copy, locking/list diagnostics, and cancellation. Destinations are
//! always supplied by Lua; no product session path or record schema exists here.

use std::cell::RefCell;
use std::rc::Rc;

use mlua::{AnyUserData, Lua, Table, UserData, UserDataMethods, Value};
use pi_rs_session::{CancellationToken, RecordCursor, RecordStore, StoreLimits};

use crate::convert::{json_to_lua, lua_to_json};

struct StoreHandle(Rc<RefCell<RecordStore>>);
struct CursorHandle(Rc<RefCell<RecordCursor>>);
#[derive(Clone)]
struct CancellationHandle(CancellationToken);

fn runtime_err(error: impl std::fmt::Display) -> mlua::Error {
    mlua::Error::runtime(error.to_string())
}

fn cancellation_from(options: Option<&Table>) -> mlua::Result<CancellationToken> {
    let Some(options) = options else {
        return Ok(CancellationToken::new());
    };
    let Some(userdata) = options.get::<Option<AnyUserData>>("cancel")? else {
        return Ok(CancellationToken::new());
    };
    Ok(userdata.borrow::<CancellationHandle>()?.0.clone())
}

fn limits_from(options: Option<&Table>) -> mlua::Result<StoreLimits> {
    let defaults = StoreLimits::default();
    let Some(options) = options else {
        return Ok(defaults);
    };
    Ok(StoreLimits {
        max_record_bytes: options
            .get::<Option<usize>>("maxRecordBytes")?
            .unwrap_or(defaults.max_record_bytes),
        max_window_records: options
            .get::<Option<usize>>("maxWindowRecords")?
            .unwrap_or(defaults.max_window_records),
        max_window_bytes: options
            .get::<Option<usize>>("maxWindowBytes")?
            .unwrap_or(defaults.max_window_bytes),
    })
}

impl UserData for CancellationHandle {
    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        methods.add_method("cancel", |_, this, ()| {
            this.0.cancel();
            Ok(())
        });
        methods.add_method("is_cancelled", |_, this, ()| Ok(this.0.is_cancelled()));
    }
}

impl UserData for StoreHandle {
    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        methods.add_method("path", |_, this, ()| {
            Ok(this.0.borrow().path().to_string_lossy().into_owned())
        });
        methods.add_method("record_count", |_, this, ()| {
            Ok(this.0.borrow().record_count())
        });
        methods.add_method(
            "append",
            |_, this, (value, options): (Value, Option<Table>)| {
                let value = lua_to_json(value)?;
                let cancellation = cancellation_from(options.as_ref())?;
                this.0
                    .borrow_mut()
                    .append(&value, &cancellation)
                    .map_err(runtime_err)
            },
        );
        methods.add_method("cursor", |_, this, ()| {
            let cursor = this.0.borrow().cursor().map_err(runtime_err)?;
            Ok(CursorHandle(Rc::new(RefCell::new(cursor))))
        });
        methods.add_method("copy", |_, this, options: Table| {
            let directory: String = options.get("directory")?;
            let name: String = options.get("name")?;
            let record_count = options.get::<Option<u64>>("recordCount")?;
            let cancellation = cancellation_from(Some(&options))?;
            let copied = this
                .0
                .borrow()
                .copy_prefix(directory, &name, record_count, &cancellation)
                .map_err(runtime_err)?;
            Ok(StoreHandle(Rc::new(RefCell::new(copied))))
        });
    }
}

impl UserData for CursorHandle {
    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        methods.add_method("next_sequence", |_, this, ()| {
            Ok(this.0.borrow().next_sequence())
        });
        methods.add_method("next", |lua, this, options: Option<Table>| {
            let defaults = StoreLimits::default();
            let max_records =
                options
                    .as_ref()
                    .map_or(Ok(defaults.max_window_records), |table| {
                        table
                            .get::<Option<usize>>("maxRecords")
                            .map(|value| value.unwrap_or(defaults.max_window_records))
                    })?;
            let max_bytes = options
                .as_ref()
                .map_or(Ok(defaults.max_window_bytes), |table| {
                    table
                        .get::<Option<usize>>("maxBytes")
                        .map(|value| value.unwrap_or(defaults.max_window_bytes))
                })?;
            let cancellation = cancellation_from(options.as_ref())?;
            let window = this
                .0
                .borrow_mut()
                .next_window(max_records, max_bytes, &cancellation)
                .map_err(runtime_err)?;
            let result = lua.create_table()?;
            result.set(
                "records",
                json_to_lua(lua, &serde_json::Value::Array(window.records))?,
            )?;
            result.set("startSequence", window.start_sequence)?;
            result.set("nextSequence", window.next_sequence)?;
            result.set("encodedBytes", window.encoded_bytes)?;
            result.set("done", window.done)?;
            Ok(result)
        });
    }
}

pub(crate) fn install(lua: &Lua, pi: &Table, _cwd: &str) -> mlua::Result<()> {
    let records = lua.create_table()?;
    records.set(
        "cancellation",
        lua.create_function(|_, ()| Ok(CancellationHandle(CancellationToken::new())))?,
    )?;
    records.set(
        "create",
        lua.create_function(|_, options: Table| {
            let directory: String = options.get("directory")?;
            let name: String = options.get("name")?;
            let limits = limits_from(Some(&options))?;
            let cancellation = cancellation_from(Some(&options))?;
            let store = RecordStore::create(directory, &name, limits, &cancellation)
                .map_err(runtime_err)?;
            Ok(StoreHandle(Rc::new(RefCell::new(store))))
        })?,
    )?;
    records.set(
        "open",
        lua.create_function(|_, options: Table| {
            let path: String = options.get("path")?;
            let limits = limits_from(Some(&options))?;
            let cancellation = cancellation_from(Some(&options))?;
            let store = RecordStore::open(path, limits, &cancellation).map_err(runtime_err)?;
            Ok(StoreHandle(Rc::new(RefCell::new(store))))
        })?,
    )?;
    records.set(
        "list",
        lua.create_function(|lua, options: Table| {
            let directory: String = options.get("directory")?;
            let limits = limits_from(Some(&options))?;
            let cancellation = cancellation_from(Some(&options))?;
            let listing =
                RecordStore::list(directory, limits, &cancellation).map_err(runtime_err)?;
            let result = lua.create_table()?;
            let stores = lua.create_table()?;
            for (index, info) in listing.stores.into_iter().enumerate() {
                let row = lua.create_table()?;
                row.set("name", info.name)?;
                row.set("path", info.path.to_string_lossy().into_owned())?;
                row.set("formatVersion", info.format_version)?;
                row.set("recordCount", info.record_count)?;
                row.set("bytes", info.bytes)?;
                stores.set(index + 1, row)?;
            }
            let diagnostics = lua.create_table()?;
            for (index, diagnostic) in listing.diagnostics.into_iter().enumerate() {
                let row = lua.create_table()?;
                row.set("path", diagnostic.path.to_string_lossy().into_owned())?;
                row.set("kind", diagnostic.kind)?;
                row.set("message", diagnostic.message)?;
                diagnostics.set(index + 1, row)?;
            }
            result.set("stores", stores)?;
            result.set("diagnostics", diagnostics)?;
            Ok(result)
        })?,
    )?;
    pi.set("records", records)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::unwrap_used)]

    use crate::{Host, HostConfig};

    #[test]
    fn ordinary_file_backed_package_exercises_the_public_record_store() {
        let temporary = tempfile::tempdir().expect("temporary directory");
        let package = temporary.path().join("record-package.lua");
        let destination = temporary.path().join("xdg-state/pi/records");
        let destination_lua =
            serde_json::to_string(&destination.to_string_lossy()).expect("encode destination path");
        let source = format!(
            r#"
local pi = ...
local directory = {destination_lua}
local store = pi.records.create({{ directory=directory, name="source" }})
local sequence = store:append({{ schema="different", payload={{1, true, "x"}} }})
local copied = store:copy({{ directory=directory, name="copied", recordCount=1 }})
local cursor = copied:cursor()
local window = cursor:next({{ maxRecords=4, maxBytes=4096 }})
local cancellation = pi.records.cancellation()
cancellation:cancel()
local cancelled = not pcall(function()
  store:append({{ unreachable=true }}, {{ cancel=cancellation }})
end)
pi.on("record_store_probe", function()
  return {{
    sequence=sequence,
    copiedCount=copied:record_count(),
    schema=window.records[1].schema,
    payloadCount=#window.records[1].payload,
    done=window.done,
    cancelled=cancelled,
    sourcePath=store:path(),
  }}
end)
"#
        );
        std::fs::write(&package, source).expect("write file-backed package");

        let host = Host::new(HostConfig::default()).expect("host starts");
        let package_path = package.to_string_lossy().into_owned();
        let loaded = host.load_extensions(std::slice::from_ref(&package_path));
        assert!(loaded.errors.is_empty(), "{:?}", loaded.errors);
        let outcomes = host
            .emit("record_store_probe", &serde_json::json!({}))
            .expect("dispatch probe");
        assert_eq!(outcomes.len(), 1);
        assert_eq!(outcomes[0].source, package_path);
        let result = outcomes[0]
            .result
            .as_ref()
            .expect("handler succeeds")
            .as_ref()
            .expect("handler returns value");
        assert_eq!(result["sequence"], 0);
        assert_eq!(result["copiedCount"], 1);
        assert_eq!(result["schema"], "different");
        assert_eq!(result["payloadCount"], 3);
        assert_eq!(result["done"], true);
        assert_eq!(result["cancelled"], true);
        assert!(
            result["sourcePath"]
                .as_str()
                .is_some_and(|path| path.starts_with(destination.to_string_lossy().as_ref()))
        );
    }
}
