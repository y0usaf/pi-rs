//! Assembly of the compact, versioned Lua mechanism surface.

mod effects;
mod models;
mod packages;
mod records;
mod roots;
mod terminal;

use crate::runtime_registry::REGISTRY_KEY;

/// Registry key holding the one mechanism table handed to every package.
const MECHANISM_KEY: &str = "mechanism";

/// The `pi` table every package receives, whatever its provenance.
pub(crate) fn mechanism_table(lua: &mlua::Lua) -> Result<mlua::Table, crate::HostError> {
    crate::api::registry_table(lua)
        .and_then(|registry| registry.get::<mlua::Table>(MECHANISM_KEY))
        .map_err(|error| crate::HostError::Lua(error.to_string()))
}

pub(crate) fn build(
    lua: &mlua::Lua,
    cwd: &str,
    config: crate::HostConfig,
    control: std::sync::Arc<crate::kernel::Control>,
    effects: crate::effects::EffectHub,
) -> mlua::Result<mlua::Table> {
    let registry = lua.create_table()?;
    registry.set("modules", lua.create_table()?)?;
    registry.set("module_order", lua.create_table()?)?;
    registry.set("module_stack", lua.create_table()?)?;
    registry.set("source", "<host>")?;
    lua.set_named_registry_value(REGISTRY_KEY, &registry)?;

    // The environment crosses as one immutable startup snapshot: Lua reads
    // names and values from it, so no dispatch observes a mutating process
    // environment and no policy default lives in Rust.
    let environment = config
        .environment
        .clone()
        .unwrap_or_else(|| std::env::vars().collect());
    let pi = lua.create_table()?;
    let module_api = crate::module_api::install(lua)?;
    crate::kernel_api::install(lua, &pi, &module_api, control.clone())?;
    records::install(lua, &pi, control.clone())?;
    packages::install(lua, &pi, control.clone(), effects.clone())?;
    roots::install(lua, &pi, control, config)?;
    crate::middleware::install(lua, &pi)?;
    terminal::install(lua, &pi)?;
    models::install(lua, &pi)?;
    effects::install(lua, &pi, cwd, effects, environment)?;
    registry.set(MECHANISM_KEY, pi.clone())?;
    Ok(pi)
}
