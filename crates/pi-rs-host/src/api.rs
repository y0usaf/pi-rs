//! The `pi` API table — the canonical extension surface, named after the
//! spec's `ExtensionAPI` parameter so example translations stay literal
//! (`export default function (pi) { ... }` → `local pi = ...`).
//!
//! Registration state lives Lua-side in a registry-held table (handlers
//! stay Lua functions); the host reads it at dispatch time. The table is
//! kept in the named registry, not a global, so extension code can't
//! clobber it by accident.
//!
//! Surface at WS1.3 (grown only alongside an exercising example):
//! - `pi.on(event, fn)` — open string vocabulary, no closed enum.
//! - `pi.sleep(ms, signal?)` — awaitable host timer, optionally abortable;
//!   suspends the handler coroutine without burning watchdog budget and ports
//!   Pi's `sleep(ms, signal)` cancellation seam.
//! - `pi.parallel(tasks)` — structured concurrency for awaitable Lua callbacks;
//!   completion-order outcomes let Lua policy reproduce Promise semantics.
//! - `pi.module.define/require/list` — exact-version Lua modules with sorted,
//!   declared dependencies, lazy cycle detection, source attribution, and one
//!   cache shared identically by embedded and file-backed packages.
//! - `pi.register_tool(def)` — spec `registerTool` (`loader.ts`):
//!   per-extension map, re-registration replaces in place.
//! - `pi.register_command(name, opts)` — spec `registerCommand`
//!   (`loader.ts`): `{ name, ...opts }` into the per-extension map.
//! - `pi.get_commands()` — spec `getCommands`: resolved extension-command
//!   snapshots; prompt/skill resource rows join with their Lua registries.
//! - `pi.register_role(def)` / `pi.registered_roles(role?)` — additive public
//!   application/frontend declarations with explicit activation + priority.
//! - `pi.declare_package(opts)` — source-neutral declaration defaults used by
//!   embedded and file-backed packages alike.
//! - `pi.register_provider(name, config)` / `pi.unregister_provider(name)`
//!   — spec `registerProvider`/`unregisterProvider`: registrations are
//!   queued host-side (the spec's initial-load behavior — the runner
//!   applies them once bound); re-registration merges defined keys over
//!   the stored config (spec `upsertRegisteredProvider`). Application to
//!   the model registry is the embedder's (pi-rs-app, WS2.6).
//! - `pi.exec(command, args?, options?)` — spec `ExtensionAPI.exec`
//!   (`exec.rs` ← `core/exec.ts`).
//! - `pi.fs` / `pi.path` / `pi.env` / `pi.cwd()` — OS bindings (`os.rs`;
//!   ambient Node in the spec, explicit bindings under divergence 1).
//! - `pi.http.get(url, options?)` — awaitable HTTP GET mechanism for Lua policy;
//!   endpoint choice and response interpretation remain in extensions.

use crate::runtime_registry::REGISTRY_KEY;
pub(crate) use crate::runtime_registry::{current_source, registry_table, set_current_source};
use mlua::{AnyUserData, UserData, UserDataMethods};
use std::{
    collections::{HashMap, HashSet},
    sync::{Arc, Mutex},
};
fn loader_indicator(
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

struct LuaStdinBuffer(pi_rs_tui::stdin_buffer::StdinBuffer);
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

struct LuaTerminal(pi_rs_tui::terminal::TerminalState);
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
struct LuaSpawnHandle(
    std::cell::RefCell<Option<tokio::task::JoinHandle<mlua::Result<mlua::Value>>>>,
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

struct LuaProcessTui(pi_rs_tui::process::ProcessTui);

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

struct LuaTui(pi_rs_tui::tui::Tui);
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

struct LuaLoader(pi_rs_tui::loader::Loader);
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

struct LuaCancellableLoader(pi_rs_tui::loader::CancellableLoader);
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

struct LuaSelectList(pi_rs_tui::select_list::SelectList);
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

/// `pi.tui.autocomplete_provider` — pi's `CombinedAutocompleteProvider` over
/// the Rust mechanism, with per-command `get_argument_completions` policy
/// callbacks staying in Lua.
struct LuaAutocompleteProvider {
    provider: pi_rs_tui::autocomplete::CombinedProvider,
    argument_completions: std::collections::HashMap<String, mlua::Function>,
}

fn autocomplete_items_table(
    lua: &mlua::Lua,
    items: &[pi_rs_tui::autocomplete::AutocompleteItem],
) -> mlua::Result<mlua::Table> {
    let table = lua.create_table()?;
    for item in items {
        let value = lua.create_table()?;
        value.set("value", item.value.clone())?;
        value.set("label", item.label.clone())?;
        value.set("description", item.description.clone())?;
        table.push(value)?;
    }
    Ok(table)
}

fn autocomplete_items_from_table(
    items: mlua::Table,
) -> mlua::Result<Vec<pi_rs_tui::autocomplete::AutocompleteItem>> {
    let mut parsed = Vec::new();
    for item in items.sequence_values::<mlua::Table>() {
        let item = item?;
        parsed.push(pi_rs_tui::autocomplete::AutocompleteItem {
            value: item.get("value")?,
            label: item.get("label")?,
            description: item.get("description")?,
        });
    }
    Ok(parsed)
}

impl UserData for LuaAutocompleteProvider {
    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        methods.add_method(
            "get_suggestions",
            |lua,
             this,
             (lines, cursor_line, cursor_col, opts): (
                mlua::Table,
                usize,
                usize,
                Option<mlua::Table>,
            )| {
                let lines: Vec<String> = lines.sequence_values().collect::<mlua::Result<_>>()?;
                let force = opts
                    .map(|t| t.get::<Option<bool>>("force"))
                    .transpose()?
                    .flatten()
                    .unwrap_or(false);
                let mut callback_error: Option<mlua::Error> = None;
                let suggestions = this.provider.get_suggestions(
                    &lines,
                    cursor_line,
                    cursor_col,
                    force,
                    &mut |name, prefix| {
                        let func = this.argument_completions.get(name)?;
                        match func.call::<mlua::Value>(prefix.to_owned()) {
                            Ok(mlua::Value::Table(items)) => {
                                match autocomplete_items_from_table(items) {
                                    Ok(items) => Some(items),
                                    Err(error) => {
                                        callback_error = Some(error);
                                        None
                                    }
                                }
                            }
                            Ok(_) => None,
                            Err(error) => {
                                callback_error = Some(error);
                                None
                            }
                        }
                    },
                );
                if let Some(error) = callback_error {
                    return Err(error);
                }
                let Some(suggestions) = suggestions else {
                    return Ok(mlua::Value::Nil);
                };
                let result = lua.create_table()?;
                result.set("prefix", suggestions.prefix)?;
                result.set("items", autocomplete_items_table(lua, &suggestions.items)?)?;
                Ok(mlua::Value::Table(result))
            },
        );
        methods.add_method(
            "should_trigger_file_completion",
            |_, _, (lines, cursor_line, cursor_col): (mlua::Table, usize, usize)| {
                let lines: Vec<String> = lines.sequence_values().collect::<mlua::Result<_>>()?;
                Ok(
                    pi_rs_tui::autocomplete::CombinedProvider::should_trigger_file_completion(
                        &lines,
                        cursor_line,
                        cursor_col,
                    ),
                )
            },
        );
        methods.add_method(
            "apply_completion",
            |lua,
             _,
             (lines, cursor_line, cursor_col, item, prefix): (
                mlua::Table,
                usize,
                usize,
                mlua::Table,
                String,
            )| {
                let lines: Vec<String> = lines.sequence_values().collect::<mlua::Result<_>>()?;
                let item = pi_rs_tui::autocomplete::AutocompleteItem {
                    value: item.get("value")?,
                    label: item.get("label")?,
                    description: item.get("description")?,
                };
                let applied = pi_rs_tui::autocomplete::apply_completion(
                    &lines,
                    cursor_line,
                    cursor_col,
                    &item,
                    &prefix,
                );
                let result = lua.create_table()?;
                result.set("lines", rendered_lines(lua, applied.lines)?)?;
                result.set("cursor_line", applied.cursor_line)?;
                result.set("cursor_col", applied.cursor_col)?;
                Ok(result)
            },
        );
    }
}

fn editor_effect(
    lua: &mlua::Lua,
    effect: Option<pi_rs_tui::editor::EditorEffect>,
) -> mlua::Result<mlua::Table> {
    let result = lua.create_table()?;
    match effect {
        Some(pi_rs_tui::editor::EditorEffect::Changed(text)) => {
            result.set("kind", "changed")?;
            result.set("text", text)?;
        }
        Some(pi_rs_tui::editor::EditorEffect::Submit(text)) => {
            result.set("kind", "submit")?;
            result.set("text", text)?;
        }
        None => result.set("kind", "none")?,
    }
    Ok(result)
}

struct LuaEditor(pi_rs_tui::editor::Editor);
impl UserData for LuaEditor {
    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        methods.add_method("value", |_, this, ()| Ok(this.0.value().to_owned()));
        methods.add_method("cursor", |_, this, ()| Ok(this.0.cursor()));
        methods.add_method("get_text", |_, this, ()| Ok(this.0.text().to_owned()));
        methods.add_method("get_expanded_text", |_, this, ()| {
            Ok(this.0.expanded_text())
        });
        methods.add_method("get_lines", |lua, this, ()| {
            rendered_lines(lua, this.0.lines())
        });
        methods.add_method("get_cursor", |lua, this, ()| {
            let cursor = this.0.logical_cursor();
            let result = lua.create_table()?;
            result.set("line", cursor.line)?;
            result.set("col", cursor.col)?;
            result.set("offset", this.0.cursor())?;
            Ok(result)
        });
        methods.add_method_mut("set_text", |_, this, text: String| {
            this.0.set_text(text);
            Ok(())
        });
        methods.add_method_mut("insert_text_at_cursor", |_, this, text: String| {
            this.0.insert_text_at_cursor(&text);
            Ok(())
        });
        methods.add_method_mut("add_to_history", |_, this, text: String| {
            this.0.add_to_history(&text);
            Ok(())
        });
        methods.add_method_mut("paste", |_, this, text: String| {
            this.0.paste(&text);
            Ok(())
        });
        methods.add_method_mut("input_effect", |lua, this, data: String| {
            editor_effect(lua, this.0.handle_effect(&data))
        });
        methods.add_method_mut("submit", |lua, this, ()| {
            editor_effect(
                lua,
                this.0.submit().map(pi_rs_tui::editor::EditorEffect::Submit),
            )
        });
        methods.add_method_mut("newline", |_, this, ()| {
            this.0.add_newline();
            Ok(())
        });
        methods.add_method_mut("set_padding_x", |_, this, padding: usize| {
            this.0.set_padding_x(padding);
            Ok(())
        });
        methods.add_method_mut(
            "set_border_style",
            |_, this, (open, close): (String, String)| {
                this.0.set_border_style(open, close);
                Ok(())
            },
        );
        methods.add_method_mut("set_select_list_theme", |_, this, theme: mlua::Table| {
            let style = |key: &str| -> mlua::Result<pi_rs_tui::select_list::Style> {
                let Some(slot) = theme.get::<Option<mlua::Table>>(key)? else {
                    return Ok(pi_rs_tui::select_list::Style::default());
                };
                Ok(pi_rs_tui::select_list::Style {
                    open: slot.get::<Option<String>>("open")?.unwrap_or_default(),
                    close: slot.get::<Option<String>>("close")?.unwrap_or_default(),
                })
            };
            this.0
                .set_select_list_theme(pi_rs_tui::select_list::SelectListTheme {
                    selected_text: style("selected_text")?,
                    description: style("description")?,
                    scroll_info: style("scroll_info")?,
                    no_match: style("no_match")?,
                });
            Ok(())
        });
        methods.add_method("padding_x", |_, this, ()| Ok(this.0.padding_x()));
        methods.add_method_mut("set_autocomplete_max_visible", |_, this, value: usize| {
            this.0.set_autocomplete_max_visible(value);
            Ok(())
        });
        methods.add_method("autocomplete_max_visible", |_, this, ()| {
            Ok(this.0.autocomplete_max_visible())
        });
        methods.add_method_mut("set_terminal_rows", |_, this, rows: usize| {
            this.0.set_terminal_rows(rows);
            Ok(())
        });
        methods.add_method_mut("set_focused", |_, this, focused: bool| {
            this.0.set_focused(focused);
            Ok(())
        });
        methods.add_method_mut("set_disable_submit", |_, this, disabled: bool| {
            this.0.set_disable_submit(disabled);
            Ok(())
        });
        methods.add_method("disable_submit", |_, this, ()| Ok(this.0.disable_submit()));
        methods.add_method_mut(
            "set_autocomplete_triggers",
            |_, this, triggers: mlua::Table| {
                let triggers = triggers
                    .sequence_values()
                    .collect::<mlua::Result<Vec<String>>>()?;
                this.0.set_autocomplete_triggers(&triggers);
                Ok(())
            },
        );
        methods.add_method_mut("take_autocomplete_request", |lua, this, ()| {
            let Some(request) = this.0.take_autocomplete_request() else {
                return Ok(mlua::Value::Nil);
            };
            let result = lua.create_table()?;
            result.set("id", request.id)?;
            result.set("lines", rendered_lines(lua, request.lines)?)?;
            result.set("cursor_line", request.cursor_line)?;
            result.set("cursor_col", request.cursor_col)?;
            result.set("force", request.force)?;
            result.set("explicit_tab", request.explicit_tab)?;
            result.set("debounce_ms", request.debounce_ms)?;
            Ok(mlua::Value::Table(result))
        });
        methods.add_method_mut(
            "apply_autocomplete",
            |lua, this, (id, value): (u64, mlua::Value)| {
                let suggestions = match value {
                    mlua::Value::Nil => None,
                    mlua::Value::Table(table) => {
                        let mut items = Vec::new();
                        let item_table: mlua::Table = table.get("items")?;
                        for item in item_table.sequence_values::<mlua::Table>() {
                            let item = item?;
                            items.push(pi_rs_tui::autocomplete::AutocompleteItem {
                                value: item.get("value")?,
                                label: item.get("label")?,
                                description: item.get("description")?,
                            });
                        }
                        Some(pi_rs_tui::autocomplete::Suggestions {
                            items,
                            prefix: table.get("prefix")?,
                        })
                    }
                    _ => {
                        return Err(mlua::Error::runtime(
                            "autocomplete response must be a table or nil",
                        ));
                    }
                };
                let (accepted, changed) = this.0.apply_autocomplete_suggestions(id, suggestions);
                let result = lua.create_table()?;
                result.set("accepted", accepted)?;
                result.set("changed", changed)?;
                result.set("text", this.0.value().to_owned())?;
                Ok(result)
            },
        );
        methods.add_method("autocomplete_showing", |_, this, ()| {
            Ok(this.0.autocomplete_showing())
        });
        methods.add_method_mut("render", |lua, this, width: usize| {
            rendered_lines(lua, this.0.render_configured(width))
        });
        methods.add_method_mut("input", |_, this, data: String| {
            Ok(this.0.handle(&data).is_some())
        });
        methods.add_method_mut("insert", |_, this, text: String| {
            this.0.insert(&text);
            Ok(())
        });
        methods.add_method_mut("backspace", |_, this, ()| {
            this.0.backspace();
            Ok(())
        });
        methods.add_method_mut("delete", |_, this, ()| {
            this.0.delete();
            Ok(())
        });
        methods.add_method_mut("undo", |_, this, ()| {
            this.0.undo();
            Ok(())
        });
        methods.add_method_mut("yank", |_, this, ()| {
            this.0.yank();
            Ok(())
        });
        methods.add_method_mut("yank_pop", |_, this, ()| {
            this.0.yank_pop();
            Ok(())
        });
        methods.add_method_mut("kill_to_start", |_, this, ()| {
            this.0.kill_to_start();
            Ok(())
        });
        methods.add_method_mut("kill_to_end", |_, this, ()| {
            this.0.kill_to_end();
            Ok(())
        });
        methods.add_method_mut("word_left", |_, this, ()| {
            this.0.move_word_left();
            Ok(())
        });
        methods.add_method_mut("word_right", |_, this, ()| {
            this.0.move_word_right();
            Ok(())
        });
    }
}

