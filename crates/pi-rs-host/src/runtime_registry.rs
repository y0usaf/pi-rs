//! VM-local attribution and registry access shared by independent bindings.
//!
//! This is deliberately limited to registry ownership primitives. Package,
//! declaration, module, and mechanism bindings keep their own state schemas.

/// Key of the host registration table in the Lua named registry.
pub(crate) const REGISTRY_KEY: &str = "pi-rs-host";

pub(crate) fn registry_table(lua: &mlua::Lua) -> mlua::Result<mlua::Table> {
    lua.named_registry_value::<mlua::Table>(REGISTRY_KEY)
}

pub(crate) fn current_source(lua: &mlua::Lua) -> String {
    registry_table(lua)
        .and_then(|table| table.get::<String>("source"))
        .unwrap_or_else(|_| "<unknown>".to_owned())
}

pub(crate) fn set_current_source(lua: &mlua::Lua, source: &str) {
    if let Ok(registry) = registry_table(lua) {
        let _ = registry.set("source", source);
    }
}
