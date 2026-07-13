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

pub(crate) fn parse_display_batch(table: mlua::Table) -> mlua::Result<DisplayBatch> {
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

pub(crate) struct LuaTerminal(pub(crate) pi_rs_tui::terminal::TerminalState);
impl UserData for LuaTerminal {
    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        methods.add_method_mut("start", |_, this, ()| {
            this.0.start();
            Ok(())
        });
        methods.add_method_mut("feed", |lua, this, data: mlua::String| {
            let events = this
                .0
                .try_feed_input(&data.as_bytes())
                .map_err(mlua::Error::external)?;
            crate::bindings::rendered_lines(lua, events)
        });
        methods.add_method_mut("flush", |lua, this, ()| {
            let events = this
                .0
                .try_flush_input()
                .map_err(mlua::Error::external)?
                .into_iter()
                .chain(this.0.flush_keyboard_negotiation())
                .collect();
            crate::bindings::rendered_lines(lua, events)
        });
        methods.add_method_mut("drain", |_, this, ()| {
            this.0.begin_drain();
            Ok(())
        });
        methods.add_method_mut("stop", |_, this, ()| {
            this.0.stop();
            Ok(())
        });
        methods.add_method_mut("output", |lua, this, ()| {
            lua.create_string(this.0.take_output())
        });
        methods.add_method("dimensions", |lua, this, ()| {
            let result = lua.create_table()?;
            result.set("columns", this.0.columns())?;
            result.set("rows", this.0.rows())?;
            Ok(result)
        });
        methods.add_method("protocol_flags", |lua, this, ()| {
            let result = lua.create_table()?;
            result.set("kitty", this.0.kitty_protocol_active())?;
            result.set("modify_other_keys", this.0.modify_other_keys_active())?;
            Ok(result)
        });
        methods.add_method_mut("resize", |_, this, (columns, rows)| {
            this.0.resize(columns, rows);
            Ok(())
        });
        methods.add_method_mut("write", |_, this, data: String| {
            this.0.write(&data);
            Ok(())
        });
        methods.add_method_mut("move", |_, this, lines: i32| {
            this.0.move_by(lines);
            Ok(())
        });
        methods.add_method_mut("cursor", |_, this, visible: bool| {
            if visible {
                this.0.show_cursor()
            } else {
                this.0.hide_cursor()
            }
            Ok(())
        });
        methods.add_method_mut("clear", |_, this, target: Option<String>| {
            match target.as_deref().unwrap_or("line") {
                "line" => this.0.clear_line(),
                "below" | "from_cursor" => this.0.clear_from_cursor(),
                "screen" => this.0.clear_screen(),
                _ => {
                    return Err(mlua::Error::runtime(
                        "terminal clear target must be line, below, or screen",
                    ));
                }
            }
            Ok(())
        });
        methods.add_method_mut("title", |_, this, title: String| {
            this.0.set_title(&title);
            Ok(())
        });
        methods.add_method_mut("progress", |_, this, active: bool| {
            this.0.set_progress(active);
            Ok(())
        });
        methods.add_method_mut("progress_keepalive", |_, this, ()| {
            this.0.progress_keepalive();
            Ok(())
        });
    }
}

/// Handle for a `pi.spawn` background coroutine.
pub(crate) struct LuaSpawnHandle(
    pub(crate) std::cell::RefCell<Option<tokio::task::JoinHandle<mlua::Result<mlua::Value>>>>,
);

impl UserData for LuaSpawnHandle {
    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        methods.add_method("done", |_, this, ()| {
            Ok(this
                .0
                .borrow()
                .as_ref()
                .is_none_or(tokio::task::JoinHandle::is_finished))
        });
        methods.add_async_method("join", |_, this, ()| async move {
            let handle = this.0.borrow_mut().take();
            match handle {
                Some(handle) => match handle.await {
                    Ok(result) => result,
                    Err(join_error) => Err(mlua::Error::runtime(format!(
                        "spawned task failed: {join_error}"
                    ))),
                },
                None => Err(mlua::Error::runtime("spawn handle already joined")),
            }
        });
    }
}