/// jsdiff change objects as Lua tables (`{value, count, added, removed}`),
/// matching the vendored library's `ChangeObject` shape.
fn changes_to_lua(
    lua: &mlua::Lua,
    changes: Vec<crate::jsdiff::Change>,
) -> mlua::Result<mlua::Table> {
    let result = lua.create_table()?;
    for change in changes {
        let entry = lua.create_table()?;
        entry.set("value", change.value)?;
        entry.set("count", change.count)?;
        entry.set("added", change.added)?;
        entry.set("removed", change.removed)?;
        result.push(entry)?;
    }
    Ok(result)
}

fn rendered_lines(lua: &mlua::Lua, lines: Vec<String>) -> mlua::Result<mlua::Table> {
    let result = lua.create_table()?;
    for line in lines {
        result.push(line)?;
    }
    Ok(result)
}

struct LuaText(Arc<pi_rs_tui::component::Text>);
impl UserData for LuaText {
    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        methods.add_method("render", |lua, this, width: usize| {
            rendered_lines(
                lua,
                pi_rs_tui::component::Component::render(&*this.0, width),
            )
        });
        methods.add_method("set_text", |_, this, text: String| {
            this.0.set_text(text);
            Ok(())
        });
    }
}

struct LuaInput(Arc<pi_rs_tui::input::Input>);
impl UserData for LuaInput {
    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        methods.add_method("render", |lua, this, width: usize| {
            rendered_lines(
                lua,
                pi_rs_tui::component::Component::render(&*this.0, width),
            )
        });
        methods.add_method("value", |_, this, ()| Ok(this.0.value()));
        methods.add_method("set_value", |_, this, value: String| {
            this.0.set_value(value);
            Ok(())
        });
        methods.add_method("set_focused", |_, this, focused: bool| {
            this.0.set_focused(focused);
            Ok(())
        });
        methods.add_method("input", |lua, this, data: String| {
            use pi_rs_tui::input::InputEvent;
            let event = lua.create_table()?;
            match this.0.handle_input(&data) {
                InputEvent::Changed(value) => {
                    event.set("kind", "changed")?;
                    event.set("value", value)?;
                }
                InputEvent::Submit(value) => {
                    event.set("kind", "submit")?;
                    event.set("value", value)?;
                }
                InputEvent::Cancel => event.set("kind", "cancel")?,
                InputEvent::None => event.set("kind", "none")?,
            }
            Ok(event)
        });
    }
}

struct LuaSettingsList(Arc<pi_rs_tui::settings_list::SettingsList>);
impl UserData for LuaSettingsList {
    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        methods.add_method("render", |lua, this, width: usize| {
            rendered_lines(
                lua,
                pi_rs_tui::component::Component::render(&*this.0, width),
            )
        });
        methods.add_method("selected", |lua, this, ()| {
            this.0
                .selected()
                .map(|item| {
                    let value = lua.create_table()?;
                    value.set("id", item.id)?;
                    value.set("label", item.label)?;
                    value.set("description", item.description)?;
                    value.set("current_value", item.current_value)?;
                    value.set("values", item.values)?;
                    Ok(value)
                })
                .transpose()
        });
        methods.add_method("update_value", |_, this, (id, value): (String, String)| {
            this.0.update_value(&id, value);
            Ok(())
        });
        methods.add_method("set_query", |_, this, query: String| {
            this.0.set_query(&query);
            Ok(())
        });
        methods.add_method("query", |_, this, ()| Ok(this.0.query()));
        methods.add_method("select_id", |_, this, id: String| {
            this.0.select_id(&id);
            Ok(())
        });
        methods.add_method("move_up", |_, this, ()| {
            this.0.move_up();
            Ok(())
        });
        methods.add_method("move_down", |_, this, ()| {
            this.0.move_down();
            Ok(())
        });
        methods.add_method("activate", |lua, this, ()| {
            use pi_rs_tui::settings_list::SettingsListAction;
            let action = lua.create_table()?;
            match this.0.activate() {
                SettingsListAction::Changed { id, value } => {
                    action.set("id", id)?;
                    action.set("value", value)?;
                }
                SettingsListAction::Submenu { id, current_value } => {
                    action.set("kind", "submenu")?;
                    action.set("id", id)?;
                    action.set("value", current_value)?;
                }
                SettingsListAction::Cancel => action.set("kind", "cancel")?,
                SettingsListAction::None => action.set("kind", "none")?,
            }
            Ok(action)
        });
        methods.add_method("input", |lua, this, data: String| {
            use pi_rs_tui::settings_list::SettingsListAction;
            let action = lua.create_table()?;
            match this.0.handle_input(&data) {
                SettingsListAction::Changed { id, value } => {
                    action.set("kind", "changed")?;
                    action.set("id", id)?;
                    action.set("value", value)?;
                }
                SettingsListAction::Submenu { id, current_value } => {
                    action.set("kind", "submenu")?;
                    action.set("id", id)?;
                    action.set("value", current_value)?;
                }
                SettingsListAction::Cancel => action.set("kind", "cancel")?,
                SettingsListAction::None => action.set("kind", "none")?,
            }
            Ok(action)
        });
    }
}

struct LuaSpacer(Arc<pi_rs_tui::spacer::Spacer>);
impl UserData for LuaSpacer {
    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        methods.add_method("render", |lua, this, width: usize| {
            rendered_lines(
                lua,
                pi_rs_tui::component::Component::render(&*this.0, width),
            )
        });
        methods.add_method("set_lines", |_, this, lines: usize| {
            this.0.set_lines(lines);
            Ok(())
        });
    }
}

struct LuaTruncatedText(Arc<pi_rs_tui::truncated_text::TruncatedText>);
impl UserData for LuaTruncatedText {
    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        methods.add_method("render", |lua, this, width: usize| {
            rendered_lines(
                lua,
                pi_rs_tui::component::Component::render(&*this.0, width),
            )
        });
        methods.add_method("set_text", |_, this, text: String| {
            this.0.set_text(text);
            Ok(())
        });
    }
}

fn component_handle(value: &AnyUserData) -> mlua::Result<Arc<dyn pi_rs_tui::component::Component>> {
    if let Ok(component) = value.borrow::<LuaText>() {
        return Ok(component.0.clone());
    }
    if let Ok(component) = value.borrow::<LuaInput>() {
        return Ok(component.0.clone());
    }
    if let Ok(component) = value.borrow::<LuaSettingsList>() {
        return Ok(component.0.clone());
    }
    if let Ok(component) = value.borrow::<LuaSpacer>() {
        return Ok(component.0.clone());
    }
    if let Ok(component) = value.borrow::<LuaTruncatedText>() {
        return Ok(component.0.clone());
    }
    if let Ok(component) = value.borrow::<LuaBox>() {
        return Ok(component.component.clone());
    }
    Err(mlua::Error::external(
        "expected a pi.tui component userdata",
    ))
}

struct LuaBox {
    component: Arc<pi_rs_tui::box_component::BoxComponent>,
    background: Mutex<Option<mlua::RegistryKey>>,
}
impl UserData for LuaBox {
    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        methods.add_method("add", |_, this, child: AnyUserData| {
            this.component.add(component_handle(&child)?);
            Ok(())
        });
        methods.add_method("remove", |_, this, child: AnyUserData| {
            let child = component_handle(&child)?;
            this.component.remove(&child);
            Ok(())
        });
        methods.add_method("clear", |_, this, ()| {
            this.component.clear();
            Ok(())
        });
        methods.add_method(
            "set_background",
            |lua, this, background: Option<mlua::Function>| {
                let key = background
                    .map(|function| lua.create_registry_value(function))
                    .transpose()?;
                let mut stored = this
                    .background
                    .lock()
                    .map_err(|_| mlua::Error::external("box background lock poisoned"))?;
                if let Some(old) = stored.take() {
                    lua.remove_registry_value(old)?;
                }
                *stored = key;
                Ok(())
            },
        );
        methods.add_method("render", |lua, this, width: usize| {
            let mut lines = pi_rs_tui::component::Component::render(&*this.component, width);
            let stored = this
                .background
                .lock()
                .map_err(|_| mlua::Error::external("box background lock poisoned"))?;
            if let Some(key) = stored.as_ref() {
                let background: mlua::Function = lua.registry_value(key)?;
                lines = lines
                    .into_iter()
                    .map(|line| background.call(line))
                    .collect::<mlua::Result<_>>()?;
            }
            rendered_lines(lua, lines)
        });
    }
}

/// Get-or-create the per-extension registration entry (spec: the
/// `Extension` object in `types.ts` — per-extension maps of tools and
/// commands). `*_order` arrays make JS `Map` insertion order explicit,
/// which Lua hash parts don't preserve.
fn ext_entry(lua: &mlua::Lua, source: &str) -> mlua::Result<mlua::Table> {
    let registry = registry_table(lua)?;
    let exts: mlua::Table = registry.get("exts")?;
    if let Some(entry) = exts.get::<Option<mlua::Table>>(source)? {
        return Ok(entry);
    }
    let entry = lua.create_table()?;
    entry.set("tools", lua.create_table()?)?;
    entry.set("tool_order", lua.create_table()?)?;
    entry.set("commands", lua.create_table()?)?;
    entry.set("command_order", lua.create_table()?)?;
    entry.set("providers", lua.create_table()?)?;
    entry.set("provider_order", lua.create_table()?)?;
    entry.set("shortcuts", lua.create_table()?)?;
    entry.set("shortcut_order", lua.create_table()?)?;
    entry.set("flags", lua.create_table()?)?;
    entry.set("flag_order", lua.create_table()?)?;
    entry.set("roles", lua.create_table()?)?;
    entry.set("role_order", lua.create_table()?)?;
    let package = lua.create_table()?;
    package.set("command_visibility", "public")?;
    entry.set("package", package)?;
    entry.set("render_middleware", lua.create_table()?)?;
    entry.set("render_middleware_order", lua.create_table()?)?;
    entry.set("ui_slots", lua.create_table()?)?;
    entry.set("ui_slot_order", lua.create_table()?)?;
    exts.set(source, &entry)?;
    let ext_order: mlua::Table = registry.get("ext_order")?;
    ext_order.push(source)?;
    Ok(entry)
}

fn module_key(name: &str, version: &str) -> String {
    format!("{name}\0{version}")
}

