//! Package composition and lifecycle for ordinary Lua packages.
//!
//! A package may compose other packages: it loads them, lists what it composed,
//! and disposes them. Rust chooses no location, order, name, or reload policy —
//! the caller passes an explicit path or an inline named source, so selection,
//! precedence, and swap order stay entirely in Lua.
//!
//! A loaded package is registered as one disposable resource of its loader
//! through the same path as `pi.kernel.v1.resource`, so disposing a composing
//! package disposes everything it composed, transitively.

use std::sync::Arc;

use mlua::{Lua, Table, UserData, UserDataMethods, Value};

use crate::effects::EffectHub;
use crate::kernel::{Control, ScopeId};

/// Packages API version.
const PACKAGES_API_VERSION: u32 = 1;
/// Registry key of the loaded-package record, in load order.
const PACKAGES_KEY: &str = "lua_packages";
/// Registry key of the in-flight nested-load stack.
const LOAD_STACK_KEY: &str = "lua_package_load_stack";
/// Maximum depth of package loads nested inside package loads.
const MAX_LOAD_DEPTH: usize = 4;
/// Maximum number of simultaneously Lua-loaded packages.
const MAX_PACKAGES: usize = 64;
/// Maximum package source size accepted through this mechanism.
const MAX_SOURCE_BYTES: usize = 4 * 1024 * 1024;

struct LuaPackage {
    source: String,
    scope: ScopeId,
    control: Arc<Control>,
    effects: EffectHub,
}

impl UserData for LuaPackage {
    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        methods.add_method("source", |_, this, ()| Ok(this.source.clone()));
        methods.add_method("scope", |_, this, ()| Ok(this.scope.get()));
        methods.add_method("disposed", |lua, this, ()| {
            Ok(active_entry(lua, this.scope)?.is_none())
        });
        methods.add_async_method("dispose", |lua, this, ()| async move {
            dispose_package(&lua, &this.control, &this.effects, this.scope)
                .await
                .map(|_| ())
        });
    }
}

fn registry_list(lua: &Lua, key: &str) -> mlua::Result<Table> {
    crate::api::registry_table(lua)?.get::<Table>(key)
}

fn active_entry(lua: &Lua, scope: ScopeId) -> mlua::Result<Option<Table>> {
    for entry in registry_list(lua, PACKAGES_KEY)?.sequence_values::<Table>() {
        let entry = entry?;
        if entry.get::<u64>("scope")? == scope.get() && entry.get::<bool>("active")? {
            return Ok(Some(entry));
        }
    }
    Ok(None)
}

/// Drop disposed records so a long-running load/dispose cycle cannot grow the
/// record without bound.
fn prune_disposed(lua: &Lua) -> mlua::Result<()> {
    let list = registry_list(lua, PACKAGES_KEY)?;
    let kept = lua.create_table()?;
    for entry in list.sequence_values::<Table>() {
        let entry = entry?;
        if entry.get::<bool>("active")? {
            kept.push(entry)?;
        }
    }
    crate::api::registry_table(lua)?.set(PACKAGES_KEY, kept)
}

fn loading(lua: &Lua, scope: ScopeId) -> mlua::Result<bool> {
    for value in registry_list(lua, LOAD_STACK_KEY)?.sequence_values::<u64>() {
        if value? == scope.get() {
            return Ok(true);
        }
    }
    Ok(false)
}

/// Resolve one load request through the same provenance resolution as the host
/// package API: only byte loading and attribution differ between sources.
fn resolve_request(request: &Table) -> mlua::Result<(String, String)> {
    let path = request.get::<Option<String>>("path")?;
    let source = request.get::<Option<String>>("source")?;
    let resolved = match (path, source) {
        (Some(path), None) => {
            if path.trim().is_empty() {
                return Err(mlua::Error::runtime(
                    "packages.v1.load path must be a non-empty string",
                ));
            }
            crate::PackageSource::File {
                path: std::path::Path::new(&path),
            }
            .resolve()
        }
        (None, Some(source)) => {
            let name = request
                .get::<Option<String>>("name")?
                .filter(|value| !value.trim().is_empty())
                .ok_or_else(|| {
                    mlua::Error::runtime(
                        "packages.v1.load requires a non-empty name with an inline source",
                    )
                })?;
            crate::PackageSource::Memory {
                key: &name,
                source: &source,
            }
            .resolve()
        }
        _ => {
            return Err(mlua::Error::runtime(
                "packages.v1.load requires exactly one of path or (name, source)",
            ));
        }
    }
    .map_err(mlua::Error::external)?;
    if resolved.source.len() > MAX_SOURCE_BYTES {
        return Err(mlua::Error::runtime(format!(
            "package source {:?} exceeds {MAX_SOURCE_BYTES} bytes",
            resolved.source_key
        )));
    }
    Ok((resolved.source_key, resolved.source))
}

