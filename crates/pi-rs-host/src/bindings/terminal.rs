//! Versioned terminal-input and retained-display bindings for the coding spine.

pub(crate) fn install(lua: &mlua::Lua, pi: &mlua::Table) -> mlua::Result<()> {
    let bridge = lua.create_table()?;
    crate::tui_api::install(lua, &bridge)?;
    let mechanisms: mlua::Table = bridge.get("tui")?;

    let v1 = lua.create_table()?;
    v1.set("api_version", 1_u32)?;
    v1.set(
        "display_schema_version",
        mechanisms.get::<u32>("display_schema_version")?,
    )?;
    v1.set(
        "input_buffer",
        mechanisms.get::<mlua::Function>("stdin_buffer")?,
    )?;
    v1.set("display", mechanisms.get::<mlua::Function>("display")?)?;

    let terminal = lua.create_table()?;
    terminal.set("v1", v1)?;
    pi.set("terminal", terminal)
}