fn validate_module_identity(name: &str, version: &str) -> mlua::Result<()> {
    if name.trim().is_empty() {
        return Err(mlua::Error::runtime(
            "module name must be a non-empty string",
        ));
    }
    if version.trim().is_empty() {
        return Err(mlua::Error::runtime(
            "module version must be a non-empty string",
        ));
    }
    Ok(())
}

/// Resolve one exact module version. Factories receive only their declared
/// dependencies, sorted by alias before resolution; the cache is VM-wide and
/// source identity is restored around every factory call.
fn require_module(lua: &mlua::Lua, name: &str, version: &str) -> mlua::Result<mlua::Value> {
    validate_module_identity(name, version)?;
    let registry = registry_table(lua)?;
    let modules: mlua::Table = registry.get("modules")?;
    let key = module_key(name, version);
    let Some(entry) = modules.get::<Option<mlua::Table>>(key.as_str())? else {
        return Err(mlua::Error::runtime(format!(
            "module {name:?} version {version:?} is not defined (required by {})",
            current_source(lua)
        )));
    };
    let state: String = entry.get("state")?;
    if state == "loaded" {
        return entry.get("value");
    }
    let stack: mlua::Table = registry.get("module_stack")?;
    if state == "loading" {
        let mut chain = stack
            .sequence_values::<String>()
            .collect::<mlua::Result<Vec<_>>>()?;
        chain.push(format!("{name}@{version}"));
        return Err(mlua::Error::runtime(format!(
            "module dependency cycle: {}",
            chain.join(" -> ")
        )));
    }

    entry.set("state", "loading")?;
    stack.push(format!("{name}@{version}"))?;
    let source: String = entry.get("source")?;
    let previous_source = current_source(lua);
    set_current_source(lua, &source);
    let result = (|| {
        let declarations: mlua::Table = entry.get("dependencies")?;
        let mut aliases = declarations
            .pairs::<String, mlua::Table>()
            .map(|pair| pair.map(|(alias, _)| alias))
            .collect::<mlua::Result<Vec<_>>>()?;
        aliases.sort();
        let dependencies = lua.create_table()?;
        for alias in aliases {
            let dependency: mlua::Table = declarations.get(alias.as_str())?;
            let dependency_name: String = dependency.get("name")?;
            let dependency_version: String = dependency.get("version")?;
            let value = require_module(lua, &dependency_name, &dependency_version)?;
            dependencies.set(alias, value)?;
        }
        let factory: mlua::Function = entry.get("factory")?;
        let value: mlua::Value = factory.call(dependencies)?;
        if value.is_nil() {
            return Err(mlua::Error::runtime(format!(
                "module {name}@{version} factory returned nil"
            )));
        }
        Ok(value)
    })();
    set_current_source(lua, &previous_source);
    let stack_len = stack.raw_len();
    if stack_len > 0 {
        stack.raw_set(stack_len, mlua::Nil)?;
    }
    match result {
        Ok(value) => {
            entry.set("value", value.clone())?;
            entry.set("state", "loaded")?;
            Ok(value)
        }
        Err(error) => {
            entry.set("state", "defined")?;
            entry.set("value", mlua::Nil)?;
            Err(mlua::Error::runtime(format!(
                "module {name}@{version} from {source}: {error}"
            )))
        }
    }
}

