//! Versioned Lua kernel surface and transaction-local publication queues.

use std::sync::Arc;

use mlua::{AnyUserData, Function, Table, UserData, UserDataMethods, Value};

use crate::convert::{immutable_json_to_lua, json_to_lua, lua_to_json_strict};
use crate::kernel::{
    Action, CancellationToken, Control, DeclarationKind, DispatchBatch, DispatchRequest, Effect,
    Generation, KERNEL_API_VERSION, MAX_BATCH_ITEMS, MAX_ITEM_BYTES, ResourceId, RootKind, ScopeId,
};
use crate::{HostConfig, HostError};

const ROOTS_KEY: &str = "kernel_roots";
const DECLARATIONS_KEY: &str = "kernel_declarations";
const RESOURCES_KEY: &str = "kernel_resources";
const TRANSACTION_KEY: &str = "kernel_transaction";
const CURRENT_SCOPE_KEY: &str = "kernel_scope";

#[derive(Clone)]
struct LuaCancellation(CancellationToken);

impl UserData for LuaCancellation {
    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        methods.add_method("is_cancelled", |_, this, ()| Ok(this.0.is_cancelled()));
        methods.add_async_method("wait", |_, this, ()| async move {
            this.0.cancelled().await;
            Ok(())
        });
    }
}

struct LuaReadHandle {
    handle: crate::kernel::ReadHandle,
    control: Arc<Control>,
}

impl UserData for LuaReadHandle {
    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        methods.add_method("generation", |_, this, ()| {
            Ok(this.handle.generation().get())
        });
        methods.add_method("read", |lua, this, ()| {
            let value = this
                .control
                .read_handle(&this.handle)
                .map_err(mlua::Error::external)?;
            immutable_json_to_lua(lua, &value)
        });
    }
}

struct LuaResource {
    scope: ScopeId,
    id: ResourceId,
    entry: Table,
    control: Arc<Control>,
}

impl UserData for LuaResource {
    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        methods.add_method("dispose", |_, this, ()| {
            if this.entry.get::<bool>("active")? {
                let callback: Function = this.entry.get("callback")?;
                callback.call::<()>(())?;
                this.entry.set("active", false)?;
            }
            this.control.release_resource(this.scope, this.id);
            Ok(())
        });
        methods.add_method("disposed", |_, this, ()| {
            Ok(!this.entry.get::<bool>("active")?)
        });
    }
}

pub(crate) struct ResolvedRoot {
    pub(crate) source: String,
    pub(crate) scope: ScopeId,
    pub(crate) handler: Function,
}

pub(crate) struct ResourceDisposer {
    pub(crate) source: String,
    pub(crate) id: ResourceId,
    pub(crate) callback: Function,
}

