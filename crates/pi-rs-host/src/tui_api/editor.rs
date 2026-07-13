use crate::bindings::rendered_lines;
use mlua::{UserData, UserDataMethods};

pub(crate) struct LuaAutocompleteProvider {
    pub(crate) provider: pi_rs_tui::autocomplete::CombinedProvider,
    pub(crate) argument_completions: std::collections::HashMap<String, mlua::Function>,
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

pub(crate) struct LuaEditor(pub(crate) pi_rs_tui::editor::Editor);
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
