//! Narrow terminal/display mechanism bindings.
//!
//! Lua submits complete versioned retained-display batches. Product component,
//! editor, transcript, dialog, selector, and chrome policy is intentionally not
//! represented here.

pub(crate) mod runtime;

use mlua::FromLua;
use runtime::*;

pub(crate) fn install(lua: &mlua::Lua, pi: &mlua::Table) -> mlua::Result<()> {
    let tui = lua.create_table()?;
    tui.set(
        "display_schema_version",
        pi_rs_tui::display::DISPLAY_SCHEMA_VERSION,
    )?;
    tui.set(
        "display",
        lua.create_function(|lua, limits: Option<mlua::Table>| {
            lua.create_userdata(LuaRetainedDisplay::new(limits)?)
        })?,
    )?;
    tui.set(
        "stdin_buffer",
        lua.create_function(|lua, ()| {
            lua.create_userdata(LuaStdinBuffer(pi_rs_tui::stdin_buffer::StdinBuffer::new()))
        })?,
    )?;
    tui.set(
        "terminal",
        lua.create_function(|lua, (columns, rows): (Option<u16>, Option<u16>)| {
            lua.create_userdata(LuaTerminal(pi_rs_tui::terminal::TerminalState::new(
                columns, rows,
            )))
        })?,
    )?;
    tui.set(
        "display_process",
        lua.create_function(|lua, ()| {
            lua.create_userdata(LuaDisplayProcess(pi_rs_tui::process::DisplayProcess::new()))
        })?,
    )?;
    tui.set(
        "visible_width",
        lua.create_function(|_, text: String| Ok(pi_rs_tui::utils::visible_width(&text)))?,
    )?;
    tui.set(
        "truncate",
        lua.create_function(
            |_, (text, width, ellipsis, pad): (String, usize, Option<String>, Option<bool>)| {
                Ok(pi_rs_tui::utils::truncate_to_width(
                    &text,
                    width,
                    ellipsis.as_deref().unwrap_or("..."),
                    pad.unwrap_or(false),
                ))
            },
        )?,
    )?;
    tui.set(
        "slice_by_column",
        lua.create_function(|_, (text, start, width): (String, usize, usize)| {
            Ok(pi_rs_tui::utils::slice_by_column(&text, start, width, true))
        })?,
    )?;
    tui.set(
        "terminal_capabilities",
        lua.create_function(|lua, ()| {
            let caps = pi_rs_tui::terminal_image::get_capabilities();
            let result = lua.create_table()?;
            result.set(
                "images",
                caps.images.map(|protocol| match protocol {
                    pi_rs_tui::terminal_image::ImageProtocol::Kitty => "kitty",
                    pi_rs_tui::terminal_image::ImageProtocol::ITerm2 => "iterm2",
                }),
            )?;
            result.set("true_color", caps.true_color)?;
            result.set("hyperlinks", caps.hyperlinks)?;
            Ok(result)
        })?,
    )?;
    tui.set(
        "image_dimensions",
        lua.create_function(|lua, (data, mime_type): (String, String)| {
            let Some(dimensions) =
                pi_rs_tui::terminal_image::get_image_dimensions(&data, &mime_type)
            else {
                return Ok(mlua::Value::Nil);
            };
            let result = lua.create_table()?;
            result.set("width_px", dimensions.width_px)?;
            result.set("height_px", dimensions.height_px)?;
            Ok(mlua::Value::Table(result))
        })?,
    )?;
    tui.set(
        "image_render",
        lua.create_function(
            |lua,
             (protocol, data, dimensions, options): (
                String,
                String,
                mlua::Table,
                Option<mlua::Table>,
            )| {
                let protocol = match protocol.as_str() {
                    "kitty" => pi_rs_tui::terminal_image::ImageProtocol::Kitty,
                    "iterm2" => pi_rs_tui::terminal_image::ImageProtocol::ITerm2,
                    _ => {
                        return Err(mlua::Error::runtime(
                            "image_render: protocol must be kitty or iterm2",
                        ));
                    }
                };
                let option = |key: &str| -> mlua::Result<mlua::Value> {
                    options
                        .as_ref()
                        .map(|table| table.get(key))
                        .transpose()
                        .map(|value| value.unwrap_or(mlua::Value::Nil))
                };
                let max_width_cells = match option("max_width_cells")? {
                    mlua::Value::Nil => None,
                    value => Some(u32::from_lua(value, lua)?),
                };
                let max_height_cells = match option("max_height_cells")? {
                    mlua::Value::Nil => None,
                    value => Some(u32::from_lua(value, lua)?),
                };
                let preserve_aspect_ratio = match option("preserve_aspect_ratio")? {
                    mlua::Value::Nil => None,
                    value => Some(bool::from_lua(value, lua)?),
                };
                let image_id = match option("image_id")? {
                    mlua::Value::Nil => None,
                    value => Some(u32::from_lua(value, lua)?),
                };
                let move_cursor = match option("move_cursor")? {
                    mlua::Value::Nil => None,
                    value => Some(bool::from_lua(value, lua)?),
                };
                let rendered = pi_rs_tui::terminal_image::render_image_with_protocol(
                    protocol,
                    &data,
                    pi_rs_tui::terminal_image::ImageDimensions {
                        width_px: dimensions.get("width_px")?,
                        height_px: dimensions.get("height_px")?,
                    },
                    pi_rs_tui::terminal_image::ImageRenderOptions {
                        max_width_cells,
                        max_height_cells,
                        preserve_aspect_ratio,
                        image_id,
                        move_cursor,
                    },
                );
                let result = lua.create_table()?;
                result.set("sequence", rendered.sequence)?;
                result.set("rows", rendered.rows)?;
                result.set("image_id", rendered.image_id)?;
                Ok(result)
            },
        )?,
    )?;
    tui.set(
        "is_image_line",
        lua.create_function(|_, line: String| Ok(pi_rs_tui::terminal_image::is_image_line(&line)))?,
    )?;
    tui.set(
        "image_fallback",
        lua.create_function(
            |_,
             (mime_type, width, height, filename): (
                String,
                Option<u32>,
                Option<u32>,
                Option<String>,
            )| {
                let dimensions = width.zip(height).map(|(width_px, height_px)| {
                    pi_rs_tui::terminal_image::ImageDimensions {
                        width_px,
                        height_px,
                    }
                });
                Ok(pi_rs_tui::terminal_image::image_fallback(
                    &mime_type,
                    dimensions,
                    filename.as_deref(),
                ))
            },
        )?,
    )?;
    tui.set(
        "hyperlink",
        lua.create_function(|_, (text, url): (String, String)| {
            Ok(pi_rs_tui::terminal_image::hyperlink(&text, &url))
        })?,
    )?;
    tui.set(
        "delete_kitty_image",
        lua.create_function(|_, image_id: u32| {
            Ok(pi_rs_tui::terminal_image::delete_kitty_image(image_id))
        })?,
    )?;
    tui.set(
        "delete_all_kitty_images",
        lua.create_function(|_, ()| Ok(pi_rs_tui::terminal_image::delete_all_kitty_images()))?,
    )?;
    pi.set("tui", tui)?;
    Ok(())
}