pub(crate) fn install(
    lua: &mlua::Lua,
    pi: &Table,
    module_api: &Table,
    control: Arc<Control>,
) -> mlua::Result<()> {
    let registry = crate::api::registry_table(lua)?;
    registry.set(ROOTS_KEY, lua.create_table()?)?;
    registry.set(DECLARATIONS_KEY, lua.create_table()?)?;
    registry.set(RESOURCES_KEY, lua.create_table()?)?;
    registry.set(TRANSACTION_KEY, Value::Nil)?;
    registry.set(CURRENT_SCOPE_KEY, 0_u64)?;

    let version = lua.create_table()?;
    version.set("api_version", KERNEL_API_VERSION)?;
    version.set("module", module_api.clone())?;

    version.set(
        "root",
        lua.create_function(|lua, definition: Table| register_root(lua, definition))?,
    )?;
    version.set(
        "declare",
        lua.create_function(|lua, (kind, definition): (String, Table)| {
            register_declaration(lua, &kind, definition)
        })?,
    )?;
    version.set(
        "registered",
        lua.create_function(|lua, kind: String| registered_declarations(lua, &kind))?,
    )?;
    version.set(
        "action",
        lua.create_function(|lua, (kind, payload): (String, Option<Value>)| {
            queue_item(lua, "actions", &kind, payload.unwrap_or(Value::Nil))
        })?,
    )?;
    version.set(
        "effect",
        lua.create_function(|lua, (kind, payload): (String, Option<Value>)| {
            queue_item(lua, "effects", &kind, payload.unwrap_or(Value::Nil))
        })?,
    )?;

    let handle_control = Arc::clone(&control);
    version.set(
        "read_handle",
        lua.create_function(move |lua, value: Value| {
            let value = lua_to_json_strict(value)?;
            lua.create_userdata(LuaReadHandle {
                handle: handle_control.issue_handle(value),
                control: Arc::clone(&handle_control),
            })
        })?,
    )?;
    version.set(
        "cancellation",
        lua.create_function(|lua, ()| {
            let transaction = active_transaction(lua)?;
            transaction.get::<AnyUserData>("cancellation")
        })?,
    )?;

    let resource_control = Arc::clone(&control);
    version.set(
        "resource",
        lua.create_function(move |lua, callback: Function| {
            let registry = crate::api::registry_table(lua)?;
            let scope = ScopeId::from_raw(registry.get::<u64>(CURRENT_SCOPE_KEY)?);
            if scope.get() == 0 {
                return Err(mlua::Error::runtime(
                    "resource registration requires a package or dispatch scope",
                ));
            }
            let id = resource_control
                .register_resource(scope)
                .map_err(mlua::Error::external)?;
            let resources: Table = registry.get(RESOURCES_KEY)?;
            let scope_key = scope.get();
            let list = resources
                .get::<Option<Table>>(scope_key)?
                .unwrap_or(lua.create_table()?);
            let entry = lua.create_table()?;
            entry.set("id", id.get())?;
            entry.set("source", crate::api::current_source(lua))?;
            entry.set("callback", callback)?;
            entry.set("active", true)?;
            list.push(entry.clone())?;
            resources.set(scope_key, list)?;
            lua.create_userdata(LuaResource {
                scope,
                id,
                entry,
                control: Arc::clone(&resource_control),
            })
        })?,
    )?;

    let kernel = lua.create_table()?;
    kernel.set("v1", version)?;
    pi.set("kernel", kernel)
}

fn register_root(lua: &mlua::Lua, definition: Table) -> mlua::Result<()> {
    let kind = required_name(&definition, "kind", "root kind")?;
    RootKind::parse(&kind).map_err(mlua::Error::external)?;
    let id = required_name(&definition, "id", "root id")?;
    let _handler: Function = definition
        .get("dispatch")
        .map_err(|_| mlua::Error::runtime("root dispatch must be a function"))?;
    let active = definition.get::<Option<bool>>("active")?.unwrap_or(true);
    let priority = definition.get::<Option<i64>>("priority")?.unwrap_or(0);
    let source = crate::api::current_source(lua);
    let scope = current_scope(lua)?;
    let registry = crate::api::registry_table(lua)?;
    let roots: Table = registry.get(ROOTS_KEY)?;
    let key = format!("{kind}\0{id}");
    if let Some(existing) = roots.get::<Option<Table>>(key.as_str())? {
        let other: String = existing.get("source")?;
        return Err(conflict_error("root", &kind, &id, &source, &other));
    }
    let entry = copy_table(lua, &definition)?;
    entry.set("kind", kind)?;
    entry.set("id", id)?;
    entry.set("active", active)?;
    entry.set("priority", priority)?;
    entry.set("source", source)?;
    entry.set("scope", scope.get())?;
    roots.set(key, entry)
}

fn register_declaration(lua: &mlua::Lua, kind: &str, definition: Table) -> mlua::Result<()> {
    DeclarationKind::parse(kind).map_err(mlua::Error::external)?;
    let id = definition
        .get::<Option<String>>("id")?
        .or(definition.get::<Option<String>>("name")?)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| mlua::Error::runtime("declaration id/name must be a non-empty string"))?;
    let source = crate::api::current_source(lua);
    let scope = current_scope(lua)?;
    let registry = crate::api::registry_table(lua)?;
    let declarations: Table = registry.get(DECLARATIONS_KEY)?;
    let key = format!("{kind}\0{id}");
    if let Some(existing) = declarations.get::<Option<Table>>(key.as_str())? {
        let other: String = existing.get("source")?;
        return Err(conflict_error("declaration", kind, &id, &source, &other));
    }
    let entry = copy_table(lua, &definition)?;
    entry.set("kind", kind)?;
    entry.set("id", id)?;
    entry.set(
        "order",
        definition.get::<Option<i64>>("order")?.unwrap_or(0),
    )?;
    entry.set("source", source)?;
    entry.set("scope", scope.get())?;
    declarations.set(key, entry)
}

