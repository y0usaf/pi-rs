use mlua::{UserData, UserDataMethods};

pub(crate) fn loader_indicator(
    table: Option<mlua::Table>,
) -> mlua::Result<Option<pi_rs_tui::loader::Indicator>> {
    table
        .map(|table| {
            let frames = table
                .get::<Option<mlua::Table>>("frames")?
                .map(|frames| {
                    frames
                        .sequence_values()
                        .collect::<mlua::Result<Vec<String>>>()
                })
                .transpose()?
                .unwrap_or_else(|| {
                    pi_rs_tui::loader::DEFAULT_FRAMES
                        .iter()
                        .map(|frame| (*frame).to_owned())
                        .collect()
                });
            Ok(pi_rs_tui::loader::Indicator {
                frames,
                interval_ms: table
                    .get::<Option<u64>>("interval_ms")?
                    .unwrap_or(pi_rs_tui::loader::DEFAULT_INTERVAL_MS),
            })
        })
        .transpose()
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
            stdin_events(lua, this.0.process_bytes(&data.as_bytes()))
        });
        methods.add_method_mut("flush", |lua, this, ()| stdin_events(lua, this.0.flush()));
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
            let result = lua.create_table()?;
            for event in this.0.feed_input(&data.as_bytes()) {
                result.push(event)?;
            }
            Ok(result)
        });
        // Preserve the existing deterministic Lua seam: `flush` represents
        // advancing both the stdin parser and negotiation deadlines.
        methods.add_method_mut("flush", |lua, this, ()| {
            let result = lua.create_table()?;
            for event in this
                .0
                .flush_input()
                .into_iter()
                .chain(this.0.flush_keyboard_negotiation())
            {
                result.push(event)?;
            }
            Ok(result)
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

/// Handle for a `pi.spawn` background coroutine. `join()` awaits the
/// task and returns its value (or re-raises its error); `done()` reports
/// completion without blocking.
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

pub(crate) struct LuaProcessTui(pub(crate) pi_rs_tui::process::ProcessTui);

impl UserData for LuaProcessTui {
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
                                let lines = control
                                    .get::<Option<mlua::Table>>("lines")?
                                    .map(|lines| lines.sequence_values().collect())
                                    .transpose()?;
                                let inherited_process = control
                                    .get::<Option<mlua::Table>>("inheritedProcess")?
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
                                    lines,
                                    force: control.get::<Option<bool>>("force")?.unwrap_or(false),
                                    exit: control.get::<Option<bool>>("exit")?.unwrap_or(false),
                                    title: control.get("title")?,
                                    progress: control.get("progress")?,
                                    show_hardware_cursor: control.get("showHardwareCursor")?,
                                    clear_on_shrink: control.get("clearOnShrink")?,
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

pub(crate) struct LuaTui(pub(crate) pi_rs_tui::tui::Tui);
impl UserData for LuaTui {
    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        methods.add_method_mut("start", |_, this, ()| {
            this.0.start();
            Ok(())
        });
        methods.add_method_mut("request_render", |_, this, force: Option<bool>| {
            this.0.request_render(force.unwrap_or(false));
            Ok(())
        });
        methods.add_method_mut("feed", |lua, this, data: mlua::String| {
            let result = lua.create_table()?;
            for event in this.0.feed_input(&data.as_bytes()) {
                result.push(event)?;
            }
            Ok(result)
        });
        methods.add_method_mut("flush", |lua, this, ()| {
            let result = lua.create_table()?;
            for event in this.0.flush_input() {
                result.push(event)?;
            }
            Ok(result)
        });
        methods.add_method_mut(
            "resize",
            |_, this, (columns, rows): (Option<u16>, Option<u16>)| {
                this.0.resize(columns, rows);
                Ok(())
            },
        );
        methods.add_method_mut("render", |_, this, lines: mlua::Table| {
            let lines = lines
                .sequence_values()
                .collect::<mlua::Result<Vec<String>>>()?;
            this.0
                .render_if_requested(lines)
                .map_err(mlua::Error::external)
        });
        methods.add_method_mut("stop", |_, this, ()| {
            this.0.stop();
            Ok(())
        });
        methods.add_method_mut("output", |lua, this, ()| {
            lua.create_string(this.0.take_output())
        });
        methods.add_method("full_redraws", |_, this, ()| Ok(this.0.full_redraws()));
    }
}

pub(crate) struct LuaLoader(pub(crate) pi_rs_tui::loader::Loader);
impl UserData for LuaLoader {
    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        methods.add_method_mut("start", |_, this, ()| {
            this.0.start();
            Ok(())
        });
        methods.add_method_mut("stop", |_, this, ()| {
            this.0.stop();
            Ok(())
        });
        methods.add_method_mut("advance", |_, this, elapsed_ms: u64| {
            Ok(this.0.advance(elapsed_ms))
        });
        methods.add_method_mut("set_message", |_, this, message: String| {
            this.0.set_message(message);
            Ok(())
        });
        methods.add_method("frame", |_, this, ()| Ok(this.0.frame().to_owned()));
        methods.add_method("running", |_, this, ()| Ok(this.0.running()));
        methods.add_method("render", |lua, this, width: usize| {
            let result = lua.create_table()?;
            for line in pi_rs_tui::component::Component::render(&this.0, width) {
                result.push(line)?;
            }
            Ok(result)
        });
    }
}

pub(crate) struct LuaCancellableLoader(pub(crate) pi_rs_tui::loader::CancellableLoader);
impl UserData for LuaCancellableLoader {
    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        methods.add_method_mut("advance", |_, this, elapsed_ms: u64| {
            Ok(this.0.loader_mut().advance(elapsed_ms))
        });
        methods.add_method_mut("input", |_, this, data: String| {
            Ok(this.0.handle_input(&data))
        });
        methods.add_method_mut("dispose", |_, this, ()| {
            this.0.dispose();
            Ok(())
        });
        methods.add_method("aborted", |_, this, ()| Ok(this.0.aborted()));
        methods.add_method("frame", |_, this, ()| {
            Ok(this.0.loader().frame().to_owned())
        });
        methods.add_method("render", |lua, this, width: usize| {
            let result = lua.create_table()?;
            for line in pi_rs_tui::component::Component::render(&this.0, width) {
                result.push(line)?;
            }
            Ok(result)
        });
    }
}

pub(crate) struct LuaSelectList(pub(crate) pi_rs_tui::select_list::SelectList);
impl UserData for LuaSelectList {
    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        methods.add_method_mut("filter", |_, this, filter: String| {
            this.0.set_filter(&filter);
            Ok(())
        });
        methods.add_method("selected", |lua, this, ()| {
            this.0
                .selected()
                .map(|item| {
                    let table = lua.create_table()?;
                    table.set("value", item.value.clone())?;
                    table.set("label", item.label.clone())?;
                    table.set("description", item.description.clone())?;
                    Ok(table)
                })
                .transpose()
        });
        methods.add_method_mut("set_selected_index", |_, this, index: usize| {
            this.0.set_selected_index(index);
            Ok(())
        });
        methods.add_method("render", |lua, this, width: usize| {
            let result = lua.create_table()?;
            for line in this.0.render(width) {
                result.push(line)?;
            }
            Ok(result)
        });
        methods.add_method_mut("input", |_, this, data: String| Ok(this.0.handle(&data)))
    }
}
