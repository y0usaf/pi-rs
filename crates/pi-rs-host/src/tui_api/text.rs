//! Bounded Unicode measurement for Lua-authored terminal layout.
//!
//! Every member reports what the retained display would paint; none of them
//! decides what to paint. Widths, wrap points, and truncation come from the
//! same traversal the rasterizer uses, so a package that measures a string and
//! then submits it in a text node sees the two agree.

use pi_rs_tui::display::{self, WrapMode};

const MAX_TEXT_BYTES: usize = 1_048_576;
const DEFAULT_MAX_GRAPHEMES: usize = 1_024;
const MAX_GRAPHEMES: usize = 16_384;
const DEFAULT_MAX_ROWS: usize = 1_024;
const MAX_ROWS: usize = 16_384;

fn check_bytes(member: &str, text: &str) -> mlua::Result<()> {
    if text.len() > MAX_TEXT_BYTES {
        return Err(mlua::Error::runtime(format!(
            "terminal.v1.text.{member} received {} bytes; limit is {MAX_TEXT_BYTES}",
            text.len()
        )));
    }
    Ok(())
}

fn field<T: mlua::FromLua>(options: Option<&mlua::Table>, name: &str) -> mlua::Result<Option<T>> {
    options
        .map(|options| options.get::<Option<T>>(name))
        .transpose()
        .map(Option::flatten)
}

fn layout_width(member: &str, options: Option<&mlua::Table>) -> mlua::Result<u16> {
    field::<u16>(options, "width")?
        .ok_or_else(|| mlua::Error::runtime(format!("terminal.v1.text.{member} requires a width")))
}

fn tab_width(options: Option<&mlua::Table>) -> mlua::Result<u8> {
    Ok(field::<u8>(options, "tab_width")?.unwrap_or(4))
}

fn wrap_mode(options: Option<&mlua::Table>) -> mlua::Result<WrapMode> {
    match field::<String>(options, "wrap")?
        .as_deref()
        .unwrap_or("grapheme")
    {
        "grapheme" => Ok(WrapMode::Grapheme),
        "clip" => Ok(WrapMode::Clip),
        other => Err(mlua::Error::runtime(format!(
            "terminal.v1.text.measure wrap must be grapheme or clip, got {other}"
        ))),
    }
}

fn bounded(member: &str, name: &str, value: usize, max: usize) -> mlua::Result<usize> {
    if !(1..=max).contains(&value) {
        return Err(mlua::Error::runtime(format!(
            "terminal.v1.text.{member} {name} must be in 1..={max}"
        )));
    }
    Ok(value)
}

pub(crate) fn install(lua: &mlua::Lua, v1: &mlua::Table) -> mlua::Result<()> {
    let text = lua.create_table()?;
    text.set("max_bytes", MAX_TEXT_BYTES)?;
    text.set("default_max_graphemes", DEFAULT_MAX_GRAPHEMES)?;
    text.set("max_graphemes", MAX_GRAPHEMES)?;
    text.set("default_max_rows", DEFAULT_MAX_ROWS)?;
    text.set("max_rows", MAX_ROWS)?;

    text.set(
        "width",
        lua.create_function(|_, value: String| {
            check_bytes("width", &value)?;
            display::text_width(&value).map_err(mlua::Error::external)
        })?,
    )?;

    text.set(
        "measure",
        lua.create_function(|lua, (value, options): (String, Option<mlua::Table>)| {
            check_bytes("measure", &value)?;
            let metrics = display::measure_text(
                &value,
                layout_width("measure", options.as_ref())?,
                wrap_mode(options.as_ref())?,
                tab_width(options.as_ref())?,
            )
            .map_err(mlua::Error::external)?;
            let result = lua.create_table()?;
            result.set("rows", metrics.rows)?;
            result.set("max_width", metrics.max_width)?;
            result.set("last_width", metrics.last_width)?;
            result.set("cells", metrics.cells)?;
            Ok(result)
        })?,
    )?;

    text.set(
        "wrap",
        lua.create_function(|lua, (value, options): (String, Option<mlua::Table>)| {
            check_bytes("wrap", &value)?;
            let limit = bounded(
                "wrap",
                "limit",
                field::<usize>(options.as_ref(), "limit")?.unwrap_or(DEFAULT_MAX_ROWS),
                MAX_ROWS,
            )?;
            let (rows, overflow) = display::wrap_text(
                &value,
                layout_width("wrap", options.as_ref())?,
                tab_width(options.as_ref())?,
                limit,
            )
            .map_err(mlua::Error::external)?;
            let result = lua.create_table()?;
            for row in rows {
                result.push(row)?;
            }
            Ok((result, overflow))
        })?,
    )?;

    text.set(
        "truncate",
        lua.create_function(|_, (value, options): (String, Option<mlua::Table>)| {
            check_bytes("truncate", &value)?;
            let ellipsis = field::<String>(options.as_ref(), "ellipsis")?.unwrap_or_default();
            check_bytes("truncate", &ellipsis)?;
            let truncated = display::truncate_text(
                &value,
                layout_width("truncate", options.as_ref())?,
                &ellipsis,
            )
            .map_err(mlua::Error::external)?;
            Ok((truncated.text, truncated.width, truncated.truncated))
        })?,
    )?;

    text.set(
        "graphemes",
        lua.create_function(|lua, (value, options): (String, Option<mlua::Table>)| {
            check_bytes("graphemes", &value)?;
            let offset = field::<usize>(options.as_ref(), "offset")?.unwrap_or(0);
            let limit = bounded(
                "graphemes",
                "limit",
                field::<usize>(options.as_ref(), "limit")?.unwrap_or(DEFAULT_MAX_GRAPHEMES),
                MAX_GRAPHEMES,
            )?;
            let (window, total) =
                display::text_graphemes(&value, offset, limit).map_err(mlua::Error::external)?;
            let result = lua.create_table()?;
            for cell in window {
                let entry = lua.create_table()?;
                // One-based so `string.sub(source, entry.byte, ...)` is direct.
                entry.set("byte", cell.offset.saturating_add(1))?;
                entry.set("width", cell.width)?;
                entry.set("text", cell.text)?;
                result.push(entry)?;
            }
            Ok((result, total))
        })?,
    )?;

    v1.set("text", text)
}