fn registered_declarations(lua: &mlua::Lua, kind: &str) -> mlua::Result<Table> {
    DeclarationKind::parse(kind).map_err(mlua::Error::external)?;
    let declarations: Table = crate::api::registry_table(lua)?.get(DECLARATIONS_KEY)?;
    let mut entries = declarations
        .pairs::<String, Table>()
        .filter_map(|pair| match pair {
            Ok((_, entry)) if entry.get::<String>("kind").is_ok_and(|value| value == kind) => {
                Some(Ok(entry))
            }
            Ok(_) => None,
            Err(error) => Some(Err(error)),
        })
        .collect::<mlua::Result<Vec<_>>>()?;
    entries.sort_by(|left, right| {
        let left_key = (
            left.get::<i64>("order").unwrap_or(0),
            left.get::<String>("source").unwrap_or_default(),
            left.get::<String>("id").unwrap_or_default(),
        );
        let right_key = (
            right.get::<i64>("order").unwrap_or(0),
            right.get::<String>("source").unwrap_or_default(),
            right.get::<String>("id").unwrap_or_default(),
        );
        left_key.cmp(&right_key)
    });
    let result = lua.create_table()?;
    for entry in entries {
        result.push(entry)?;
    }
    Ok(result)
}

fn required_name(table: &Table, key: &str, label: &str) -> mlua::Result<String> {
    table
        .get::<Option<String>>(key)?
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| mlua::Error::runtime(format!("{label} must be a non-empty string")))
}

fn current_scope(lua: &mlua::Lua) -> mlua::Result<ScopeId> {
    let raw = crate::api::registry_table(lua)?.get::<u64>(CURRENT_SCOPE_KEY)?;
    if raw == 0 {
        return Err(mlua::Error::runtime(
            "kernel declarations require a package scope",
        ));
    }
    Ok(ScopeId::from_raw(raw))
}

fn copy_table(lua: &mlua::Lua, source: &Table) -> mlua::Result<Table> {
    let copied = lua.create_table()?;
    for pair in source.clone().pairs::<Value, Value>() {
        let (key, value) = pair?;
        copied.set(key, value)?;
    }
    Ok(copied)
}

fn conflict_error(kind: &str, family: &str, id: &str, left: &str, right: &str) -> mlua::Error {
    let mut sources = [left, right];
    sources.sort_unstable();
    mlua::Error::runtime(format!(
        "{}{kind} conflict for {family}/{id}: {} <> {}",
        crate::error::CONFLICT_MARKER,
        sources[0],
        sources[1]
    ))
}

pub(crate) fn set_scope(lua: &mlua::Lua, scope: Option<ScopeId>) -> mlua::Result<()> {
    crate::api::registry_table(lua)?.set(
        CURRENT_SCOPE_KEY,
        scope.map(ScopeId::get).unwrap_or_default(),
    )
}

pub(crate) fn resolve_root(lua: &mlua::Lua, kind: RootKind) -> Result<ResolvedRoot, HostError> {
    let roots: Table = crate::api::registry_table(lua)
        .and_then(|registry| registry.get(ROOTS_KEY))
        .map_err(|error| HostError::Lua(error.to_string()))?;
    let mut candidates = roots
        .pairs::<String, Table>()
        .filter_map(|pair| match pair {
            Ok((_, entry))
                if entry
                    .get::<String>("kind")
                    .is_ok_and(|value| value == kind.as_str())
                    && entry.get::<bool>("active").unwrap_or(false) =>
            {
                Some(Ok(entry))
            }
            Ok(_) => None,
            Err(error) => Some(Err(error)),
        })
        .collect::<mlua::Result<Vec<_>>>()
        .map_err(|error| HostError::Lua(error.to_string()))?;
    let Some(priority) = candidates
        .iter()
        .filter_map(|entry| entry.get::<i64>("priority").ok())
        .max()
    else {
        return Err(HostError::UnknownRoot(kind.as_str().to_owned()));
    };
    candidates.retain(|entry| {
        entry
            .get::<i64>("priority")
            .is_ok_and(|value| value == priority)
    });
    candidates.sort_by_key(|entry| {
        (
            entry.get::<String>("source").unwrap_or_default(),
            entry.get::<String>("id").unwrap_or_default(),
        )
    });
    if candidates.len() != 1 {
        let owners = candidates
            .iter()
            .map(|entry| {
                format!(
                    "{}:{}",
                    entry.get::<String>("source").unwrap_or_default(),
                    entry.get::<String>("id").unwrap_or_default()
                )
            })
            .collect::<Vec<_>>()
            .join(", ");
        return Err(HostError::Conflict(format!(
            "root {} priority {priority}: {owners}",
            kind.as_str()
        )));
    }
    let entry = candidates.remove(0);
    Ok(ResolvedRoot {
        source: entry
            .get("source")
            .map_err(|error| HostError::Lua(error.to_string()))?,
        scope: ScopeId::from_raw(
            entry
                .get("scope")
                .map_err(|error| HostError::Lua(error.to_string()))?,
        ),
        handler: entry
            .get("dispatch")
            .map_err(|error| HostError::Lua(error.to_string()))?,
    })
}

