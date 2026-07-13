//! Policy-free terminal/display mechanism bindings.

mod components;
mod editor;
pub(crate) mod runtime;

use components::*;
use editor::*;
use runtime::*;
use std::sync::{Arc, Mutex};

pub(crate) fn install(lua: &mlua::Lua, pi: &mlua::Table) -> mlua::Result<()> {
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

    Ok(())
}