pub(crate) struct LuaDisplayProcess(pub(crate) pi_rs_tui::process::DisplayProcess);

impl UserData for LuaDisplayProcess {
    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        methods.add_method("dimensions", |lua, this, ()| {
            let (columns, rows) = this.0.dimensions();
            let dimensions = lua.create_table()?;
            dimensions.set("columns", columns)?;
            dimensions.set("rows", rows)?;
            Ok(dimensions)
        });
        methods.add_async_method_mut(
            "run",
            |lua, mut this, callback: mlua::Function| async move {
                let exit = this
                    .0
                    .run(|event| {
                        let lua = lua.clone();
                        let callback = callback.clone();
                        async move {
                            let build = async {
                                let value = lua.create_table()?;
                                match event {
                                    pi_rs_tui::process::ProcessEvent::Start { columns, rows } => {
                                        value.set("type", "start")?;
                                        value.set("columns", columns)?;
                                        value.set("rows", rows)?;
                                    }
                                    pi_rs_tui::process::ProcessEvent::Input(data) => {
                                        value.set("type", "input")?;
                                        value.set("data", data)?;
                                    }
                                    pi_rs_tui::process::ProcessEvent::Resize { columns, rows } => {
                                        value.set("type", "resize")?;
                                        value.set("columns", columns)?;
                                        value.set("rows", rows)?;
                                    }
                                    pi_rs_tui::process::ProcessEvent::Tick => {
                                        value.set("type", "tick")?
                                    }
                                    pi_rs_tui::process::ProcessEvent::Signal(signal) => {
                                        value.set("type", "signal")?;
                                        value.set("signal", signal)?;
                                    }
                                    pi_rs_tui::process::ProcessEvent::InheritedProcessResult(
                                        result,
                                    ) => {
                                        value.set("type", "inherited_process_result")?;
                                        value.set("id", result.id)?;
                                        value.set("status", result.status)?;
                                    }
                                }
                                let control: Option<mlua::Table> =
                                    callback.call_async(value).await?;
                                let Some(control) = control else {
                                    return Ok(pi_rs_tui::process::ProcessControl::default());
                                };
                                let display = control
                                    .get::<Option<mlua::Table>>("display")?
                                    .map(parse_display_batch)
                                    .transpose()?;
                                let inherited_process = control
                                    .get::<Option<mlua::Table>>("inherited_process")?
                                    .map(|action| {
                                        let args = action
                                            .get::<Option<mlua::Table>>("args")?
                                            .map(|args| args.sequence_values().collect())
                                            .transpose()?
                                            .unwrap_or_default();
                                        Ok::<_, mlua::Error>(
                                            pi_rs_tui::process::InheritedProcessAction {
                                                id: action.get("id")?,
                                                program: action.get("program")?,
                                                args,
                                                shell: action
                                                    .get::<Option<bool>>("shell")?
                                                    .unwrap_or(false),
                                                message: action.get("message")?,
                                            },
                                        )
                                    })
                                    .transpose()?;
                                Ok(pi_rs_tui::process::ProcessControl {
                                    display,
                                    exit: control.get::<Option<bool>>("exit")?.unwrap_or(false),
                                    title: control.get("title")?,
                                    progress: control.get("progress")?,
                                    inherited_process,
                                    suspend: control
                                        .get::<Option<bool>>("suspend")?
                                        .unwrap_or(false),
                                })
                            }
                            .await;
                            build.map_err(|error: mlua::Error| {
                                pi_rs_tui::process::ProcessError::Callback(error.to_string())
                            })
                        }
                    })
                    .await
                    .map_err(mlua::Error::external)?;
                match exit {
                    pi_rs_tui::process::ProcessExit::Requested => Ok(("requested", None)),
                    pi_rs_tui::process::ProcessExit::Signal(signal) => Ok(("signal", Some(signal))),
                }
            },
        );
    }
}