/// Create the registration table and build the `pi` API table. The table
/// is passed to each extension chunk as its single argument; it is *not*
/// installed as a global. `cwd` is the host working directory (spec: the
/// loader's injected `cwd`) — the `pi.exec` default and `pi.cwd()`.
pub(crate) fn build(
    lua: &mlua::Lua,
    cwd: &str,
    project_trusted: bool,
    control: std::sync::Arc<crate::kernel::Control>,
) -> mlua::Result<mlua::Table> {
    let registry = lua.create_table()?;
    registry.set("events", lua.create_table()?)?;
    registry.set("exts", lua.create_table()?)?;
    registry.set("ext_order", lua.create_table()?)?;
    registry.set("flag_values", lua.create_table()?)?;
    registry.set("bus_listeners", lua.create_table()?)?;
    registry.set("modules", lua.create_table()?)?;
    registry.set("module_order", lua.create_table()?)?;
    registry.set("module_stack", lua.create_table()?)?;
    registry.set("source", "<host>")?;
    lua.set_named_registry_value(REGISTRY_KEY, registry)?;

    let pi = lua.create_table()?;

    // JavaScript String.prototype.normalize mechanism. Lua 5.4 has no
    // Unicode normalization; product policy (which form and when) stays
    // in Lua. Exercised by examples/extensions/os-demo.lua.
    let text = lua.create_table()?;
    text.set(
        "nfkc",
        lua.create_function(|_, value: String| {
            use unicode_normalization::UnicodeNormalization;
            Ok(value.nfkc().collect::<String>())
        })?,
    )?;
    pi.set("text", text)?;

    // jsdiff 8.0.4 mechanism (spec `edit-diff.ts` uses `Diff.diffLines` /
    // `Diff.createTwoFilesPatch`; `components/diff.ts` uses `Diff.diffWords`).
    // What to diff and how to present it stays in Lua. Exercised by
    // examples/extensions/diff-demo.lua and tests/jsdiff-parity fixtures.
    let diff = lua.create_table()?;
    diff.set(
        "lines",
        lua.create_function(|lua, (old, new): (String, String)| {
            let changes = crate::jsdiff::diff_lines(&old, &new)
                .map_err(|error| mlua::Error::runtime(error.to_string()))?;
            changes_to_lua(lua, changes)
        })?,
    )?;
    diff.set(
        "words",
        lua.create_function(|lua, (old, new): (String, String)| {
            let changes = crate::jsdiff::diff_words(&old, &new)
                .map_err(|error| mlua::Error::runtime(error.to_string()))?;
            changes_to_lua(lua, changes)
        })?,
    )?;
    diff.set(
        "unified_patch",
        lua.create_function(
            |_,
             (old_name, new_name, old, new, opts): (
                String,
                String,
                String,
                String,
                Option<mlua::Table>,
            )| {
                let context = match &opts {
                    Some(opts) => opts.get::<Option<usize>>("context")?.unwrap_or(4),
                    None => 4,
                };
                let headers = match &opts {
                    Some(opts) => opts.get::<Option<String>>("headers")?,
                    None => None,
                };
                let headers = match headers.as_deref() {
                    None | Some("include") => crate::jsdiff::HeaderOptions::Include,
                    Some("file") => crate::jsdiff::HeaderOptions::FileHeadersOnly,
                    Some("omit") => crate::jsdiff::HeaderOptions::Omit,
                    Some(other) => {
                        return Err(mlua::Error::runtime(format!(
                            "unified_patch: unknown headers option {other:?} (expected include, file, or omit)"
                        )));
                    }
                };
                crate::jsdiff::create_two_files_patch(
                    &old_name, &new_name, &old, &new, context, headers,
                )
                .map_err(|error| mlua::Error::runtime(error.to_string()))
            },
        )?,
    )?;
    pi.set("diff", diff)?;

    // highlight.js 10.7.3 mechanism (spec `utils/syntax-highlight.ts` wraps
    // the library; the Lua port of that wrapper — renderHighlightedHtml,
    // theme mapping, language validation — lives in the builtin packs).
    // Exercised by examples/extensions/highlight-demo.lua and
    // tests/hljs-parity fixtures.
    let hljs = lua.create_table()?;
    hljs.set(
        "highlight",
        lua.create_function(|lua, (code, opts): (String, Option<mlua::Table>)| {
            let language = match &opts {
                Some(opts) => opts.get::<Option<String>>("language")?,
                None => None,
            };
            let ignore_illegals = match &opts {
                Some(opts) => opts
                    .get::<Option<bool>>("ignore_illegals")?
                    .unwrap_or(false),
                None => false,
            };
            let subset = match &opts {
                Some(opts) => opts.get::<Option<Vec<String>>>("language_subset")?,
                None => None,
            };
            let result = match language {
                Some(language) => crate::hljs::highlight(&code, &language, ignore_illegals),
                None => crate::hljs::highlight_auto(&code, subset.as_deref()),
            }
            .map_err(|error| mlua::Error::runtime(error.to_string()))?;
            let table = lua.create_table()?;
            table.set("value", result.value)?;
            table.set("illegal", result.illegal)?;
            table.set("relevance", result.relevance)?;
            table.set("language", result.language)?;
            Ok(table)
        })?,
    )?;
    hljs.set(
        "supports_language",
        lua.create_function(|_, name: String| Ok(crate::hljs::supports_language(&name)))?,
    )?;
    hljs.set(
        "list_languages",
        lua.create_function(|lua, ()| {
            let names = crate::hljs::list_languages()
                .map_err(|error| mlua::Error::runtime(error.to_string()))?;
            let table = lua.create_table()?;
            for name in names {
                table.push(name)?;
            }
            Ok(table)
        })?,
    )?;
    pi.set("hljs", hljs)?;

    // photon 0.3.4 mechanism (spec `utils/image-resize-core.ts`
    // `resizeImageInProcess` and `utils/image-convert.ts` `convertToPng`;
    // pi runs the WASM build in a worker thread, pi-rs the same code on the
    // blocking pool). What to read/note/attach stays Lua (`tools/read.lua`).
    // Exercised by examples/extensions/image-demo.lua and
    // tests/image-parity fixtures.
    let image_api = lua.create_table()?;
    image_api.set(
        "resize",
        lua.create_async_function(
            |lua,
             (bytes, mime_type, options): (mlua::String, String, Option<mlua::Table>)|
             async move {
                let opts = match &options {
                    Some(options) => crate::image::ImageResizeOptions {
                        max_width: options.get::<Option<f64>>("maxWidth")?,
                        max_height: options.get::<Option<f64>>("maxHeight")?,
                        max_bytes: options.get::<Option<f64>>("maxBytes")?,
                        jpeg_quality: options.get::<Option<f64>>("jpegQuality")?,
                    },
                    None => crate::image::ImageResizeOptions::default(),
                };
                let input = bytes.as_bytes().to_vec();
                let resized = tokio::task::spawn_blocking(move || {
                    crate::image::resize_image(&input, &mime_type, opts)
                })
                .await
                .map_err(|error| mlua::Error::runtime(error.to_string()))?;
                match resized {
                    None => Ok(mlua::Value::Nil),
                    Some(resized) => {
                        let table = lua.create_table()?;
                        table.set("data", resized.data)?;
                        table.set("mimeType", resized.mime_type)?;
                        table.set("originalWidth", resized.original_width)?;
                        table.set("originalHeight", resized.original_height)?;
                        table.set("width", resized.width)?;
                        table.set("height", resized.height)?;
                        table.set("wasResized", resized.was_resized)?;
                        Ok(mlua::Value::Table(table))
                    }
                }
            },
        )?,
    )?;
    image_api.set(
        "convert_to_png",
        lua.create_async_function(
            |lua, (base64_data, mime_type): (String, String)| async move {
                let converted = tokio::task::spawn_blocking(move || {
                    crate::image::convert_to_png_base64(&base64_data, &mime_type)
                })
                .await
                .map_err(|error| mlua::Error::runtime(error.to_string()))?;
                match converted {
                    None => Ok(mlua::Value::Nil),
                    Some((data, mime_type)) => {
                        let table = lua.create_table()?;
                        table.set("data", data)?;
                        table.set("mimeType", mime_type)?;
                        Ok(mlua::Value::Table(table))
                    }
                }
            },
        )?,
    )?;
    pi.set("image", image_api)?;

    // JSON.parse mechanism for model argument recovery (some providers
    // stringify structured arguments). Validation and fallback stay Lua.
    let json = lua.create_table()?;
    json.set(
        "decode",
        lua.create_function(|lua, value: String| {
            let parsed: serde_json::Value = serde_json::from_str(&value)
                .map_err(|error| mlua::Error::runtime(error.to_string()))?;
            crate::convert::json_to_lua(lua, &parsed)
        })?,
    )?;
    json.set(
        "encode",
        lua.create_function(|_, (value, pretty): (mlua::Value, Option<bool>)| {
            let json = crate::convert::lua_to_json(value)?;
            let encoded = if pretty.unwrap_or(false) {
                serde_json::to_string_pretty(&json)
            } else {
                serde_json::to_string(&json)
            };
            encoded.map_err(|error| mlua::Error::runtime(error.to_string()))
        })?,
    )?;
    pi.set("json", json)?;

    // Deterministic, exact-version Lua modules. A module factory receives only
    // its declared dependency aliases; definitions and imports are identical
    // for embedded and file-backed sources. Package transport is deliberately
    // outside this mechanism: modules are values after their package is loaded.
    let module_api = lua.create_table()?;
    module_api.set(
        "define",
        lua.create_function(|lua, definition: mlua::Table| {
            let name: String = definition.get("name")?;
            let version: String = definition.get("version")?;
            validate_module_identity(&name, &version)?;
            let factory = definition
                .get::<Option<mlua::Function>>("factory")?
                .ok_or_else(|| mlua::Error::runtime("module factory must be a function"))?;
            let dependencies = definition
                .get::<Option<mlua::Table>>("dependencies")?
                .unwrap_or(lua.create_table()?);

            // Copy + validate declarations so later caller mutation cannot alter
            // dependency resolution. Aliases are sorted to make diagnostics stable.
            let mut declared = dependencies
                .pairs::<String, mlua::Table>()
                .collect::<mlua::Result<Vec<_>>>()?;
            declared.sort_by(|(left, _), (right, _)| left.cmp(right));
            let copied_dependencies = lua.create_table()?;
            for (alias, dependency) in declared {
                if alias.trim().is_empty() {
                    return Err(mlua::Error::runtime(
                        "module dependency alias must be a non-empty string",
                    ));
                }
                let dependency_name: String = dependency.get("name")?;
                let dependency_version: String = dependency.get("version")?;
                validate_module_identity(&dependency_name, &dependency_version)?;
                let copied = lua.create_table()?;
                copied.set("name", dependency_name)?;
                copied.set("version", dependency_version)?;
                copied_dependencies.set(alias, copied)?;
            }

            let registry = registry_table(lua)?;
            let modules: mlua::Table = registry.get("modules")?;
            let key = module_key(&name, &version);
            if let Some(existing) = modules.get::<Option<mlua::Table>>(key.as_str())? {
                let existing_source: String = existing.get("source")?;
                return Err(mlua::Error::runtime(format!(
                    "module {name}@{version} is already defined by {existing_source}"
                )));
            }
            let entry = lua.create_table()?;
            entry.set("name", name.as_str())?;
            entry.set("version", version.as_str())?;
            entry.set("source", current_source(lua))?;
            entry.set("dependencies", copied_dependencies)?;
            entry.set("factory", factory)?;
            entry.set("state", "defined")?;
            modules.set(key.as_str(), entry)?;
            let order: mlua::Table = registry.get("module_order")?;
            order.push(key)?;
            Ok(())
        })?,
    )?;
    module_api.set(
        "require",
        lua.create_function(|lua, (name, version): (String, String)| {
            require_module(lua, &name, &version)
        })?,
    )?;
    module_api.set(
        "list",
        lua.create_function(|lua, ()| {
            let registry = registry_table(lua)?;
            let modules: mlua::Table = registry.get("modules")?;
            let order: mlua::Table = registry.get("module_order")?;
            let result = lua.create_table()?;
            for key in order.sequence_values::<String>() {
                let key = key?;
                let Some(entry) = modules.get::<Option<mlua::Table>>(key.as_str())? else {
                    continue;
                };
                let item = lua.create_table()?;
                item.set("name", entry.get::<String>("name")?)?;
                item.set("version", entry.get::<String>("version")?)?;
                item.set("source", entry.get::<String>("source")?)?;
                item.set("state", entry.get::<String>("state")?)?;
                result.push(item)?;
            }
            Ok(result)
        })?,
    )?;
    pi.set("module", module_api.clone())?;
    crate::kernel_api::install(lua, &pi, &module_api, control)?;

    // Mechanisms used by the Lua-authored agent loop. Definitions stay in
    // Lua (including execute/prepare functions); this returns the resolved
    // first-registration-wins view rather than a JSON metadata mirror.
    pi.set(
        "registered_tools",
        lua.create_function(|lua, ()| {
            let result = lua.create_table()?;
            for (_, _, def) in all_tools(lua)? {
                result.push(def)?;
            }
            Ok(result)
        })?,
    )?;
    // Product activation is declaration data on each tool, never inferred
    // from whether its source key happens to be synthetic. Ordinary tools
    // default active for Pi compatibility; shipped tools state the field
    // explicitly in their package.
    let registered_active_tools = lua.create_function(|lua, ()| {
        let result = lua.create_table()?;
        for (_, _, def) in all_tools(lua)? {
            if def
                .get::<Option<bool>>("active_by_default")?
                .unwrap_or(true)
            {
                result.push(def)?;
            }
        }
        Ok(result)
    })?;
    pi.set("registered_active_tools", registered_active_tools.clone())?;
    // Compatibility alias for the first product-loading slice. Its behavior
    // is now declaration-driven and source-identity-neutral.
    pi.set("registered_extension_tools", registered_active_tools)?;
    pi.set(
        "registered_extension_commands",
        lua.create_function(|lua, ()| {
            let result = lua.create_table()?;
            for command in resolved_commands(lua)? {
                if !command_is_public(lua, &command.source, &command.entry)? {
                    continue;
                }
                let entry = lua.create_table()?;
                entry.set("name", command.name)?;
                entry.set("invocation_name", command.invocation_name)?;
                entry.set("source", command.source.as_str())?;
                entry.set("description", command.description)?;
                let source_info = lua.create_table()?;
                source_info.set("path", command.source.as_str())?;
                source_info.set("source", "cli")?;
                source_info.set("scope", "temporary")?;
                source_info.set("origin", "top-level")?;
                entry.set("source_info", source_info)?;
                if let Some(completions) = command.get_argument_completions {
                    let source = command.source.clone();
                    entry.set(
                        "get_argument_completions",
                        lua.create_function(move |lua, prefix: String| {
                            let previous = current_source(lua);
                            set_current_source(lua, &source);
                            let outcome: mlua::Result<mlua::Value> = completions.call(prefix);
                            set_current_source(lua, &previous);
                            outcome
                        })?,
                    )?;
                }
                let source = command.source;
                let handler = command.handler;
                entry.set(
                    "handler",
                    lua.create_async_function(move |lua, (args, ctx): (String, mlua::Table)| {
                        let source = source.clone();
                        let handler = handler.clone();
                        async move {
                            let previous = current_source(&lua);
                            set_current_source(&lua, &source);
                            let outcome: mlua::Result<mlua::Value> =
                                handler.call_async((args, ctx)).await;
                            set_current_source(&lua, &previous);
                            outcome
                        }
                    })?,
                )?;
                result.push(entry)?;
            }
            Ok(result)
        })?,
    )?;
    // ExtensionAPI.getCommands. Prompt-template and skill rows join when their
    // Lua resource registries land; extension commands use the same resolved
    // invocation names and source snapshots as the product router.
    pi.set(
        "get_commands",
        lua.create_function(|lua, ()| {
            let result = lua.create_table()?;
            for command in resolved_commands(lua)? {
                if !command_is_public(lua, &command.source, &command.entry)? {
                    continue;
                }
                let entry = lua.create_table()?;
                entry.set("name", command.invocation_name)?;
                entry.set("description", command.description)?;
                entry.set("source", "extension")?;
                let source_info = lua.create_table()?;
                source_info.set("path", command.source.as_str())?;
                source_info.set("source", "cli")?;
                source_info.set("scope", "temporary")?;
                source_info.set("origin", "top-level")?;
                entry.set("sourceInfo", source_info)?;
                result.push(entry)?;
            }
            Ok(result)
        })?,
    )?;
    // Snapshot extension handlers for Lua-authored product fold policy. Each
    // wrapper restores source attribution around the async callback; Rust does
    // not interpret event results or choose stop/merge semantics.
    pi.set(
        "extension_handlers",
        lua.create_function(|lua, event: String| {
            let result = lua.create_table()?;
            for (source, handler) in event_handlers(lua, &event)? {
                let entry = lua.create_table()?;
                entry.set("source", source.as_str())?;
                let wrapper_source = source.clone();
                entry.set(
                    "handler",
                    lua.create_async_function(
                        move |lua, (event, ctx): (mlua::Table, mlua::Table)| {
                            let handler = handler.clone();
                            let source = wrapper_source.clone();
                            async move {
                                let previous = current_source(&lua);
                                set_current_source(&lua, &source);
                                let outcome: mlua::Result<mlua::Value> =
                                    handler.call_async((event, ctx)).await;
                                set_current_source(&lua, &previous);
                                outcome
                            }
                        },
                    )?,
                )?;
                result.push(entry)?;
            }
            Ok(result)
        })?,
    )?;
    pi.set(
        "validate_tool_arguments",
        lua.create_function(
            |lua, (name, schema, arguments): (String, mlua::Value, mlua::Value)| {
                let schema = crate::convert::lua_to_json(schema)?;
                let arguments = crate::convert::lua_to_json(arguments)?;
                let validated = crate::schema::validate_tool_arguments(&name, &schema, &arguments)
                    .map_err(|error| mlua::Error::runtime(error.to_string()))?;
                crate::convert::json_to_lua(lua, &validated)
            },
        )?,
    )?;

    // Thin CLI output mechanism; formatting and event policy remain Lua.
    pi.set(
        "output",
        lua.create_function(|_, text: String| {
            use std::io::Write as _;
            print!("{text}");
            std::io::stdout().flush().map_err(mlua::Error::external)
        })?,
    )?;

    // Package-level declaration defaults. Embedded and file-backed chunks
    // call the same function; source keys remain attribution only.
    pi.set(
        "declare_package",
        lua.create_function(|lua, options: mlua::Table| {
            let visibility = options
                .get::<Option<String>>("command_visibility")?
                .unwrap_or_else(|| "public".to_owned());
            if visibility != "public" && visibility != "internal" {
                return Err(mlua::Error::runtime(
                    "declare_package: command_visibility must be 'public' or 'internal'",
                ));
            }
            let source = current_source(lua);
            let ext = ext_entry(lua, &source)?;
            let package = lua.create_table()?;
            package.set("command_visibility", visibility)?;
            ext.set("package", package)?;
            Ok(())
        })?,
    )?;

    let on = lua.create_function(|lua, (event, handler): (String, mlua::Function)| {
        if event.trim().is_empty() {
            return Err(mlua::Error::runtime(
                "on: event name must be a non-empty string",
            ));
        }
        let events: mlua::Table = registry_table(lua)?.get("events")?;
        let list: mlua::Table = match events.get::<Option<mlua::Table>>(event.as_str())? {
            Some(list) => list,
            None => {
                let list = lua.create_table()?;
                events.set(event.as_str(), &list)?;
                list
            }
        };
        let entry = lua.create_table()?;
        entry.set("fn", handler)?;
        entry.set("source", current_source(lua))?;
        list.push(entry)?;
        Ok(())
    })?;
    pi.set("on", on)?;

    // Shared inter-extension event bus (`pi.events`). Unlike lifecycle events,
    // channels are open strings, listeners see the same payload, and failures
    // are isolated. Snapshotting active listeners preserves EventEmitter's
    // add/remove-during-emit behavior.
    let events_api = lua.create_table()?;
    events_api.set(
        "on",
        lua.create_function(|lua, (channel, handler): (String, mlua::Function)| {
            let registry = registry_table(lua)?;
            let listeners: mlua::Table = registry.get("bus_listeners")?;
            let list = match listeners.get::<Option<mlua::Table>>(channel.as_str())? {
                Some(list) => list,
                None => {
                    let list = lua.create_table()?;
                    listeners.set(channel.as_str(), &list)?;
                    list
                }
            };
            let entry = lua.create_table()?;
            entry.set("fn", handler)?;
            entry.set("source", current_source(lua))?;
            entry.set("active", true)?;
            list.push(entry.clone())?;
            lua.create_function(move |_, ()| entry.set("active", false))
        })?,
    )?;
    events_api.set(
        "emit",
        lua.create_function(|lua, (channel, data): (String, mlua::Value)| {
            let listeners: mlua::Table = registry_table(lua)?.get("bus_listeners")?;
            let Some(list) = listeners.get::<Option<mlua::Table>>(channel.as_str())? else {
                return Ok(());
            };
            let mut snapshot = Vec::new();
            for entry in list.sequence_values::<mlua::Table>() {
                let entry = entry?;
                if entry.get::<Option<bool>>("active")?.unwrap_or(true) {
                    snapshot.push((
                        entry.get::<String>("source")?,
                        entry.get::<mlua::Function>("fn")?,
                    ));
                }
            }
            for (source, handler) in snapshot {
                let previous = current_source(lua);
                set_current_source(lua, &source);
                let outcome = handler.call::<()>(data.clone());
                set_current_source(lua, &previous);
                if let Err(error) = outcome {
                    eprintln!("Event handler error ({channel}): {error}");
                }
            }
            Ok(())
        })?,
    )?;
    pi.set("events", events_api)?;

    // Awaitable host future: the calling coroutine suspends; the VM thread
    // stays free to run the timer. Await time is excluded from the watchdog
    // budget (see vm.rs). The optional signal ports Pi's sleep(ms, signal)
    // cancellation seam used by AgentSession retry backoff.
    let sleep = lua.create_async_function(
        |lua, (ms, signal): (u64, Option<AnyUserData>)| async move {
            let signal = signal
                .map(|signal| {
                    signal
                        .borrow::<crate::ai::LuaAbortSignal>()
                        .map(|signal| signal.0.clone())
                })
                .transpose()?;
            let scope = crate::kernel_api::current_cancellation(&lua)?;
            match (signal, scope) {
                (Some(signal), Some(scope)) => tokio::select! {
                    () = tokio::time::sleep(std::time::Duration::from_millis(ms)) => Ok(()),
                    () = signal.aborted() => Err(mlua::Error::runtime("sleep aborted")),
                    () = scope.cancelled() => Err(mlua::Error::runtime(crate::error::CANCEL_MARKER)),
                },
                (Some(signal), None) => tokio::select! {
                    () = tokio::time::sleep(std::time::Duration::from_millis(ms)) => Ok(()),
                    () = signal.aborted() => Err(mlua::Error::runtime("sleep aborted")),
                },
                (None, Some(scope)) => tokio::select! {
                    () = tokio::time::sleep(std::time::Duration::from_millis(ms)) => Ok(()),
                    () = scope.cancelled() => Err(mlua::Error::runtime(crate::error::CANCEL_MARKER)),
                },
                (None, None) => {
                    tokio::time::sleep(std::time::Duration::from_millis(ms)).await;
                    Ok(())
                }
            }
        },
    )?;
    pi.set("sleep", sleep)?;
    // Generic structured concurrency mechanism. Policy (which tasks to group,
    // and how to order their results) remains in Lua. Results are reported in
    // completion order so callers can reproduce Promise completion semantics.
    let parallel = lua.create_async_function(|lua, tasks: mlua::Table| async move {
        use futures_util::stream::{FuturesUnordered, StreamExt};

        let pending = FuturesUnordered::new();
        for (offset, task) in tasks.sequence_values::<mlua::Function>().enumerate() {
            let task = task?;
            pending.push(async move {
                let result = task.call_async::<mlua::Value>(()).await;
                (offset + 1, result)
            });
        }
        let completed = lua.create_table()?;
        let mut pending = pending;
        while let Some((index, result)) = pending.next().await {
            let entry = lua.create_table()?;
            entry.set("index", index)?;
            match result {
                Ok(value) => {
                    entry.set("ok", true)?;
                    entry.set("value", value)?;
                }
                Err(error) => {
                    entry.set("ok", false)?;
                    entry.set("error", error.to_string())?;
                }
            }
            completed.push(entry)?;
        }
        Ok(completed)
    })?;
    pi.set("parallel", parallel)?;
    // Structured background concurrency for long-lived handlers: the
    // function runs as its own coroutine on the dispatch's task set,
    // interleaving with the caller at await points (the interactive
    // frontend runs agent turns this way while its event loop keeps
    // rendering). The task lives within the current dispatch — anything
    // still pending when the dispatch returns is dropped.
    pi.set(
        "spawn",
        lua.create_function(|lua, func: mlua::Function| {
            let handle = tokio::task::spawn_local(func.call_async::<mlua::Value>(()));
            lua.create_userdata(LuaSpawnHandle(std::cell::RefCell::new(Some(handle))))
        })?,
    )?;
    let epoch = std::time::Instant::now();
    pi.set(
        "monotonic_ms",
        lua.create_function(move |_, ()| {
            Ok(u64::try_from(epoch.elapsed().as_millis()).unwrap_or(u64::MAX))
        })?,
    )?;
    // JS `Date.now()` — epoch milliseconds (the spec's timestamps and
    // timing gates use wall-clock ms, not seconds).
    pi.set(
        "now_ms",
        lua.create_function(|_, ()| {
            Ok(std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
                .unwrap_or(0))
        })?,
    )?;

    // Spec `registerTool` (loader.ts): validate the mechanism-required
    // fields, then per-extension `Map.set` — re-registration of the same
    // name replaces the definition but keeps its position.
    let register_tool = lua.create_function(|lua, def: mlua::Table| {
        let name = def
            .get::<Option<String>>("name")?
            .filter(|n| !n.trim().is_empty())
            .ok_or_else(|| {
                mlua::Error::runtime("register_tool: tool.name must be a non-empty string")
            })?;
        if def.get::<Option<mlua::Function>>("execute")?.is_none() {
            return Err(mlua::Error::runtime(
                "register_tool: tool.execute must be a function",
            ));
        }
        let source = current_source(lua);
        let ext = ext_entry(lua, &source)?;
        let tools: mlua::Table = ext.get("tools")?;
        if tools.get::<Option<mlua::Value>>(name.as_str())?.is_none() {
            let order: mlua::Table = ext.get("tool_order")?;
            order.push(name.as_str())?;
        }
        tools.set(name.as_str(), def)?;
        Ok(())
    })?;
    pi.set("register_tool", register_tool)?;

    // Spec `registerCommand` (loader.ts): `{ name, ...options }` into the
    // per-extension map. Options are shallow-copied so later mutation of
    // the caller's table doesn't alias the registry.
    let register_command = lua.create_function(|lua, (name, options): (String, mlua::Table)| {
        if name.trim().is_empty() {
            return Err(mlua::Error::runtime(
                "register_command: name must be a non-empty string",
            ));
        }
        if options.get::<Option<mlua::Function>>("handler")?.is_none() {
            return Err(mlua::Error::runtime(
                "register_command: options.handler must be a function",
            ));
        }
        let source = current_source(lua);
        let ext = ext_entry(lua, &source)?;
        let entry = lua.create_table()?;
        for pair in options.pairs::<mlua::Value, mlua::Value>() {
            let (k, v) = pair?;
            entry.set(k, v)?;
        }
        entry.set("name", name.as_str())?;
        let commands: mlua::Table = ext.get("commands")?;
        if commands
            .get::<Option<mlua::Value>>(name.as_str())?
            .is_none()
        {
            let order: mlua::Table = ext.get("command_order")?;
            order.push(name.as_str())?;
        }
        commands.set(name.as_str(), entry)?;
        Ok(())
    })?;
    pi.set("register_command", register_command)?;

    // Public application/frontend declarations. Selection is by generic role
    // plus explicit active/priority data; extension load order and source-key
    // syntax never affect the winner.
    pi.set(
        "register_role",
        lua.create_function(|lua, definition: mlua::Table| {
            let id = definition
                .get::<Option<String>>("id")?
                .filter(|value| !value.trim().is_empty())
                .ok_or_else(|| {
                    mlua::Error::runtime("register_role: id must be a non-empty string")
                })?;
            let role = definition
                .get::<Option<String>>("role")?
                .filter(|value| !value.trim().is_empty())
                .ok_or_else(|| {
                    mlua::Error::runtime("register_role: role must be a non-empty string")
                })?;
            if definition.get::<Option<bool>>("active")?.is_none() {
                return Err(mlua::Error::runtime(
                    "register_role: active must be declared explicitly",
                ));
            }
            if definition.get::<Option<i64>>("priority")?.is_none() {
                return Err(mlua::Error::runtime(
                    "register_role: priority must be declared explicitly",
                ));
            }
            if definition
                .get::<Option<mlua::Function>>("handler")?
                .is_none()
            {
                return Err(mlua::Error::runtime(
                    "register_role: handler must be a function",
                ));
            }
            let source = current_source(lua);
            let ext = ext_entry(lua, &source)?;
            let roles: mlua::Table = ext.get("roles")?;
            if roles.get::<Option<mlua::Value>>(id.as_str())?.is_none() {
                let order: mlua::Table = ext.get("role_order")?;
                order.push(id.as_str())?;
            }
            let entry = lua.create_table()?;
            for pair in definition.pairs::<mlua::Value, mlua::Value>() {
                let (key, value) = pair?;
                entry.set(key, value)?;
            }
            entry.set("id", id.as_str())?;
            entry.set("role", role)?;
            roles.set(id, entry)?;
            Ok(())
        })?,
    )?;
    pi.set(
        "registered_roles",
        lua.create_function(|lua, role: Option<String>| {
            let result = lua.create_table()?;
            for (_, _, declaration) in all_roles(lua)? {
                if role
                    .as_ref()
                    .is_none_or(|role| declaration.get::<String>("role").is_ok_and(|v| &v == role))
                {
                    result.push(declaration)?;
                }
            }
            Ok(result)
        })?,
    )?;

    // Spec `registerShortcut` (loader.ts): `{ shortcut, ...options }` into
    // the per-extension map, keyed by the lowercased key id. Conflict policy
    // against built-in keybindings stays with the frontend (runner.ts
    // getShortcuts); the host registry is mechanism only.
    let register_shortcut = lua.create_function(|lua, (key, options): (String, mlua::Table)| {
        if key.trim().is_empty() {
            return Err(mlua::Error::runtime(
                "register_shortcut: shortcut must be a non-empty string",
            ));
        }
        if options.get::<Option<mlua::Function>>("handler")?.is_none() {
            return Err(mlua::Error::runtime(
                "register_shortcut: options.handler must be a function",
            ));
        }
        let key = key.to_lowercase();
        let source = current_source(lua);
        let ext = ext_entry(lua, &source)?;
        let entry = lua.create_table()?;
        for pair in options.pairs::<mlua::Value, mlua::Value>() {
            let (k, v) = pair?;
            entry.set(k, v)?;
        }
        entry.set("shortcut", key.as_str())?;
        let shortcuts: mlua::Table = ext.get("shortcuts")?;
        if shortcuts
            .get::<Option<mlua::Value>>(key.as_str())?
            .is_none()
        {
            let order: mlua::Table = ext.get("shortcut_order")?;
            order.push(key.as_str())?;
        }
        shortcuts.set(key.as_str(), entry)?;
        Ok(())
    })?;
    pi.set("register_shortcut", register_shortcut)?;

    // Resolved first-registration-wins view for the frontend (spec
    // runner.ts getShortcuts, minus the keybinding-conflict diagnostics
    // that land with user keybinding config).
    pi.set(
        "registered_shortcuts",
        lua.create_function(|lua, ()| {
            let mut seen = std::collections::HashSet::new();
            let result = lua.create_table()?;
            for (_, key, entry) in registrations(lua, "shortcuts", "shortcut_order")? {
                if seen.insert(key) {
                    result.push(entry)?;
                }
            }
            Ok(result)
        })?,
    )?;

    // Additive pi-rs composition surface: transcript renderers and shell slots
    // use one ordered declaration mechanism for embedded and file-backed Lua.
    // Definitions remain Lua functions; wrappers restore source attribution.
    let register_render_middleware =
        lua.create_function(|lua, (kind, options): (String, mlua::Table)| {
            if kind.trim().is_empty() {
                return Err(mlua::Error::runtime(
                    "register_render_middleware: kind must be a non-empty string",
                ));
            }
            if options.get::<Option<mlua::Function>>("render")?.is_none() {
                return Err(mlua::Error::runtime(
                    "register_render_middleware: options.render must be a function",
                ));
            }
            let name = options
                .get::<Option<String>>("name")?
                .filter(|name| !name.trim().is_empty())
                .unwrap_or_else(|| kind.clone());
            let key = format!("{kind}\u{0}{name}");
            let source = current_source(lua);
            let ext = ext_entry(lua, &source)?;
            let entries: mlua::Table = ext.get("render_middleware")?;
            if entries.get::<Option<mlua::Value>>(key.as_str())?.is_none() {
                let order: mlua::Table = ext.get("render_middleware_order")?;
                order.push(key.as_str())?;
            }
            let entry = lua.create_table()?;
            entry.set("kind", kind)?;
            entry.set("name", name)?;
            entry.set("order", options.get::<Option<i64>>("order")?.unwrap_or(0))?;
            entry.set("render", options.get::<mlua::Function>("render")?)?;
            entries.set(key, entry)?;
            Ok(())
        })?;
    pi.set("register_render_middleware", register_render_middleware)?;
    pi.set(
        "registered_render_middlewares",
        lua.create_function(|lua, kind: Option<String>| {
            let mut entries = registrations(lua, "render_middleware", "render_middleware_order")?
                .into_iter()
                .enumerate()
                .filter_map(|(index, (source, _, entry))| {
                    let entry_kind = entry.get::<String>("kind").ok()?;
                    if kind
                        .as_ref()
                        .is_some_and(|kind| kind != &entry_kind && entry_kind != "*")
                    {
                        return None;
                    }
                    let order = entry
                        .get::<Option<i64>>("order")
                        .ok()
                        .flatten()
                        .unwrap_or(0);
                    Some((order, index, source, entry))
                })
                .collect::<Vec<_>>();
            entries.sort_by_key(|(order, index, _, _)| (*order, *index));
            let result = lua.create_table()?;
            for (order, _, source, entry) in entries {
                let value = lua.create_table()?;
                value.set("source", source.as_str())?;
                value.set("kind", entry.get::<String>("kind")?)?;
                value.set("name", entry.get::<String>("name")?)?;
                value.set("order", order)?;
                let render = entry.get::<mlua::Function>("render")?;
                value.set(
                    "render",
                    lua.create_function(move |lua, args: mlua::MultiValue| {
                        let previous = current_source(lua);
                        set_current_source(lua, &source);
                        let outcome = render.call::<mlua::Value>(args);
                        set_current_source(lua, &previous);
                        outcome
                    })?,
                )?;
                result.push(value)?;
            }
            Ok(result)
        })?,
    )?;

    let register_ui_slot = lua.create_function(|lua, (slot, options): (String, mlua::Table)| {
        if slot.trim().is_empty() {
            return Err(mlua::Error::runtime(
                "register_ui_slot: slot must be a non-empty string",
            ));
        }
        if options.get::<Option<mlua::Function>>("render")?.is_none() {
            return Err(mlua::Error::runtime(
                "register_ui_slot: options.render must be a function",
            ));
        }
        let name = options
            .get::<Option<String>>("name")?
            .filter(|name| !name.trim().is_empty())
            .unwrap_or_else(|| slot.clone());
        let key = format!("{slot}\u{0}{name}");
        let source = current_source(lua);
        let ext = ext_entry(lua, &source)?;
        let entries: mlua::Table = ext.get("ui_slots")?;
        if entries.get::<Option<mlua::Value>>(key.as_str())?.is_none() {
            let order: mlua::Table = ext.get("ui_slot_order")?;
            order.push(key.as_str())?;
        }
        let entry = lua.create_table()?;
        entry.set("slot", slot)?;
        entry.set("name", name)?;
        entry.set("order", options.get::<Option<i64>>("order")?.unwrap_or(0))?;
        entry.set("render", options.get::<mlua::Function>("render")?)?;
        entries.set(key, entry)?;
        Ok(())
    })?;
    pi.set("register_ui_slot", register_ui_slot)?;
    pi.set(
        "registered_ui_slots",
        lua.create_function(|lua, slot: Option<String>| {
            let mut entries = registrations(lua, "ui_slots", "ui_slot_order")?
                .into_iter()
                .enumerate()
                .filter_map(|(index, (source, _, entry))| {
                    let entry_slot = entry.get::<String>("slot").ok()?;
                    if slot
                        .as_ref()
                        .is_some_and(|slot| slot != &entry_slot && entry_slot != "*")
                    {
                        return None;
                    }
                    let order = entry
                        .get::<Option<i64>>("order")
                        .ok()
                        .flatten()
                        .unwrap_or(0);
                    Some((order, index, source, entry))
                })
                .collect::<Vec<_>>();
            entries.sort_by_key(|(order, index, _, _)| (*order, *index));
            let result = lua.create_table()?;
            for (order, _, source, entry) in entries {
                let value = lua.create_table()?;
                value.set("source", source.as_str())?;
                value.set("slot", entry.get::<String>("slot")?)?;
                value.set("name", entry.get::<String>("name")?)?;
                value.set("order", order)?;
                let render = entry.get::<mlua::Function>("render")?;
                value.set(
                    "render",
                    lua.create_function(move |lua, args: mlua::MultiValue| {
                        let previous = current_source(lua);
                        set_current_source(lua, &source);
                        let outcome = render.call::<mlua::Value>(args);
                        set_current_source(lua, &previous);
                        outcome
                    })?,
                )?;
                result.push(value)?;
            }
            Ok(result)
        })?,
    )?;

    // Spec `registerFlag`/`getFlag`: definitions stay per extension while
    // parsed/default values live in one shared runtime map. Defaults are
    // first-wins, matching `runtime.flagValues.has(name)`.
    let register_flag = lua.create_function(|lua, (name, options): (String, mlua::Table)| {
        if name.trim().is_empty() {
            return Err(mlua::Error::runtime(
                "register_flag: name must be a non-empty string",
            ));
        }
        let flag_type: String = options.get("type")?;
        if flag_type != "boolean" && flag_type != "string" {
            return Err(mlua::Error::runtime(
                "register_flag: options.type must be 'boolean' or 'string'",
            ));
        }
        let source = current_source(lua);
        let ext = ext_entry(lua, &source)?;
        let entry = lua.create_table()?;
        for pair in options.pairs::<mlua::Value, mlua::Value>() {
            let (key, value) = pair?;
            entry.set(key, value)?;
        }
        entry.set("name", name.as_str())?;
        entry.set("extension_path", source.as_str())?;
        let flags: mlua::Table = ext.get("flags")?;
        if flags.get::<Option<mlua::Value>>(name.as_str())?.is_none() {
            let order: mlua::Table = ext.get("flag_order")?;
            order.push(name.as_str())?;
        }
        let default: mlua::Value = entry.get("default")?;
        flags.set(name.as_str(), entry)?;
        if !default.is_nil() {
            let values: mlua::Table = registry_table(lua)?.get("flag_values")?;
            if values.get::<Option<mlua::Value>>(name.as_str())?.is_none() {
                values.set(name.as_str(), default)?;
            }
        }
        Ok(())
    })?;
    pi.set("register_flag", register_flag)?;
    pi.set(
        "get_flag",
        lua.create_function(|lua, name: String| {
            let source = current_source(lua);
            let ext = ext_entry(lua, &source)?;
            let flags: mlua::Table = ext.get("flags")?;
            if flags.get::<Option<mlua::Value>>(name.as_str())?.is_none() {
                return Ok(mlua::Value::Nil);
            }
            let values: mlua::Table = registry_table(lua)?.get("flag_values")?;
            values.get(name.as_str())
        })?,
    )?;

    // Spec `registerProvider` (loader.ts / model-registry.ts): the
    // host-side half — store the config per extension, merging defined
    // keys over an existing registration of the same name (spec
    // `upsertRegisteredProvider`). Function values (`streamSimple`,
    // `oauth.*`) stay Lua-side, invocable when their mechanisms land
    // (WS2.5 auth, WS5 custom streams); the JSON mirror strips them.
    let register_provider = lua.create_function(|lua, (name, config): (String, mlua::Table)| {
        if name.trim().is_empty() {
            return Err(mlua::Error::runtime(
                "register_provider: name must be a non-empty string",
            ));
        }
        let source = current_source(lua);
        let ext = ext_entry(lua, &source)?;
        let providers: mlua::Table = ext.get("providers")?;
        let entry = match providers.get::<Option<mlua::Table>>(name.as_str())? {
            Some(existing) => existing,
            None => {
                let order: mlua::Table = ext.get("provider_order")?;
                order.push(name.as_str())?;
                let entry = lua.create_table()?;
                providers.set(name.as_str(), &entry)?;
                entry
            }
        };
        for pair in config.pairs::<mlua::Value, mlua::Value>() {
            let (k, v) = pair?;
            entry.set(k, v)?;
        }
        Ok(())
    })?;
    pi.set("register_provider", register_provider)?;

    // Spec `unregisterProvider`: removal by name regardless of which
    // extension registered it (the spec's registry is keyed globally);
    // no effect if the name was never registered.
    let unregister_provider = lua.create_function(|lua, name: String| {
        let registry = registry_table(lua)?;
        let exts: mlua::Table = registry.get("exts")?;
        let ext_order: mlua::Table = registry.get("ext_order")?;
        for source in ext_order.sequence_values::<String>() {
            let source = source?;
            let Some(ext) = exts.get::<Option<mlua::Table>>(source.as_str())? else {
                continue;
            };
            let providers: mlua::Table = ext.get("providers")?;
            if providers
                .get::<Option<mlua::Value>>(name.as_str())?
                .is_none()
            {
                continue;
            }
            providers.set(name.as_str(), mlua::Value::Nil)?;
            let order: mlua::Table = ext.get("provider_order")?;
            let kept = lua.create_table()?;
            for entry in order.sequence_values::<String>() {
                let entry = entry?;
                if entry != name {
                    kept.push(entry)?;
                }
            }
            ext.set("provider_order", kept)?;
        }
        Ok(())
    })?;
    pi.set("unregister_provider", unregister_provider)?;

    // TUI mechanism bindings. Components and differential cell output remain
    // policy-free Rust mechanism; Lua owns composition and frontend behavior.
    let tui = lua.create_table()?;
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
        "session",
        lua.create_function(
            |lua, (columns, rows, show_cursor): (Option<u16>, Option<u16>, Option<bool>)| {
                lua.create_userdata(LuaTui(pi_rs_tui::tui::Tui::new(
                    pi_rs_tui::terminal::TerminalState::new(columns, rows),
                    show_cursor.unwrap_or(false),
                )))
            },
        )?,
    )?;
    tui.set(
        "process_session",
        lua.create_function(|lua, show_cursor: Option<bool>| {
            lua.create_userdata(LuaProcessTui(pi_rs_tui::process::ProcessTui::new(
                show_cursor.unwrap_or(false),
            )))
        })?,
    )?;
    tui.set("text_render", lua.create_function(|lua, (text, width, padding_x, padding_y): (String, usize, Option<usize>, Option<usize>)| {
        let lines = pi_rs_tui::component::render_text(&text, width, padding_x.unwrap_or(1), padding_y.unwrap_or(1));
        let result = lua.create_table()?;
        for line in lines { result.push(line)?; }
        Ok(result)
    })?)?;
    tui.set(
        "text",
        lua.create_function(
            |lua, (text, padding_x, padding_y): (Option<String>, Option<usize>, Option<usize>)| {
                lua.create_userdata(LuaText(Arc::new(pi_rs_tui::component::Text::new(
                    text.unwrap_or_default(),
                    padding_x.unwrap_or(0),
                    padding_y.unwrap_or(0),
                ))))
            },
        )?,
    )?;
    tui.set(
        "input",
        lua.create_function(|lua, value: Option<String>| {
            lua.create_userdata(LuaInput(Arc::new(pi_rs_tui::input::Input::new(
                value.unwrap_or_default(),
            ))))
        })?,
    )?;
    tui.set(
        "spacer",
        lua.create_function(|lua, lines: Option<usize>| {
            lua.create_userdata(LuaSpacer(Arc::new(pi_rs_tui::spacer::Spacer::new(
                lines.unwrap_or(1),
            ))))
        })?,
    )?;
    tui.set(
        "truncated_text",
        lua.create_function(
            |lua, (text, padding_x, padding_y): (Option<String>, Option<usize>, Option<usize>)| {
                lua.create_userdata(LuaTruncatedText(Arc::new(
                    pi_rs_tui::truncated_text::TruncatedText::new(
                        text.unwrap_or_default(),
                        padding_x.unwrap_or(0),
                        padding_y.unwrap_or(0),
                    ),
                )))
            },
        )?,
    )?;
    tui.set(
        "box",
        lua.create_function(
            |lua,
             (padding_x, padding_y, background): (
                Option<usize>,
                Option<usize>,
                Option<mlua::Function>,
            )| {
                let key = background
                    .map(|function| lua.create_registry_value(function))
                    .transpose()?;
                lua.create_userdata(LuaBox {
                    component: Arc::new(pi_rs_tui::box_component::BoxComponent::new(
                        padding_x.unwrap_or(1),
                        padding_y.unwrap_or(1),
                    )),
                    background: Mutex::new(key),
                })
            },
        )?,
    )?;
    tui.set(
        "settings_list",
        lua.create_function(
            |lua,
             (items, max_visible, search, theme): (
                mlua::Table,
                Option<usize>,
                Option<bool>,
                Option<mlua::Table>,
            )| {
                let mut values = Vec::new();
                for item in items.sequence_values::<mlua::Table>() {
                    let item = item?;
                    let item_values = item
                        .get::<Option<mlua::Table>>("values")?
                        .map(|values| {
                            values
                                .sequence_values()
                                .collect::<mlua::Result<Vec<String>>>()
                        })
                        .transpose()?
                        .unwrap_or_default();
                    values.push(pi_rs_tui::settings_list::SettingItem {
                        id: item.get("id")?,
                        label: item.get("label")?,
                        description: item.get("description")?,
                        current_value: item
                            .get("current_value")
                            .or_else(|_| item.get("currentValue"))?,
                        values: item_values,
                        submenu: item.get::<Option<bool>>("submenu")?.unwrap_or(false),
                    });
                }
                let mut settings_theme = pi_rs_tui::settings_list::SettingsTheme::default();
                if let Some(theme) = theme {
                    let style = |prefix: &str| -> mlua::Result<(String, String)> {
                        Ok((
                            theme
                                .get::<Option<String>>(format!("{prefix}_open"))?
                                .unwrap_or_default(),
                            theme
                                .get::<Option<String>>(format!("{prefix}_close"))?
                                .unwrap_or_default(),
                        ))
                    };
                    let (label_open, label_close) = style("label_selected")?;
                    settings_theme.label = Arc::new(move |text, selected| {
                        if selected {
                            format!("{label_open}{text}{label_close}")
                        } else {
                            text.to_owned()
                        }
                    });
                    let (value_open, value_close) = style("value")?;
                    let (selected_open, selected_close) = style("value_selected")?;
                    settings_theme.value = Arc::new(move |text, selected| {
                        if selected {
                            format!("{selected_open}{text}{selected_close}")
                        } else {
                            format!("{value_open}{text}{value_close}")
                        }
                    });
                    let (description_open, description_close) = style("description")?;
                    settings_theme.description = Arc::new(move |text| {
                        format!("{description_open}{text}{description_close}")
                    });
                    let (hint_open, hint_close) = style("hint")?;
                    settings_theme.hint =
                        Arc::new(move |text| format!("{hint_open}{text}{hint_close}"));
                    settings_theme.cursor = theme
                        .get::<Option<String>>("cursor")?
                        .unwrap_or_else(|| "→ ".to_owned());
                }
                lua.create_userdata(LuaSettingsList(Arc::new(
                    pi_rs_tui::settings_list::SettingsList::new(
                        values,
                        max_visible.unwrap_or(5),
                        settings_theme,
                        search.unwrap_or(false),
                    ),
                )))
            },
        )?,
    )?;
    tui.set(
        "markdown_render",
        lua.create_function(
            |lua,
             (text, width, padding_x, padding_y, opts): (
                String,
                usize,
                Option<usize>,
                Option<usize>,
                Option<mlua::Table>,
            )| {
                use pi_rs_tui::markdown::{
                    DefaultTextStyle, MarkdownOptions, MarkdownRenderer, MarkdownTheme, StyleFn,
                };
                let error: std::rc::Rc<std::cell::RefCell<Option<mlua::Error>>> =
                    std::rc::Rc::default();
                let style_fn = |function: mlua::Function| -> StyleFn<'static> {
                    let error = std::rc::Rc::clone(&error);
                    Box::new(move |input: &str| match function.call::<String>(input) {
                        Ok(styled) => styled,
                        Err(failure) => {
                            error.borrow_mut().get_or_insert(failure);
                            input.to_owned()
                        }
                    })
                };
                let mut theme = MarkdownTheme::plain();
                let mut default_style = DefaultTextStyle::default();
                let mut options = MarkdownOptions::default();
                if let Some(opts) = &opts {
                    if let Some(theme_table) = opts.get::<Option<mlua::Table>>("theme")? {
                        let set = |slot: &mut StyleFn<'_>, key: &str| -> mlua::Result<()> {
                            if let Some(function) =
                                theme_table.get::<Option<mlua::Function>>(key)?
                            {
                                *slot = style_fn(function);
                            }
                            Ok(())
                        };
                        set(&mut theme.heading, "heading")?;
                        set(&mut theme.link, "link")?;
                        set(&mut theme.link_url, "link_url")?;
                        set(&mut theme.code, "code")?;
                        set(&mut theme.code_block, "code_block")?;
                        set(&mut theme.code_block_border, "code_block_border")?;
                        set(&mut theme.quote, "quote")?;
                        set(&mut theme.quote_border, "quote_border")?;
                        set(&mut theme.hr, "hr")?;
                        set(&mut theme.list_bullet, "list_bullet")?;
                        set(&mut theme.bold, "bold")?;
                        set(&mut theme.italic, "italic")?;
                        set(&mut theme.strikethrough, "strikethrough")?;
                        set(&mut theme.underline, "underline")?;
                        if let Some(function) =
                            theme_table.get::<Option<mlua::Function>>("highlight_code")?
                        {
                            let error = std::rc::Rc::clone(&error);
                            theme.highlight_code = Some(Box::new(
                                move |code: &str, lang: Option<&str>| match function
                                    .call::<Vec<String>>((code, lang))
                                {
                                    Ok(lines) => lines,
                                    Err(failure) => {
                                        error.borrow_mut().get_or_insert(failure);
                                        code.split('\n').map(str::to_owned).collect()
                                    }
                                },
                            ));
                        }
                        if let Some(indent) =
                            theme_table.get::<Option<String>>("code_block_indent")?
                        {
                            theme.code_block_indent = Some(indent);
                        }
                    }
                    if let Some(function) = opts.get::<Option<mlua::Function>>("color")? {
                        default_style.color = Some(style_fn(function));
                    }
                    if let Some(function) = opts.get::<Option<mlua::Function>>("bg_color")? {
                        default_style.bg_color = Some(style_fn(function));
                    }
                    default_style.bold = opts.get::<Option<bool>>("bold")?.unwrap_or(false);
                    default_style.italic = opts.get::<Option<bool>>("italic")?.unwrap_or(false);
                    default_style.strikethrough =
                        opts.get::<Option<bool>>("strikethrough")?.unwrap_or(false);
                    default_style.underline =
                        opts.get::<Option<bool>>("underline")?.unwrap_or(false);
                    options.preserve_ordered_list_markers = opts
                        .get::<Option<bool>>("preserve_ordered_list_markers")?
                        .unwrap_or(false);
                }
                let has_default_style = default_style.color.is_some()
                    || default_style.bg_color.is_some()
                    || default_style.bold
                    || default_style.italic
                    || default_style.strikethrough
                    || default_style.underline;
                let renderer = MarkdownRenderer::new(
                    &theme,
                    has_default_style.then_some(&default_style),
                    options,
                );
                let lines =
                    renderer.render(&text, width, padding_x.unwrap_or(0), padding_y.unwrap_or(0));
                if let Some(failure) = error.borrow_mut().take() {
                    return Err(failure);
                }
                let result = lua.create_table()?;
                for line in lines {
                    result.push(line)?;
                }
                Ok(result)
            },
        )?,
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
        "fuzzy_filter",
        lua.create_function(
            |lua, (items, query, get_text): (mlua::Table, String, mlua::Function)| {
                let mut pairs: Vec<(mlua::Value, String)> = Vec::new();
                for item in items.sequence_values::<mlua::Value>() {
                    let item = item?;
                    let text: String = get_text.call(item.clone())?;
                    pairs.push((item, text));
                }
                let filtered =
                    pi_rs_tui::fuzzy::fuzzy_filter(pairs, &query, |(_, text)| text.clone());
                let result = lua.create_table()?;
                for (item, _) in filtered {
                    result.push(item)?;
                }
                Ok(result)
            },
        )?,
    )?;
    // Spec: pi-tui `fuzzyMatch(query, text)` — the single-token match the
    // session-selector search scores per token (`matchSession`).
    tui.set(
        "fuzzy_match",
        lua.create_function(|lua, (query, text): (String, String)| {
            let result = pi_rs_tui::fuzzy::fuzzy_match(&query, &text);
            let table = lua.create_table()?;
            table.set("matches", result.matches)?;
            table.set("score", result.score)?;
            Ok(table)
        })?,
    )?;
    // JS `text.search(new RegExp(pattern, "i"))` as a mechanism binding
    // (the session selector's `re:` search mode). Returns the JS string
    // index (UTF-16 units) or nil for no match; an invalid pattern
    // returns (nil, message) like the spec's caught `new RegExp` error.
    tui.set(
        "js_regex_search",
        lua.create_function(|_, (pattern, text): (String, String)| {
            let regex = match crate::hljs::js_regex(&pattern, true) {
                Ok(regex) => regex,
                Err(error) => {
                    return Ok((None, Some(error.to_string())));
                }
            };
            match regex.find(&text) {
                Ok(Some((start, _))) => {
                    let index = text[..start].encode_utf16().count();
                    Ok((Some(index), None))
                }
                Ok(None) | Err(_) => Ok((None, None)),
            }
        })?,
    )?;
    tui.set(
        "differential_render",
        lua.create_function(
            |_, (previous, lines, clear): (mlua::Table, mlua::Table, Option<bool>)| {
                let previous: Vec<String> =
                    previous.sequence_values().collect::<mlua::Result<_>>()?;
                let lines: Vec<String> = lines.sequence_values().collect::<mlua::Result<_>>()?;
                Ok(pi_rs_tui::component::differential_render(
                    &previous,
                    &lines,
                    clear.unwrap_or(false),
                ))
            },
        )?,
    )?;
    tui.set(
        "autocomplete_provider",
        lua.create_function(|lua, options: mlua::Table| {
            let mut commands = Vec::new();
            let mut argument_completions = std::collections::HashMap::new();
            if let Ok(Some(list)) = options.get::<Option<mlua::Table>>("commands") {
                for command in list.sequence_values::<mlua::Table>() {
                    let command = command?;
                    let name: String = command.get("name").or_else(|_| command.get("value"))?;
                    let callback: Option<mlua::Function> =
                        command.get("get_argument_completions")?;
                    commands.push(pi_rs_tui::autocomplete::SlashCommand {
                        name: name.clone(),
                        description: command.get("description")?,
                        argument_hint: command.get("argument_hint")?,
                        has_argument_completions: callback.is_some(),
                    });
                    if let Some(callback) = callback {
                        argument_completions.insert(name, callback);
                    }
                }
            }
            let provider = pi_rs_tui::autocomplete::CombinedProvider {
                commands,
                base_path: options.get("base_path")?,
                fd_path: options.get("fd_path")?,
            };
            lua.create_userdata(LuaAutocompleteProvider {
                provider,
                argument_completions,
            })
        })?,
    )?;
    tui.set(
        "decode_key",
        lua.create_function(|_, data: String| Ok(pi_rs_tui::editor::decode_key(&data)))?,
    )?;
    tui.set(
        "editor",
        lua.create_function(|lua, value: Option<String>| {
            lua.create_userdata(LuaEditor(pi_rs_tui::editor::Editor::new(
                value.unwrap_or_default(),
            )))
        })?,
    )?;
    tui.set(
        "select_list",
        lua.create_function(
            |lua, (items, max_visible, opts): (mlua::Table, Option<usize>, Option<mlua::Table>)| {
                let mut values = Vec::new();
                for item in items.sequence_values::<mlua::Table>() {
                    let item = item?;
                    values.push(pi_rs_tui::select_list::SelectItem {
                        value: item.get("value")?,
                        label: item.get("label")?,
                        description: item.get("description")?,
                    });
                }
                let mut theme = pi_rs_tui::select_list::SelectListTheme::default();
                let mut layout = pi_rs_tui::select_list::SelectListLayout::default();
                if let Some(opts) = opts {
                    let style = |name: &str| -> mlua::Result<pi_rs_tui::select_list::Style> {
                        Ok(pi_rs_tui::select_list::Style {
                            open: opts
                                .get::<Option<String>>(format!("{name}_open"))?
                                .unwrap_or_default(),
                            close: opts
                                .get::<Option<String>>(format!("{name}_close"))?
                                .unwrap_or_default(),
                        })
                    };
                    theme.selected_text = style("selected")?;
                    theme.description = style("description")?;
                    theme.scroll_info = style("scroll")?;
                    theme.no_match = style("no_match")?;
                    layout.min_primary_column_width =
                        opts.get::<Option<usize>>("min_primary_column_width")?;
                    layout.max_primary_column_width =
                        opts.get::<Option<usize>>("max_primary_column_width")?;
                }
                lua.create_userdata(LuaSelectList(
                    pi_rs_tui::select_list::SelectList::with_theme(
                        values,
                        max_visible.unwrap_or(5),
                        theme,
                        layout,
                    ),
                ))
            },
        )?,
    )?;
    tui.set(
        "loader",
        lua.create_function(
            |lua, (message, indicator): (Option<String>, Option<mlua::Table>)| {
                lua.create_userdata(LuaLoader(pi_rs_tui::loader::Loader::new(
                    message.unwrap_or_else(|| "Loading...".to_owned()),
                    loader_indicator(indicator)?,
                )))
            },
        )?,
    )?;
    tui.set(
        "cancellable_loader",
        lua.create_function(
            |lua, (message, indicator): (Option<String>, Option<mlua::Table>)| {
                lua.create_userdata(LuaCancellableLoader(
                    pi_rs_tui::loader::CancellableLoader::new(
                        message.unwrap_or_else(|| "Loading...".to_owned()),
                        loader_indicator(indicator)?,
                    ),
                ))
            },
        )?,
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
                let max_width_cells = options
                    .as_ref()
                    .map(|table| table.get::<Option<u32>>("max_width_cells"))
                    .transpose()?
                    .flatten();
                let max_height_cells = options
                    .as_ref()
                    .map(|table| table.get::<Option<u32>>("max_height_cells"))
                    .transpose()?
                    .flatten();
                let preserve_aspect_ratio = options
                    .as_ref()
                    .map(|table| table.get::<Option<bool>>("preserve_aspect_ratio"))
                    .transpose()?
                    .flatten();
                let image_id = options
                    .as_ref()
                    .map(|table| table.get::<Option<u32>>("image_id"))
                    .transpose()?
                    .flatten();
                let move_cursor = options
                    .as_ref()
                    .map(|table| table.get::<Option<bool>>("move_cursor"))
                    .transpose()?
                    .flatten();
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
    tui.set(
        "decode_key",
        lua.create_function(|_, data: String| Ok(pi_rs_tui::editor::decode_key(&data)))?,
    )?;
    tui.set(
        "decode_printable",
        lua.create_function(|_, data: String| Ok(pi_rs_tui::editor::decode_printable(&data)))?,
    )?;
    pi.set("tui", tui)?;

    // One `AuthStorage` per VM (the spec: one per process), shared by
    // the `pi.auth` bindings, the `pi.ai` registry bridge, and login flows.
    let storage: crate::auth::SharedStorage = std::sync::Arc::new(tokio::sync::Mutex::new(
        crate::auth_storage::AuthStorage::create(None),
    ));
    crate::ai::install(lua, &pi, std::sync::Arc::clone(&storage))?;
    crate::auth::install(lua, &pi, storage)?;
    crate::exec::install(lua, &pi, cwd)?;
    crate::http::install(lua, &pi)?;
    crate::os::install(lua, &pi, cwd)?;
    let settings = crate::settings::install(lua, &pi, cwd, project_trusted)?;
    crate::config::install_runtime(lua, &pi, cwd, project_trusted, settings)?;
    crate::session::install(lua, &pi, cwd)?;
    crate::trust::install(lua, &pi)?;
    crate::clipboard::install(lua, &pi)?;

    Ok(pi)
}