pub(crate) fn begin_transaction(
    lua: &mlua::Lua,
    generation: Generation,
    scope: ScopeId,
    cancellation: CancellationToken,
) -> mlua::Result<()> {
    let transaction = lua.create_table()?;
    transaction.set("generation", generation.get())?;
    transaction.set("scope", scope.get())?;
    transaction.set("next_sequence", 0_u64)?;
    transaction.set("actions", lua.create_table()?)?;
    transaction.set("effects", lua.create_table()?)?;
    transaction.set(
        "cancellation",
        lua.create_userdata(LuaCancellation(cancellation))?,
    )?;
    crate::api::registry_table(lua)?.set(TRANSACTION_KEY, transaction)
}

fn active_transaction(lua: &mlua::Lua) -> mlua::Result<Table> {
    crate::api::registry_table(lua)?
        .get::<Option<Table>>(TRANSACTION_KEY)?
        .ok_or_else(|| {
            mlua::Error::runtime("actions and effects may only be queued during dispatch")
        })
}

pub(crate) fn current_cancellation(lua: &mlua::Lua) -> mlua::Result<Option<CancellationToken>> {
    let Some(transaction) =
        crate::api::registry_table(lua)?.get::<Option<Table>>(TRANSACTION_KEY)?
    else {
        return Ok(None);
    };
    let value: AnyUserData = transaction.get("cancellation")?;
    Ok(Some(value.borrow::<LuaCancellation>()?.0.clone()))
}

fn queue_item(lua: &mlua::Lua, list_key: &str, kind: &str, payload: Value) -> mlua::Result<()> {
    if kind.trim().is_empty() {
        return Err(mlua::Error::runtime(
            "action/effect kind must be a non-empty string",
        ));
    }
    let payload = lua_to_json_strict(payload)?;
    let bytes = serde_json::to_vec(&payload)
        .map_err(|error| mlua::Error::runtime(error.to_string()))?
        .len();
    if bytes > MAX_ITEM_BYTES {
        return Err(mlua::Error::runtime(format!(
            "action/effect payload exceeds {MAX_ITEM_BYTES} bytes"
        )));
    }
    let transaction = active_transaction(lua)?;
    let list: Table = transaction.get(list_key)?;
    if list.raw_len() >= MAX_BATCH_ITEMS {
        return Err(mlua::Error::runtime(format!(
            "dispatch exceeds {MAX_BATCH_ITEMS} queued {list_key}"
        )));
    }
    let sequence = transaction.get::<u64>("next_sequence")?;
    transaction.set("next_sequence", sequence + 1)?;
    let entry = lua.create_table()?;
    entry.set("sequence", sequence)?;
    entry.set("kind", kind)?;
    entry.set("payload", json_to_lua(lua, &payload)?)?;
    list.push(entry)
}

pub(crate) fn snapshot(
    lua: &mlua::Lua,
    request: &DispatchRequest,
    generation: Generation,
    scope: ScopeId,
) -> mlua::Result<Value> {
    let value = serde_json::json!({
        "version": KERNEL_API_VERSION,
        "generation": generation.get(),
        "scope": scope.get(),
        "event": request.event,
        "context": request.context,
    });
    immutable_json_to_lua(lua, &value)
}

pub(crate) fn finish_transaction(
    lua: &mlua::Lua,
    source: String,
) -> Result<DispatchBatch, HostError> {
    let transaction = active_transaction(lua).map_err(|error| HostError::Lua(error.to_string()))?;
    let generation = Generation::from_raw(
        transaction
            .get("generation")
            .map_err(|error| HostError::Lua(error.to_string()))?,
    );
    let scope = ScopeId::from_raw(
        transaction
            .get("scope")
            .map_err(|error| HostError::Lua(error.to_string()))?,
    );
    let actions = read_actions(&transaction).map_err(|error| HostError::Lua(error.to_string()))?;
    let effects =
        read_effects(&transaction, scope).map_err(|error| HostError::Lua(error.to_string()))?;
    clear_transaction(lua);
    Ok(DispatchBatch {
        version: KERNEL_API_VERSION,
        generation,
        scope,
        source,
        actions,
        effects,
    })
}

