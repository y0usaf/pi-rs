//! pi.records — named-record CRUD over the session JSONL (P2 tier-2
//! binding). Lua surface over crates/pi-rs-session::records.
//!
//! The harness schema (memories, skills, prompts, subagents, refinements)
//! is Lua policy at P4; this binding is the store, and collection names are
//! data — Rust knows none of them.
//!
//! Lua surface:
//! - records.open(path) -> store
//! - store:put(collection, key, value)
//! - store:delete(collection, key)
//! - store:get(collection, key) -> value | nil
//! - store:list(collection) -> array of { key, value }
//! - store:count_entries() -> number

use mlua::{Lua, Table, UserData, UserDataMethods};
use pi_rs_session::records::RecordStore;

use crate::convert::{json_to_lua, lua_to_json};

struct RecordStoreUserData(RecordStore);

impl UserData for RecordStoreUserData {
    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        methods.add_method("put", |_lua, this, (collection, key, value): (String, String, mlua::Value)| {
            let value = lua_to_json(value).map_err(mlua::Error::external)?;
            this.0
                .put(&collection, &key, value)
                .map_err(mlua::Error::external)
        });
        methods.add_method("delete", |_, this, (collection, key): (String, String)| {
            this.0
                .delete(&collection, &key)
                .map_err(mlua::Error::external)
        });
        methods.add_method("get", |lua, this, (collection, key): (String, String)| {
            match this.0.get(&collection, &key).map_err(mlua::Error::external)? {
                Some(value) => json_to_lua(lua, &value),
                None => Ok(mlua::Value::Nil),
            }
        });
        methods.add_method("list", |lua, this, collection: String| {
            let rows = this.0.list(&collection).map_err(mlua::Error::external)?;
            let table = lua.create_table()?;
            for (i, (key, value)) in rows.into_iter().enumerate() {
                let row = lua.create_table()?;
                row.set("key", key)?;
                row.set("value", json_to_lua(lua, &value)?)?;
                table.set(i + 1, row)?;
            }
            Ok(table)
        });
        methods.add_method("count_entries", |_, this, ()| {
            this.0.count_entries().map_err(mlua::Error::external)
        });
    }
}

pub(crate) fn install(lua: &Lua, pi: &Table) -> mlua::Result<()> {
    let records = lua.create_table()?;
    records.set(
        "open",
        lua.create_function(|_, path: String| {
            Ok(RecordStoreUserData(RecordStore::new(path)))
        })?,
    )?;
    pi.set("records", records)
}