/// Snapshot the handler list for `event` before dispatching so a handler
/// that subscribes new handlers mid-emit doesn't alter this dispatch.
pub(crate) fn event_handlers(
    lua: &mlua::Lua,
    event: &str,
) -> mlua::Result<Vec<(String, mlua::Function)>> {
    let events: mlua::Table = registry_table(lua)?.get("events")?;
    let Some(list) = events.get::<Option<mlua::Table>>(event)? else {
        return Ok(Vec::new());
    };
    let mut handlers = Vec::with_capacity(list.raw_len());
    for entry in list.sequence_values::<mlua::Table>() {
        let entry = entry?;
        let source: String = entry
            .get("source")
            .unwrap_or_else(|_| "<unknown>".to_owned());
        let handler: mlua::Function = entry.get("fn")?;
        handlers.push((source, handler));
    }
    Ok(handlers)
}

/// Roll back every registration attributed to a source whose top-level chunk
/// failed. Pi constructs an extension off-registry and publishes it only after
/// its async factory resolves; this gives direct Lua chunks the same atomicity.
pub(crate) fn remove_source(lua: &mlua::Lua, source: &str) -> mlua::Result<()> {
    crate::kernel_api::remove_source(lua, source)?;
    let registry = registry_table(lua)?;
    let exts: mlua::Table = registry.get("exts")?;
    exts.set(source, mlua::Nil)?;

    let order: mlua::Table = registry.get("ext_order")?;
    let kept = lua.create_table()?;
    for entry in order.sequence_values::<String>() {
        let entry = entry?;
        if entry != source {
            kept.push(entry)?;
        }
    }
    registry.set("ext_order", kept)?;

    remove_source_listeners(lua, &registry, "events", source)?;
    remove_source_listeners(lua, &registry, "bus_listeners", source)
}