/// Dispose one recorded package. Already-disposed packages are a no-op, so a
/// cascade and an explicit `dispose` may both run.
async fn dispose_package(
    lua: &Lua,
    control: &Arc<Control>,
    effects: &EffectHub,
    scope: ScopeId,
) -> mlua::Result<bool> {
    let Some(entry) = active_entry(lua, scope)? else {
        return Ok(false);
    };
    if crate::kernel_api::current_scope_id(lua) == Some(scope) {
        return Err(mlua::Error::runtime(
            "a package cannot dispose the package that is currently running",
        ));
    }
    if loading(lua, scope)? {
        return Err(mlua::Error::runtime(
            "a package cannot be disposed while it is still loading",
        ));
    }
    entry.set("active", false)?;
    crate::vm::dispose_nested(lua, control, effects, scope)
        .await
        .map_err(mlua::Error::external)?;
    Ok(true)
}

pub(crate) fn install(
    lua: &Lua,
    pi: &Table,
    control: Arc<Control>,
    effects: EffectHub,
) -> mlua::Result<()> {
    let registry = crate::api::registry_table(lua)?;
    registry.set(PACKAGES_KEY, lua.create_table()?)?;
    registry.set(LOAD_STACK_KEY, lua.create_table()?)?;

    let v1 = lua.create_table()?;
    v1.set("api_version", PACKAGES_API_VERSION)?;
    v1.set("max_depth", MAX_LOAD_DEPTH)?;
    v1.set("max_packages", MAX_PACKAGES)?;
    v1.set("max_source_bytes", MAX_SOURCE_BYTES)?;

    let load_control = Arc::clone(&control);
    let load_effects = effects.clone();
    v1.set(
        "load",
        lua.create_async_function(move |lua, request: Table| {
            let control = Arc::clone(&load_control);
            let effects = load_effects.clone();
            async move {
                let (source_key, source) = resolve_request(&request)?;
                let stack = registry_list(&lua, LOAD_STACK_KEY)?;
                if stack.raw_len() as usize >= MAX_LOAD_DEPTH {
                    return Err(mlua::Error::runtime(format!(
                        "nested package load exceeds depth {MAX_LOAD_DEPTH}"
                    )));
                }
                prune_disposed(&lua)?;
                let list = registry_list(&lua, PACKAGES_KEY)?;
                if list.raw_len() as usize >= MAX_PACKAGES {
                    return Err(mlua::Error::runtime(format!(
                        "at most {MAX_PACKAGES} packages may be loaded from Lua at once"
                    )));
                }
                let owner = crate::kernel_api::current_scope_id(&lua);
                stack.push(owner.map_or(0, ScopeId::get))?;
                let loaded =
                    crate::vm::load_nested(&lua, &control, &effects, &source_key, &source).await;
                let depth = stack.raw_len();
                if depth > 0 {
                    stack.raw_set(depth, Value::Nil)?;
                }
                let scope = loaded.map_err(mlua::Error::external)?;

                let entry = lua.create_table()?;
                entry.set("source", source_key.as_str())?;
                entry.set("scope", scope.get())?;
                entry.set("owner", owner.map_or(0, ScopeId::get))?;
                entry.set("active", true)?;
                // Re-read the record: a nested load may have replaced it while
                // this load was suspended.
                registry_list(&lua, PACKAGES_KEY)?.push(entry)?;

                // The composed package is one disposable resource of its
                // loader: disposing the loader disposes it, transitively.
                if owner.is_some() {
                    let disposer_control = Arc::clone(&control);
                    let disposer_effects = effects.clone();
                    let callback = lua.create_async_function(move |lua, ()| {
                        let control = Arc::clone(&disposer_control);
                        let effects = disposer_effects.clone();
                        async move {
                            dispose_package(&lua, &control, &effects, scope)
                                .await
                                .map(|_| ())
                        }
                    })?;
                    crate::kernel_api::register_scoped_resource(&lua, &control, callback)?;
                }

                lua.create_userdata(LuaPackage {
                    source: source_key,
                    scope,
                    control: Arc::clone(&control),
                    effects: effects.clone(),
                })
            }
        })?,
    )?;

    v1.set(
        "list",
        lua.create_function(|lua, ()| {
            let result = lua.create_table()?;
            for entry in registry_list(lua, PACKAGES_KEY)?.sequence_values::<Table>() {
                let entry = entry?;
                if !entry.get::<bool>("active")? {
                    continue;
                }
                let item = lua.create_table()?;
                item.set("source", entry.get::<String>("source")?)?;
                item.set("scope", entry.get::<u64>("scope")?)?;
                item.set("owner", entry.get::<u64>("owner")?)?;
                result.push(item)?;
            }
            Ok(result)
        })?,
    )?;

    let packages = lua.create_table()?;
    packages.set("v1", v1)?;
    pi.set("packages", packages)
}
