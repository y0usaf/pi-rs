use mlua::{UserData, UserDataMethods};
use pi_rs_tui::display::{
    CellStyle, Color, CursorMetadata, CursorShape, DisplayBatch, DisplayLimits, DisplayNode,
    DisplayNodeContent, IdentityDelta, NodeId, Rect, RetainedDisplay, SubmitResult, TextRun,
    Viewport, WrapMode,
};

fn parse_color(value: mlua::Value) -> mlua::Result<Color> {
    match value {
        mlua::Value::Nil => Ok(Color::Default),
        mlua::Value::String(value) if value.to_str()?.as_ref() == "default" => Ok(Color::Default),
        mlua::Value::Table(value) => {
            if let Some(indexed) = value.get::<Option<u8>>("indexed")? {
                return Ok(Color::Indexed(indexed));
            }
            Ok(Color::Rgb {
                red: value.get("red")?,
                green: value.get("green")?,
                blue: value.get("blue")?,
            })
        }
        _ => Err(mlua::Error::runtime(
            "display color must be 'default' or an indexed/RGB table",
        )),
    }
}

fn parse_style(table: Option<mlua::Table>) -> mlua::Result<CellStyle> {
    let Some(table) = table else {
        return Ok(CellStyle::default());
    };
    Ok(CellStyle {
        foreground: parse_color(table.get::<mlua::Value>("foreground")?)?,
        background: parse_color(table.get::<mlua::Value>("background")?)?,
        bold: table.get::<Option<bool>>("bold")?.unwrap_or(false),
        dim: table.get::<Option<bool>>("dim")?.unwrap_or(false),
        italic: table.get::<Option<bool>>("italic")?.unwrap_or(false),
        underline: table.get::<Option<bool>>("underline")?.unwrap_or(false),
        reverse: table.get::<Option<bool>>("reverse")?.unwrap_or(false),
    })
}

fn parse_rect(table: mlua::Table) -> mlua::Result<Rect> {
    Ok(Rect {
        x: table.get("x")?,
        y: table.get("y")?,
        width: table.get("width")?,
        height: table.get("height")?,
    })
}

fn parse_node(table: mlua::Table) -> mlua::Result<DisplayNode> {
    let content: mlua::Table = table.get("content")?;
    let content = match content.get::<String>("kind")?.as_str() {
        "group" => DisplayNodeContent::Group,
        "text" => {
            let mut runs = Vec::new();
            if let Some(values) = content.get::<Option<mlua::Table>>("runs")? {
                for value in values.sequence_values::<mlua::Table>() {
                    let value = value?;
                    runs.push(TextRun {
                        text: value.get("text")?,
                        style: parse_style(value.get("style")?)?,
                    });
                }
            }
            let wrap = match content
                .get::<Option<String>>("wrap")?
                .as_deref()
                .unwrap_or("grapheme")
            {
                "grapheme" => WrapMode::Grapheme,
                "clip" => WrapMode::Clip,
                _ => {
                    return Err(mlua::Error::runtime(
                        "display text wrap must be grapheme or clip",
                    ));
                }
            };
            DisplayNodeContent::Text {
                runs,
                wrap,
                tab_width: content.get::<Option<u8>>("tab_width")?.unwrap_or(4),
            }
        }
        _ => {
            return Err(mlua::Error::runtime(
                "display node content kind must be group or text",
            ));
        }
    };
    let children = table
        .get::<Option<mlua::Table>>("children")?
        .map(|children| {
            children
                .sequence_values::<u64>()
                .map(|id| id.map(NodeId))
                .collect()
        })
        .transpose()?
        .unwrap_or_default();
    Ok(DisplayNode {
        id: NodeId(table.get("id")?),
        rect: parse_rect(table.get("rect")?)?,
        clip_children: table.get::<Option<bool>>("clip_children")?.unwrap_or(false),
        focusable: table.get::<Option<bool>>("focusable")?.unwrap_or(false),
        content,
        children,
    })
}

fn parse_display_batch(table: mlua::Table) -> mlua::Result<DisplayBatch> {
    let viewport: mlua::Table = table.get("viewport")?;
    let mut nodes = Vec::new();
    for node in table.get::<mlua::Table>("nodes")?.sequence_values() {
        nodes.push(parse_node(node?)?);
    }
    let cursor = table
        .get::<Option<mlua::Table>>("cursor")?
        .map(|cursor| {
            let shape = match cursor
                .get::<Option<String>>("shape")?
                .as_deref()
                .unwrap_or("block")
            {
                "block" => CursorShape::Block,
                "bar" => CursorShape::Bar,
                "underline" => CursorShape::Underline,
                _ => {
                    return Err(mlua::Error::runtime(
                        "display cursor shape must be block, bar, or underline",
                    ));
                }
            };
            Ok(CursorMetadata {
                node: NodeId(cursor.get("node")?),
                row: cursor.get("row")?,
                column: cursor.get("column")?,
                shape,
                visible: cursor.get::<Option<bool>>("visible")?.unwrap_or(true),
            })
        })
        .transpose()?;
    Ok(DisplayBatch {
        version: table.get("version")?,
        viewport: Viewport {
            columns: viewport.get("columns")?,
            rows: viewport.get("rows")?,
        },
        root: NodeId(table.get("root")?),
        nodes,
        focused: table.get::<Option<u64>>("focused")?.map(NodeId),
        cursor,
    })
}