fn remove_source_listeners(
    lua: &mlua::Lua,
    registry: &mlua::Table,
    table_key: &str,
    source: &str,
) -> mlua::Result<()> {
    let listeners: mlua::Table = registry.get(table_key)?;
    let names = listeners
        .pairs::<String, mlua::Table>()
        .map(|pair| pair.map(|(name, _)| name))
        .collect::<mlua::Result<Vec<_>>>()?;
    for name in names {
        let list: mlua::Table = listeners.get(name.as_str())?;
        let kept = lua.create_table()?;
        for entry in list.sequence_values::<mlua::Table>() {
            let entry = entry?;
            if entry.get::<String>("source")? != source {
                kept.push(entry)?;
            }
        }
        listeners.set(name, kept)?;
    }

    // Module definitions are part of the same publication transaction. Drop
    // this source's definitions and invalidate all surviving values because a
    // cached dependent may have captured one of the removed exports.
    let modules: mlua::Table = registry.get("modules")?;
    let module_order: mlua::Table = registry.get("module_order")?;
    let kept_modules = lua.create_table()?;
    for key in module_order.sequence_values::<String>() {
        let key = key?;
        let Some(entry) = modules.get::<Option<mlua::Table>>(key.as_str())? else {
            continue;
        };
        if entry.get::<String>("source")? == source {
            modules.set(key.as_str(), mlua::Nil)?;
        } else {
            entry.set("state", "defined")?;
            entry.set("value", mlua::Nil)?;
            kept_modules.push(key)?;
        }
    }
    registry.set("module_order", kept_modules)?;
    registry.set("module_stack", lua.create_table()?)?;
    Ok(())
}

