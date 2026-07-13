//! Root table assembly for independent mechanism and compatibility modules.

use crate::runtime_registry::REGISTRY_KEY;

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

pub(crate) fn rendered_lines(lua: &mlua::Lua, lines: Vec<String>) -> mlua::Result<mlua::Table> {
    let result = lua.create_table()?;
    for line in lines {
        result.push(line)?;
    }
    Ok(result)
}

pub(crate) fn build(
    lua: &mlua::Lua,
    cwd: &str,
    project_trusted: bool,
    control: std::sync::Arc<crate::kernel::Control>,
) -> mlua::Result<mlua::Table> {
    let registry = lua.create_table()?;
    registry.set("flag_values", lua.create_table()?)?;
    registry.set("flag_value_owners", lua.create_table()?)?;
    registry.set("package_options", lua.create_table()?)?;
    registry.set("modules", lua.create_table()?)?;
    registry.set("module_order", lua.create_table()?)?;
    registry.set("module_stack", lua.create_table()?)?;
    registry.set("adapter_sequence", 0_u64)?;
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

    let module_api = crate::module_api::install(lua, &pi)?;
    crate::kernel_api::install(lua, &pi, &module_api, control)?;
    crate::compatibility::install(lua, &pi)?;
    crate::runtime_api::install(lua, &pi)?;
    crate::tui_api::install(lua, &pi)?;
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
