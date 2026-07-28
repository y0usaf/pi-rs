//! Versioned durable-record persistence for ordinary Lua packages.
//!
//! The module exposes only opaque JSON append, bounded cursor windows, atomic
//! prefix copy, listing diagnostics, and explicit close. Every destination is
//! supplied by Lua, so no resource-path, schema, or session policy exists here.
//! Each open store is registered through the same scope resource path as
//! `pi.kernel.v1.resource`, so package disposal closes the store and releases
//! its file lock even when Lua never calls `close`.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use mlua::{AnyUserData, Lua, ObjectLike, Table, UserData, UserDataMethods, Value};
use pi_rs_session::{
    CancellationToken, FORMAT_VERSION, RecordCursor, RecordStore, StoreError, StoreLimits,
};

use crate::convert::{json_to_lua, lua_to_json_strict};
use crate::kernel::Control;

/// Records API version. Independent of the store's on-disk format version.
const RECORDS_API_VERSION: u32 = 1;

fn store_error(error: StoreError) -> mlua::Error {
    mlua::Error::runtime(error.to_string())
}

type StoreSlot = Rc<RefCell<Option<RecordStore>>>;

struct LuaStore {
    control: Arc<Control>,
    limits: StoreLimits,
    slot: StoreSlot,
    resource: AnyUserData,
}

impl LuaStore {
    /// Take ownership of an open store as one scope-owned disposable resource.
    fn adopt(
        lua: &Lua,
        control: &Arc<Control>,
        limits: StoreLimits,
        store: RecordStore,
    ) -> mlua::Result<Self> {
        let slot: StoreSlot = Rc::new(RefCell::new(Some(store)));
        let disposed = Rc::clone(&slot);
        let callback = lua.create_function(move |_, ()| {
            disposed.borrow_mut().take();
            Ok(())
        })?;
        let resource = crate::kernel_api::register_scoped_resource(lua, control, callback)?;
        Ok(Self {
            control: Arc::clone(control),
            limits,
            slot,
            resource,
        })
    }

    fn with<T>(&self, apply: impl FnOnce(&mut RecordStore) -> mlua::Result<T>) -> mlua::Result<T> {
        let mut slot = self.slot.borrow_mut();
        let store = slot
            .as_mut()
            .ok_or_else(|| mlua::Error::runtime("record store is closed"))?;
        apply(store)
    }
}

struct LuaCursor {
    cursor: Rc<RefCell<RecordCursor>>,
    limits: StoreLimits,
}

/// Resolve the cancellation observed by one operation. An explicit
/// `cancellation` overrides the innermost dispatch token. Record operations are
/// synchronous and bounded, so an already-cancelled token fails the call before
/// any blocking work rather than interrupting a partial commit.
fn cancellation_from(lua: &Lua, options: Option<&Table>) -> mlua::Result<CancellationToken> {
    let token = CancellationToken::new();
    let explicit = match options {
        Some(options) => options.get::<Option<AnyUserData>>("cancellation")?,
        None => None,
    };
    let observed = match explicit {
        Some(userdata) => Some(
            userdata
                .borrow::<crate::kernel_api::LuaCancellation>()
                .map_err(|_| {
                    mlua::Error::runtime("records cancellation must be a kernel cancellation")
                })?
                .0
                .clone(),
        ),
        None => crate::kernel_api::current_cancellation(lua)?,
    };
    if observed.is_some_and(|observed| observed.is_cancelled()) {
        token.cancel();
    }
    Ok(token)
}

fn limits_from(options: Option<&Table>) -> mlua::Result<StoreLimits> {
    let defaults = StoreLimits::default();
    let Some(options) = options else {
        return Ok(defaults);
    };
    let Some(limits) = options.get::<Option<Table>>("limits")? else {
        return Ok(defaults);
    };
    Ok(StoreLimits {
        max_record_bytes: limits
            .get::<Option<usize>>("max_record_bytes")?
            .unwrap_or(defaults.max_record_bytes),
        max_window_records: limits
            .get::<Option<usize>>("max_window_records")?
            .unwrap_or(defaults.max_window_records),
        max_window_bytes: limits
            .get::<Option<usize>>("max_window_bytes")?
            .unwrap_or(defaults.max_window_bytes),
    })
}