/// Flatten registrations of one kind: extension order (load order), then
/// per-extension insertion order — the iteration order of the spec's
/// nested `Map`s.
fn registrations(
    lua: &mlua::Lua,
    map_key: &str,
    order_key: &str,
) -> mlua::Result<Vec<(String, String, mlua::Table)>> {
    let registry = registry_table(lua)?;
    let exts: mlua::Table = registry.get("exts")?;
    let ext_order: mlua::Table = registry.get("ext_order")?;
    let mut out = Vec::new();
    for source in ext_order.sequence_values::<String>() {
        let source = source?;
        let Some(ext) = exts.get::<Option<mlua::Table>>(source.as_str())? else {
            continue;
        };
        let map: mlua::Table = ext.get(map_key)?;
        let order: mlua::Table = ext.get(order_key)?;
        for name in order.sequence_values::<String>() {
            let name = name?;
            if let Some(entry) = map.get::<Option<mlua::Table>>(name.as_str())? {
                out.push((source.clone(), name, entry));
            }
        }
    }
    Ok(out)
}

/// Spec `runner.getAllRegisteredTools()`: first registration per name
/// wins across extensions. Returns `(source, name, definition)`.
pub(crate) fn all_tools(lua: &mlua::Lua) -> mlua::Result<Vec<(String, String, mlua::Table)>> {
    let mut seen = HashSet::new();
    Ok(registrations(lua, "tools", "tool_order")?
        .into_iter()
        .filter(|(_, name, _)| seen.insert(name.clone()))
        .collect())
}