fn read_actions(transaction: &Table) -> mlua::Result<Vec<Action>> {
    let list: Table = transaction.get("actions")?;
    let mut actions = Vec::with_capacity(list.raw_len());
    for entry in list.sequence_values::<Table>() {
        let entry = entry?;
        actions.push(Action {
            sequence: entry.get("sequence")?,
            kind: entry.get("kind")?,
            payload: lua_to_json_strict(entry.get("payload")?)?,
        });
    }
    actions.sort_by_key(|action| action.sequence);
    Ok(actions)
}

fn read_effects(transaction: &Table, scope: ScopeId) -> mlua::Result<Vec<Effect>> {
    let list: Table = transaction.get("effects")?;
    let mut effects = Vec::with_capacity(list.raw_len());
    for entry in list.sequence_values::<Table>() {
        let entry = entry?;
        effects.push(Effect {
            sequence: entry.get("sequence")?,
            kind: entry.get("kind")?,
            payload: lua_to_json_strict(entry.get("payload")?)?,
            scope,
        });
    }
    effects.sort_by_key(|effect| effect.sequence);
    Ok(effects)
}

pub(crate) fn clear_transaction(lua: &mlua::Lua) {
    if let Ok(registry) = crate::api::registry_table(lua) {
        let _ = registry.set(TRANSACTION_KEY, Value::Nil);
    }
}

pub(crate) fn take_resource_disposers(
    lua: &mlua::Lua,
    scope: ScopeId,
) -> mlua::Result<Vec<ResourceDisposer>> {
    let resources: Table = crate::api::registry_table(lua)?.get(RESOURCES_KEY)?;
    let Some(list) = resources.get::<Option<Table>>(scope.get())? else {
        return Ok(Vec::new());
    };
    resources.set(scope.get(), Value::Nil)?;
    list.sequence_values::<Table>()
        .filter_map(|entry| match entry {
            Ok(entry) if entry.get::<bool>("active").unwrap_or(false) => {
                Some(Ok(ResourceDisposer {
                    source: entry.get("source").unwrap_or_default(),
                    id: ResourceId::from_raw(entry.get("id").unwrap_or_default()),
                    callback: match entry.get("callback") {
                        Ok(callback) => callback,
                        Err(error) => return Some(Err(error)),
                    },
                }))
            }
            Ok(_) => None,
            Err(error) => Some(Err(error)),
        })
        .collect()
}

pub(crate) fn remove_source(lua: &mlua::Lua, source: &str) -> mlua::Result<()> {
    let registry = crate::api::registry_table(lua)?;
    for table_key in [ROOTS_KEY, DECLARATIONS_KEY] {
        let table: Table = registry.get(table_key)?;
        let keys = table
            .clone()
            .pairs::<String, Table>()
            .filter_map(|pair| match pair {
                Ok((key, entry))
                    if entry
                        .get::<String>("source")
                        .is_ok_and(|owner| owner == source) =>
                {
                    Some(Ok(key))
                }
                Ok(_) => None,
                Err(error) => Some(Err(error)),
            })
            .collect::<mlua::Result<Vec<_>>>()?;
        for key in keys {
            table.set(key, Value::Nil)?;
        }
    }
    Ok(())
}

pub(crate) fn dispose_callbacks(
    lua: &mlua::Lua,
    rt: &tokio::runtime::Runtime,
    config: &HostConfig,
    control: &Control,
    scope: ScopeId,
) -> Result<(), HostError> {
    let disposers =
        take_resource_disposers(lua, scope).map_err(|error| HostError::Lua(error.to_string()))?;
    let mut first_error = None;
    for disposer in disposers {
        crate::api::set_current_source(lua, &disposer.source);
        if let Err(error) =
            crate::vm::dispatch_function(lua, rt, config, disposer.callback, (), None)
            && first_error.is_none()
        {
            first_error = Some(error);
        }
        control.release_resource(scope, disposer.id);
    }
    crate::api::set_current_source(lua, "<host>");
    first_error.map_or(Ok(()), Err)
}