fn node_ids(lua: &mlua::Lua, ids: Vec<NodeId>) -> mlua::Result<mlua::Table> {
    let result = lua.create_table()?;
    for id in ids {
        result.push(id.0)?;
    }
    Ok(result)
}

fn identity_delta(lua: &mlua::Lua, value: IdentityDelta) -> mlua::Result<mlua::Table> {
    let result = lua.create_table()?;
    result.set("added", node_ids(lua, value.added)?)?;
    result.set("changed", node_ids(lua, value.changed)?)?;
    result.set("retained", node_ids(lua, value.retained)?)?;
    result.set("removed", node_ids(lua, value.removed)?)?;
    Ok(result)
}

fn submit_result(lua: &mlua::Lua, value: SubmitResult) -> mlua::Result<mlua::Table> {
    let result = lua.create_table()?;
    result.set("revision", value.revision)?;
    result.set("ansi", lua.create_string(value.ansi)?)?;
    result.set("identities", identity_delta(lua, value.identities)?)?;
    result.set("visited_nodes", value.visited_nodes)?;
    result.set("painted_cells", value.painted_cells)?;
    result.set("changed_cells", value.changed_cells)?;
    result.set("full_redraw", value.full_redraw)?;
    Ok(result)
}

fn parse_limits(table: Option<mlua::Table>) -> mlua::Result<DisplayLimits> {
    let defaults = DisplayLimits::default();
    let Some(table) = table else {
        return Ok(defaults);
    };
    Ok(DisplayLimits {
        max_nodes: table
            .get::<Option<usize>>("max_nodes")?
            .unwrap_or(defaults.max_nodes),
        max_depth: table
            .get::<Option<usize>>("max_depth")?
            .unwrap_or(defaults.max_depth),
        max_children_per_node: table
            .get::<Option<usize>>("max_children_per_node")?
            .unwrap_or(defaults.max_children_per_node),
        max_text_runs: table
            .get::<Option<usize>>("max_text_runs")?
            .unwrap_or(defaults.max_text_runs),
        max_text_bytes: table
            .get::<Option<usize>>("max_text_bytes")?
            .unwrap_or(defaults.max_text_bytes),
        max_cells: table
            .get::<Option<usize>>("max_cells")?
            .unwrap_or(defaults.max_cells),
    })
}

pub(crate) struct LuaRetainedDisplay(RetainedDisplay);

impl LuaRetainedDisplay {
    pub(crate) fn new(limits: Option<mlua::Table>) -> mlua::Result<Self> {
        Ok(Self(RetainedDisplay::new(parse_limits(limits)?)))
    }
}

impl UserData for LuaRetainedDisplay {
    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        methods.add_method_mut("submit", |lua, this, batch: mlua::Table| {
            submit_result(
                lua,
                this.0
                    .submit(parse_display_batch(batch)?)
                    .map_err(mlua::Error::external)?,
            )
        });
        methods.add_method("revision", |_, this, ()| Ok(this.0.revision()));
        methods.add_method_mut("reset_presentation", |_, this, ()| {
            this.0.reset_presentation();
            Ok(())
        });
    }
}

fn stdin_events(
    lua: &mlua::Lua,
    events: Vec<pi_rs_tui::stdin_buffer::StdinEvent>,
) -> mlua::Result<mlua::Table> {
    let result = lua.create_table()?;
    for event in events {
        let value = lua.create_table()?;
        match event {
            pi_rs_tui::stdin_buffer::StdinEvent::Data(data) => {
                value.set("kind", "data")?;
                value.set("data", data)?;
            }
            pi_rs_tui::stdin_buffer::StdinEvent::Paste(data) => {
                value.set("kind", "paste")?;
                value.set("data", data)?;
            }
        }
        result.push(value)?;
    }
    Ok(result)
}

pub(crate) struct LuaStdinBuffer(pub(crate) pi_rs_tui::stdin_buffer::StdinBuffer);

impl UserData for LuaStdinBuffer {
    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        methods.add_method_mut("feed", |lua, this, data: mlua::String| {
            stdin_events(
                lua,
                this.0
                    .try_process_bytes(&data.as_bytes())
                    .map_err(mlua::Error::external)?,
            )
        });
        methods.add_method_mut("flush", |lua, this, ()| {
            stdin_events(lua, this.0.try_flush().map_err(mlua::Error::external)?)
        });
        methods.add_method_mut("clear", |_, this, ()| {
            this.0.clear();
            Ok(())
        });
        methods.add_method("buffer", |lua, this, ()| {
            lua.create_string(this.0.buffered())
        });
    }
}