pub(crate) fn extension_conflicts(lua: &mlua::Lua) -> mlua::Result<Vec<(String, String)>> {
    let registry = registry_table(lua)?;
    let exts: mlua::Table = registry.get("exts")?;
    let ext_order: mlua::Table = registry.get("ext_order")?;
    let mut tool_owners = std::collections::HashMap::<String, String>::new();
    let mut flag_owners = std::collections::HashMap::<String, String>::new();
    let mut conflicts = Vec::new();
    for source in ext_order.sequence_values::<String>() {
        let source = source?;

        let Some(ext) = exts.get::<Option<mlua::Table>>(source.as_str())? else {
            continue;
        };
        for (map_key, order_key, label, decoration, owners) in [
            ("tools", "tool_order", "Tool", "", &mut tool_owners),
            ("flags", "flag_order", "Flag", "--", &mut flag_owners),
        ] {
            let map: mlua::Table = ext.get(map_key)?;
            let order: mlua::Table = ext.get(order_key)?;
            for name in order.sequence_values::<String>() {
                let name = name?;
                if map.get::<Option<mlua::Value>>(name.as_str())?.is_none() {
                    continue;
                }
                if let Some(owner) = owners.get(&name) {
                    if owner != &source {
                        conflicts.push((
                            source.clone(),
                            format!("{label} \"{decoration}{name}\" conflicts with {owner}"),
                        ));
                    }
                } else {
                    owners.insert(name, source.clone());
                }
            }
        }
    }
    Ok(conflicts)
}

/// Spec `ExtensionRunner.getFlags()`: first registration per name wins.
pub(crate) fn all_flags(lua: &mlua::Lua) -> mlua::Result<Vec<(String, String, mlua::Table)>> {
    let mut seen = HashSet::new();
    Ok(registrations(lua, "flags", "flag_order")?
        .into_iter()
        .filter(|(_, name, _)| seen.insert(name.clone()))
        .collect())
}

pub(crate) fn set_flag_value(
    lua: &mlua::Lua,
    name: &str,
    value: &serde_json::Value,
) -> mlua::Result<()> {
    let values: mlua::Table = registry_table(lua)?.get("flag_values")?;
    values.set(name, crate::convert::json_to_lua(lua, value)?)
}

/// Spec `runner.getToolDefinition()`: the first extension that registered
/// the name.
pub(crate) fn find_tool(
    lua: &mlua::Lua,
    name: &str,
) -> mlua::Result<Option<(String, mlua::Table)>> {
    for (source, n, def) in all_tools(lua)? {
        if n == name {
            return Ok(Some((source, def)));
        }
    }
    Ok(None)
}

/// JSON metadata mirror of a tool definition: every field except
/// functions (`execute`, `prepare_arguments`) — handed to the host
/// uninterpreted.
pub(crate) fn tool_meta(def: &mlua::Table) -> mlua::Result<serde_json::Value> {
    let mut map = serde_json::Map::new();
    for pair in def.pairs::<mlua::Value, mlua::Value>() {
        let (k, v) = pair?;
        if matches!(v, mlua::Value::Function(_)) {
            continue;
        }
        let key = match k {
            mlua::Value::String(s) => s.to_str()?.to_owned(),
            mlua::Value::Integer(i) => i.to_string(),
            _ => continue,
        };
        map.insert(key, crate::convert::lua_to_json(v)?);
    }
    Ok(serde_json::Value::Object(map))
}

/// All provider registrations in extension load order, then
/// per-extension registration order (spec: the runner drains queued
/// `registerProvider` calls in order; the consumer applies the spec's
/// global upsert). Returns `(source, name, config)`.
pub(crate) fn all_providers(lua: &mlua::Lua) -> mlua::Result<Vec<(String, String, mlua::Table)>> {
    registrations(lua, "providers", "provider_order")
}

/// JSON mirror of a provider config: function values are stripped at
/// any depth (functions never cross the bridge — `streamSimple` and the
/// `oauth` callbacks stay Lua-side for later invocation; non-function
/// fields like `oauth.name` survive).
pub(crate) fn provider_meta(config: &mlua::Table) -> mlua::Result<serde_json::Value> {
    let mut map = serde_json::Map::new();
    for pair in config.pairs::<mlua::Value, mlua::Value>() {
        let (k, v) = pair?;
        if matches!(v, mlua::Value::Function(_)) {
            continue;
        }
        let key = match k {
            mlua::Value::String(s) => s.to_str()?.to_owned(),
            mlua::Value::Integer(i) => i.to_string(),
            _ => continue,
        };
        let value = match v {
            mlua::Value::Table(t) => provider_table_value(&t)?,
            other => crate::convert::lua_to_json(other)?,
        };
        map.insert(key, value);
    }
    Ok(serde_json::Value::Object(map))
}

/// [`provider_meta`] recursion: array tables keep their shape (function
/// entries would shift positions, so they become `null`); map tables
/// drop function values.
fn provider_table_value(t: &mlua::Table) -> mlua::Result<serde_json::Value> {
    let len = t.raw_len();
    let mut is_array = len > 0;
    if is_array {
        let mut count = 0;
        for pair in t.pairs::<mlua::Value, mlua::Value>() {
            let (k, _) = pair?;
            match k {
                mlua::Value::Integer(i) if i >= 1 && (i as usize) <= len => count += 1,
                _ => {
                    is_array = false;
                    break;
                }
            }
        }
        if is_array {
            is_array = count == len;
        }
    }
    if is_array {
        let mut arr = Vec::with_capacity(len);
        for i in 1..=len {
            let v: mlua::Value = t.get(i)?;
            arr.push(match v {
                mlua::Value::Function(_) => serde_json::Value::Null,
                mlua::Value::Table(inner) => provider_table_value(&inner)?,
                other => crate::convert::lua_to_json(other)?,
            });
        }
        return Ok(serde_json::Value::Array(arr));
    }
    provider_meta(t)
}

pub(crate) struct ResolvedCommand {
    pub(crate) source: String,
    pub(crate) name: String,
    pub(crate) invocation_name: String,
    pub(crate) description: Option<String>,
    pub(crate) get_argument_completions: Option<mlua::Function>,
    pub(crate) handler: mlua::Function,
    pub(crate) entry: mlua::Table,
}

/// Spec `runner.resolveRegisteredCommands()`: flatten in extension order;
/// names registered by more than one extension get `name:occurrence`
/// invocation names, bumping the suffix past already-taken names.
pub(crate) fn resolved_commands(lua: &mlua::Lua) -> mlua::Result<Vec<ResolvedCommand>> {
    let regs = registrations(lua, "commands", "command_order")?;
    let mut counts: HashMap<String, usize> = HashMap::new();
    for (_, name, _) in &regs {
        *counts.entry(name.clone()).or_default() += 1;
    }
    let mut seen: HashMap<String, usize> = HashMap::new();
    let mut taken: HashSet<String> = HashSet::new();
    let mut out = Vec::with_capacity(regs.len());
    for (source, name, entry) in regs {
        let occurrence = seen.get(&name).copied().unwrap_or(0) + 1;
        seen.insert(name.clone(), occurrence);
        let mut invocation_name = if counts.get(&name).copied().unwrap_or(0) > 1 {
            format!("{name}:{occurrence}")
        } else {
            name.clone()
        };
        if taken.contains(&invocation_name) {
            let mut suffix = occurrence;
            loop {
                suffix += 1;
                invocation_name = format!("{name}:{suffix}");
                if !taken.contains(&invocation_name) {
                    break;
                }
            }
        }
        taken.insert(invocation_name.clone());
        let description = entry.get::<Option<String>>("description")?;
        let get_argument_completions =
            entry.get::<Option<mlua::Function>>("get_argument_completions")?;
        let handler: mlua::Function = entry.get("handler")?;
        out.push(ResolvedCommand {
            source,
            name,
            invocation_name,
            description,
            get_argument_completions,
            handler,
            entry,
        });
    }
    Ok(out)
}

fn command_is_public(lua: &mlua::Lua, source: &str, command: &mlua::Table) -> mlua::Result<bool> {
    if let Some(visibility) = command.get::<Option<String>>("visibility")? {
        return Ok(visibility != "internal");
    }
    let registry = registry_table(lua)?;
    let exts: mlua::Table = registry.get("exts")?;
    let Some(ext) = exts.get::<Option<mlua::Table>>(source)? else {
        return Ok(true);
    };
    let package: mlua::Table = ext.get("package")?;
    Ok(package.get::<String>("command_visibility")? != "internal")
}

pub(crate) fn all_roles(lua: &mlua::Lua) -> mlua::Result<Vec<(String, String, mlua::Table)>> {
    registrations(lua, "roles", "role_order")
}

pub(crate) struct ResolvedRole {
    pub(crate) source: String,
    pub(crate) handler: mlua::Function,
}

pub(crate) fn resolve_role(lua: &mlua::Lua, requested: &str) -> mlua::Result<Option<ResolvedRole>> {
    let mut candidates = Vec::new();
    for (source, _, declaration) in all_roles(lua)? {
        if declaration.get::<String>("role")? == requested && declaration.get::<bool>("active")? {
            let priority = declaration.get::<i64>("priority")?;
            let id = declaration.get::<String>("id")?;
            candidates.push((priority, source, id, declaration));
        }
    }
    let Some(max_priority) = candidates.iter().map(|candidate| candidate.0).max() else {
        return Ok(None);
    };
    let mut selected = candidates
        .into_iter()
        .filter(|candidate| candidate.0 == max_priority);
    let Some((_, source, id, declaration)) = selected.next() else {
        return Ok(None);
    };
    if let Some((_, other_source, other_id, _)) = selected.next() {
        return Err(mlua::Error::runtime(format!(
            "role {requested:?} has conflicting active declarations at priority {max_priority}: {id:?} ({source}) and {other_id:?} ({other_source})"
        )));
    }
    Ok(Some(ResolvedRole {
        source,
        handler: declaration.get("handler")?,
    }))
}
