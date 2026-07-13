//! JSON ⇄ Lua value conversion. JSON is the boundary format for everything
//! crossing the bridge (DESIGN.md read/write path: plain tables in, plain
//! tables out — no live host references). Resurrected from the attic
//! (`rebuild` @ `e8cb418`), then extended with wire-order preservation:
//! Pi renders model-emitted JSON with `JSON.stringify`, whose key order is
//! JS [[OwnPropertyKeys]] order over the parsed wire text. Lua tables are
//! unordered, so `json_to_lua` records that order in a metatable field and
//! `lua_to_json` replays it (PLAN 2b.5). Numbers follow JS semantics too:
//! JSON.parse collapses `1.0` to the integer 1 before stringify prints "1".

/// Metatable field carrying the JS-ordered key list of an object that
/// crossed the JSON→Lua boundary. Invisible to `pairs`; lost when Lua code
/// rebuilds the table, in which case encoding falls back to sorted keys
/// (the deterministic order Lua-authored tables have always produced).
pub(crate) const JSON_KEY_ORDER: &str = "__pi_rs_json_key_order";

/// Metatable flag marking a table as a JSON array. Set on every decoded
/// array so `[]` round-trips (an unmarked empty table is an object —
/// Lua has one empty-table value for both). Lua policy that builds
/// arrays it needs encoded as `[]` when empty sets the same flag
/// (e.g. the compaction details file lists, PLAN 6.5).
const JSON_ARRAY: &str = "__pi_rs_json_array";

/// Metatable list of object keys whose decoded value was JSON null. Lua has
/// no native null value, so the table slot is necessarily absent; retaining
/// the keys beside the wire-order list lets an untouched object round-trip
/// through `JSON.parse`/`JSON.stringify` without dropping explicit nulls.
const JSON_NULL_KEYS: &str = "__pi_rs_json_null_keys";

/// JS canonical array index: the canonical string form of an integer
/// 0 ≤ n < 2^32-1. [[OwnPropertyKeys]] lists these numerically ascending
/// ahead of every string key.
fn array_index(key: &str) -> Option<u32> {
    if key.is_empty() || key.len() > 10 || !key.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    if key.len() > 1 && key.starts_with('0') {
        return None;
    }
    key.parse::<u32>().ok().filter(|n| *n < u32::MAX)
}

/// The order `JSON.parse` yields for a wire object: canonical array indices
/// ascending first, then the remaining keys in wire (insertion) order.
fn js_key_order(map: &serde_json::Map<String, serde_json::Value>) -> Vec<&str> {
    let mut indices: Vec<(u32, &str)> = Vec::new();
    let mut names: Vec<&str> = Vec::new();
    for key in map.keys() {
        match array_index(key) {
            Some(n) => indices.push((n, key)),
            None => names.push(key),
        }
    }
    indices.sort_by_key(|(n, _)| *n);
    indices.into_iter().map(|(_, k)| k).chain(names).collect()
}

/// Exact i64 range of an f64: |f| ≤ 2^53.
const F64_EXACT_INT: f64 = 9_007_199_254_740_992.0;

pub(crate) fn json_to_lua(lua: &mlua::Lua, val: &serde_json::Value) -> mlua::Result<mlua::Value> {
    match val {
        serde_json::Value::Object(map) => {
            let table = lua.create_table()?;
            let null_keys = lua.create_table()?;
            for (k, v) in map {
                if v.is_null() {
                    null_keys.push(k.as_str())?;
                } else {
                    table.set(k.as_str(), json_to_lua(lua, v)?)?;
                }
            }
            let order = lua.create_table()?;
            for key in js_key_order(map) {
                order.push(key)?;
            }
            let meta = lua.create_table()?;
            meta.raw_set(JSON_KEY_ORDER, order)?;
            meta.raw_set(JSON_NULL_KEYS, null_keys)?;
            table.set_metatable(Some(meta))?;
            Ok(mlua::Value::Table(table))
        }
        serde_json::Value::Array(arr) => {
            let table = lua.create_table()?;
            for (i, v) in arr.iter().enumerate() {
                table.set(i + 1, json_to_lua(lua, v)?)?;
            }
            let meta = lua.create_table()?;
            meta.raw_set(JSON_ARRAY, true)?;
            table.set_metatable(Some(meta))?;
            Ok(mlua::Value::Table(table))
        }
        serde_json::Value::String(s) => Ok(mlua::Value::String(lua.create_string(s)?)),
        serde_json::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                Ok(mlua::Value::Integer(i))
            } else {
                let f = n.as_f64().unwrap_or(0.0);
                // JS has one number type: JSON.parse("1.0") is the integer 1.
                if f.fract() == 0.0 && f.abs() <= F64_EXACT_INT {
                    Ok(mlua::Value::Integer(f as i64))
                } else {
                    Ok(mlua::Value::Number(f))
                }
            }
        }
        serde_json::Value::Bool(b) => Ok(mlua::Value::Boolean(*b)),
        serde_json::Value::Null => Ok(mlua::Value::Nil),
    }
}