impl UserData for LuaStore {
    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        methods.add_method("path", |_, this, ()| {
            this.with(|store| Ok(store.path().to_string_lossy().into_owned()))
        });
        methods.add_method("record_count", |_, this, ()| {
            this.with(|store| Ok(store.record_count()))
        });
        methods.add_method(
            "append",
            |lua, this, (value, options): (Value, Option<Table>)| {
                let record = lua_to_json_strict(value)?;
                let cancellation = cancellation_from(lua, options.as_ref())?;
                this.with(|store| store.append(&record, &cancellation).map_err(store_error))
            },
        );
        methods.add_method("cursor", |_, this, ()| {
            let limits = this.limits;
            this.with(|store| {
                let cursor = store.cursor().map_err(store_error)?;
                Ok(LuaCursor {
                    cursor: Rc::new(RefCell::new(cursor)),
                    limits,
                })
            })
        });
        methods.add_method("copy", |lua, this, options: Table| {
            let directory: String = options.get("directory")?;
            let name: String = options.get("name")?;
            let record_count = options.get::<Option<u64>>("record_count")?;
            let cancellation = cancellation_from(lua, Some(&options))?;
            let copied = this.with(|store| {
                store
                    .copy_prefix(&directory, &name, record_count, &cancellation)
                    .map_err(store_error)
            })?;
            LuaStore::adopt(lua, &this.control, this.limits, copied)
        });
        methods.add_method("close", |_, this, ()| {
            this.resource.call_method::<()>("dispose", ())
        });
        methods.add_method("closed", |_, this, ()| Ok(this.slot.borrow().is_none()));
    }
}

impl UserData for LuaCursor {
    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        methods.add_method("next_sequence", |_, this, ()| {
            Ok(this.cursor.borrow().next_sequence())
        });
        methods.add_method("next", |lua, this, options: Option<Table>| {
            let max_records = match options.as_ref() {
                Some(options) => options.get::<Option<usize>>("max_records")?,
                None => None,
            }
            .unwrap_or(this.limits.max_window_records);
            let max_bytes = match options.as_ref() {
                Some(options) => options.get::<Option<usize>>("max_bytes")?,
                None => None,
            }
            .unwrap_or(this.limits.max_window_bytes);
            let cancellation = cancellation_from(lua, options.as_ref())?;
            let window = this
                .cursor
                .borrow_mut()
                .next_window(max_records, max_bytes, &cancellation)
                .map_err(store_error)?;
            let result = lua.create_table()?;
            result.set(
                "records",
                json_to_lua(lua, &serde_json::Value::Array(window.records))?,
            )?;
            result.set("start_sequence", window.start_sequence)?;
            result.set("next_sequence", window.next_sequence)?;
            result.set("encoded_bytes", window.encoded_bytes)?;
            result.set("done", window.done)?;
            Ok(result)
        });
    }
}

fn limits_table(lua: &Lua, limits: StoreLimits) -> mlua::Result<Table> {
    let table = lua.create_table()?;
    table.set("max_record_bytes", limits.max_record_bytes)?;
    table.set("max_window_records", limits.max_window_records)?;
    table.set("max_window_bytes", limits.max_window_bytes)?;
    Ok(table)
}

pub(crate) fn install(lua: &Lua, pi: &Table, control: Arc<Control>) -> mlua::Result<()> {
    let v1 = lua.create_table()?;
    v1.set("api_version", RECORDS_API_VERSION)?;
    v1.set("format_version", FORMAT_VERSION)?;
    v1.set("extension", pi_rs_session::STORE_EXTENSION)?;
    v1.set("default_limits", limits_table(lua, StoreLimits::default())?)?;

    let create_control = Arc::clone(&control);
    v1.set(
        "create",
        lua.create_function(move |lua, options: Table| {
            let directory: String = options.get("directory")?;
            let name: String = options.get("name")?;
            let limits = limits_from(Some(&options))?;
            let cancellation = cancellation_from(lua, Some(&options))?;
            let store = RecordStore::create(&directory, &name, limits, &cancellation)
                .map_err(store_error)?;
            LuaStore::adopt(lua, &create_control, limits, store)
        })?,
    )?;

    let open_control = Arc::clone(&control);
    v1.set(
        "open",
        lua.create_function(move |lua, options: Table| {
            let path: String = options.get("path")?;
            let limits = limits_from(Some(&options))?;
            let cancellation = cancellation_from(lua, Some(&options))?;
            let store = RecordStore::open(&path, limits, &cancellation).map_err(store_error)?;
            LuaStore::adopt(lua, &open_control, limits, store)
        })?,
    )?;

    v1.set(
        "list",
        lua.create_function(|lua, options: Table| {
            let directory: String = options.get("directory")?;
            let limits = limits_from(Some(&options))?;
            let cancellation = cancellation_from(lua, Some(&options))?;
            let listing =
                RecordStore::list(&directory, limits, &cancellation).map_err(store_error)?;
            let stores = lua.create_table()?;
            for (index, info) in listing.stores.into_iter().enumerate() {
                let row = lua.create_table()?;
                row.set("name", info.name)?;
                row.set("path", info.path.to_string_lossy().into_owned())?;
                row.set("format_version", info.format_version)?;
                row.set("record_count", info.record_count)?;
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
            let result = lua.create_table()?;
            result.set("stores", stores)?;
            result.set("diagnostics", diagnostics)?;
            Ok(result)
        })?,
    )?;

    let records = lua.create_table()?;
    records.set("v1", v1)?;
    pi.set("records", records)
}
