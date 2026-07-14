//! Versioned terminal-input and retained-display bindings for the coding spine.

pub(crate) fn install(lua: &mlua::Lua, pi: &mlua::Table) -> mlua::Result<()> {
    let v1 = lua.create_table()?;
    v1.set("api_version", 1_u32)?;
    v1.set(
        "display_schema_version",
        pi_rs_tui::display::DISPLAY_SCHEMA_VERSION,
    )?;
    v1.set(
        "input_buffer",
        lua.create_function(|lua, ()| {
            lua.create_userdata(crate::tui_api::runtime::LuaStdinBuffer(
                pi_rs_tui::stdin_buffer::StdinBuffer::new(),
            ))
        })?,
    )?;
    v1.set(
        "display",
        lua.create_function(|lua, limits: Option<mlua::Table>| {
            lua.create_userdata(crate::tui_api::runtime::LuaRetainedDisplay::new(limits)?)
        })?,
    )?;

    let terminal = lua.create_table()?;
    terminal.set("v1", v1)?;
    pi.set("terminal", terminal)
}