/// Convert JSON to recursively read-only Lua proxy tables. The backing tables
/// are captured only by metatables, so neither replacement of existing keys nor
/// insertion of new keys can mutate a dispatch snapshot.
pub(crate) fn immutable_json_to_lua(
    lua: &mlua::Lua,
    value: &serde_json::Value,
) -> mlua::Result<mlua::Value> {
    match value {
        serde_json::Value::Object(map) => {
            let backing = lua.create_table()?;
            for (key, value) in map {
                backing.raw_set(key.as_str(), immutable_json_to_lua(lua, value)?)?;
            }
            readonly_proxy(lua, backing)
        }
        serde_json::Value::Array(values) => {
            let backing = lua.create_table()?;
            for (index, value) in values.iter().enumerate() {
                backing.raw_set(index + 1, immutable_json_to_lua(lua, value)?)?;
            }
            readonly_proxy(lua, backing)
        }
        _ => json_to_lua(lua, value),
    }
}

fn readonly_proxy(lua: &mlua::Lua, backing: mlua::Table) -> mlua::Result<mlua::Value> {
    let proxy = lua.create_table()?;
    let metatable = lua.create_table()?;
    metatable.set("__index", backing.clone())?;
    metatable.set(
        "__len",
        lua.create_function({
            let backing = backing.clone();
            move |_, ()| Ok(backing.raw_len())
        })?,
    )?;
    metatable.set(
        "__pairs",
        lua.create_function(move |lua, ()| {
            let next: mlua::Function = lua.globals().get("next")?;
            Ok((next, backing.clone(), mlua::Nil))
        })?,
    )?;
    metatable.set(
        "__newindex",
        lua.create_function(
            |_, (_table, _key, _value): (mlua::Value, mlua::Value, mlua::Value)| {
                Err::<(), _>(mlua::Error::runtime("dispatch snapshot is immutable"))
            },
        )?,
    )?;
    metatable.set("__metatable", "locked")?;
    proxy.set_metatable(Some(metatable))?;
    Ok(mlua::Value::Table(proxy))
}

/// A table with sequential integer keys 1..=len becomes a JSON array;
/// everything else becomes an object. An empty table is an object
/// unless it carries the decoded-array metatable flag.
fn is_array_table(t: &mlua::Table) -> mlua::Result<bool> {
    let len = t.raw_len();
    if len == 0 {
        let flagged = t
            .metatable()
            .and_then(|meta| meta.raw_get::<bool>(JSON_ARRAY).ok())
            .unwrap_or(false);
        return Ok(flagged);
    }
    let mut count = 0;
    for pair in t.pairs::<mlua::Value, mlua::Value>() {
        let (k, _) = pair?;
        match k {
            mlua::Value::Integer(i) if i >= 1 && (i as usize) <= len => count += 1,
            _ => return Ok(false),
        }
    }
    Ok(count == len)
}

pub(crate) fn lua_to_json_strict(val: mlua::Value) -> mlua::Result<serde_json::Value> {
    validate_kernel_value(&val, &mut std::collections::HashSet::new())?;
    lua_to_json(val)
}

fn validate_kernel_value(
    value: &mlua::Value,
    ancestors: &mut std::collections::HashSet<usize>,
) -> mlua::Result<()> {
    match value {
        mlua::Value::Function(_)
        | mlua::Value::Thread(_)
        | mlua::Value::UserData(_)
        | mlua::Value::LightUserData(_)
        | mlua::Value::Error(_)
        | mlua::Value::Other(_) => Err(mlua::Error::runtime(
            "functions, threads, userdata, and errors cannot cross the kernel boundary",
        )),
        mlua::Value::Table(table) => {
            let pointer = table.to_pointer() as usize;
            if !ancestors.insert(pointer) {
                return Err(mlua::Error::runtime(
                    "cyclic tables cannot cross the kernel boundary",
                ));
            }
            for pair in table.clone().pairs::<mlua::Value, mlua::Value>() {
                let (key, value) = pair?;
                validate_kernel_value(&key, ancestors)?;
                validate_kernel_value(&value, ancestors)?;
            }
            ancestors.remove(&pointer);
            Ok(())
        }
        _ => Ok(()),
    }
}

pub(crate) fn lua_to_json(val: mlua::Value) -> mlua::Result<serde_json::Value> {
    match val {
        mlua::Value::Nil => Ok(serde_json::Value::Null),
        mlua::Value::Boolean(b) => Ok(serde_json::Value::Bool(b)),
        mlua::Value::Integer(i) => Ok(serde_json::Value::Number(i.into())),
        mlua::Value::Number(f) => {
            // JS number stringification: integral doubles print without a
            // fraction ("1", not "1.0"); non-finite numbers become null.
            if f.is_finite() && f.fract() == 0.0 && f.abs() <= F64_EXACT_INT {
                Ok(serde_json::Value::Number((f as i64).into()))
            } else {
                Ok(serde_json::Number::from_f64(f)
                    .map(serde_json::Value::Number)
                    .unwrap_or(serde_json::Value::Null))
            }
        }
        mlua::Value::String(s) => Ok(serde_json::Value::String(s.to_str()?.to_owned())),
        mlua::Value::Table(t) => {
            if is_array_table(&t)? {
                let mut arr = Vec::with_capacity(t.raw_len());
                for i in 1..=t.raw_len() {
                    arr.push(lua_to_json(t.get(i)?)?);
                }
                return Ok(serde_json::Value::Array(arr));
            }
            let mut entries: Vec<(String, mlua::Value)> = Vec::new();
            for pair in t.pairs::<mlua::Value, mlua::Value>() {
                let (k, v) = pair?;
                let key = match k {
                    mlua::Value::String(s) => s.to_str()?.to_owned(),
                    mlua::Value::Integer(i) => i.to_string(),
                    _ => continue,
                };
                entries.push((key, v));
            }
            // Keys are emitted in the recorded boundary order when present
            // (Pi's JSON.stringify order), then any Lua-added remainder
            // sorted — the deterministic order all tables produced before
            // order preservation landed. Explicit decoded null slots are
            // absent from the Lua table but retained in metatable metadata.
            entries.sort_by(|a, b| a.0.cmp(&b.0));
            let mut map = serde_json::Map::new();
            if let Some(meta) = t.metatable() {
                let null_keys: Vec<String> = meta
                    .raw_get::<mlua::Table>(JSON_NULL_KEYS)
                    .ok()
                    .map(|keys| keys.sequence_values::<String>().collect())
                    .transpose()?
                    .unwrap_or_default();
                if let Ok(order) = meta.raw_get::<mlua::Table>(JSON_KEY_ORDER) {
                    for key in order.sequence_values::<String>() {
                        let key = key?;
                        if let Some(pos) = entries.iter().position(|(k, _)| *k == key) {
                            let (key, value) = entries.remove(pos);
                            map.insert(key, lua_to_json(value)?);
                        } else if null_keys.contains(&key) {
                            map.insert(key, serde_json::Value::Null);
                        }
                    }
                }
            }
            for (key, value) in entries {
                map.insert(key, lua_to_json(value)?);
            }
            Ok(serde_json::Value::Object(map))
        }
        // Functions, userdata, threads: not representable at the boundary.
        _ => Ok(serde_json::Value::Null),
    }
}
