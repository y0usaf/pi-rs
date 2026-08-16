//! A.3 Rust owner for the closed Pi v0.79.0 extension-surface inventory.
//!
//! Faithful port of the former `scripts/extension-inventory` (Python). Normal
//! checks consume the checked `pinned-surface.json` and manifest (and, when
//! `ref/pi` is present, cross-check the pinned surface against it), then
//! validate the fail-closed manifest and render/compare
//! `EXTENSION_INVENTORY.md` and `docs/lua-extension-api.md`. No repo-owned
//! Python/Node runtime is required.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use fancy_regex::Regex;

#[derive(Debug, thiserror::Error)]
pub enum ExtInvError {
    #[error("extension inventory: {0}")]
    Message(String),
    #[error("extension inventory: {0}")]
    Json(#[from] serde_json::Error),
    #[error("extension inventory: {0}")]
    Io(#[from] std::io::Error),
}

type Result<T> = std::result::Result<T, ExtInvError>;

const ORACLE: &str = "Pi v0.79.0 c5582102";

fn fail<T>(msg: impl Into<String>) -> Result<T> {
    Err(ExtInvError::Message(msg.into()))
}

fn re(pattern: &str) -> Result<Regex> {
    Regex::new(pattern).map_err(|e| ExtInvError::Message(format!("invalid regex {pattern:?}: {e}")))
}

/// Extract the brace-delimited body of `export interface NAME ... { ... }`,
/// ignoring comments. Mirrors the Python `interface_body`.
fn interface_body(source: &str, name: &str) -> Result<String> {
    let pat = re(&format!(
        r"export interface {}\s*(?:\s+extends\s+[^{{]+)?\s*{{",
        fancy_regex::escape(name)
    ))?;
    let caps = pat
        .captures(source)
        .map_err(|e| ExtInvError::Message(format!("regex: {e}")))?
        .ok_or_else(|| ExtInvError::Message(format!("missing interface {name}")))?;
    let m = caps.get(0).ok_or_else(|| ExtInvError::Message("internal captures".into()))?;
    let start = m.end();
    let mut depth = 1usize;
    let mut i = start;
    while i < source.len() && depth > 0 {
        if source[i..].starts_with("//") {
            match source[i..].find('\n') {
                Some(n) => i += n + 1,
                None => i = source.len(),
            }
        } else if source[i..].starts_with("/*") {
            match source[i..].find("*/") {
                Some(n) => {
                    i += n + 2;
                }
                None => {
                    i = source.len();
                }
            }
        } else {
            let ch = source[i..].chars().next().unwrap_or('\0');
            match ch {
                '{' => depth += 1,
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        return Ok(source[start..i].to_owned());
                    }
                }
                _ => {}
            }
            i += 1;
        }
    }
    if depth > 0 {
        return fail(format!("unterminated interface {name}"));
    }
    Ok(source[start..].to_owned())
}

/// Parse the members of an interface body (top-level statements ending in `;`).
fn interface_members(source: &str, name: &str) -> Result<Vec<String>> {
    let body = interface_body(source, name)?;
    // Strip block and line comments.
    let comments = re(r"/\*[\s\S]*?\*/|//[^\n]*")?;
    let body = comments.replace_all(&body, "").to_string();
    let member_re = re(
        r"(?:readonly\s+)?([A-Za-z_$][\w$]*)\??\s*(?:<[^;]+?>)?\s*(?:\(|:)",
    )?;
    let mut members: BTreeSet<String> = BTreeSet::new();
    let mut start = 0usize;
    let mut braces = 0i32;
    let mut parens = 0i32;
    let mut brackets = 0i32;
    let chars: Vec<char> = body.chars().collect();
    let mut i = 0usize;
    while i < chars.len() {
        match chars[i] {
            '{' => braces += 1,
            '}' => braces -= 1,
            '(' => parens += 1,
            ')' => parens -= 1,
            '[' => brackets += 1,
            ']' => brackets -= 1,
            ';' if braces == 0 && parens == 0 && brackets == 0 => {
                let statement: String = chars[start..i].iter().collect();
                let statement = statement.trim();
                if let Some(caps) = member_re
                    .captures(statement)
                    .map_err(|e| ExtInvError::Message(format!("regex: {e}")))?
                    && let Some(g) = caps.get(1)
                {
                    members.insert(g.as_str().to_owned());
                }
                start = i + 1;
            }
            _ => {}
        }
        i += 1;
    }
    Ok(members.into_iter().collect())
}

/// Extract the pinned extension surface from the `ref/pi` checkout.
pub fn extract_from_ref(root: &Path) -> Result<serde_json::Value> {
    let base = root.join("ref/pi/packages/coding-agent/src/core/extensions");
    let types = std::fs::read_to_string(base.join("types.ts"))?;
    let loader = std::fs::read_to_string(base.join("loader.ts"))?;
    let resource = std::fs::read_to_string(
        root.join("ref/pi/packages/coding-agent/src/core/resource-loader.ts"),
    )?;
    let runner = std::fs::read_to_string(base.join("runner.ts"))?;

    let event_iface_re = re(r"export interface ([A-Za-z0-9_]*Event)\b")?;
    let type_re = re(r#"\btype:\s*"([^"]+)""#)?;
    let mut events: BTreeSet<String> = BTreeSet::new();
    for caps in event_iface_re.captures_iter(&types) {
        let caps = caps.map_err(|e| ExtInvError::Message(format!("regex: {e}")))?;
        if let Some(g) = caps.get(1) {
            let body = interface_body(&types, g.as_str())?;
            if let Some(tcaps) = type_re
                .captures(&body)
                .map_err(|e| ExtInvError::Message(format!("regex: {e}")))?
                && let Some(tg) = tcaps.get(1)
            {
                events.insert(tg.as_str().to_owned());
            }
        }
    }
    let alias_re = re(r"export type (ToolCallEvent|ToolResultEvent)\s*=")?;
    if alias_re.is_match(&types).map_err(|e| ExtInvError::Message(format!("regex: {e}")))? {
        // aliases present
    } else {
        return fail("missing event alias");
    }
    let has_call = re(r"export type ToolCallEvent\s*=")?.is_match(&types).map_err(|e| ExtInvError::Message(format!("regex: {e}")))?;
    let has_result = re(r"export type ToolResultEvent\s*=")?.is_match(&types).map_err(|e| ExtInvError::Message(format!("regex: {e}")))?;
    if !has_call {
        return fail("missing event alias ToolCallEvent");
    }
    if !has_result {
        return fail("missing event alias ToolResultEvent");
    }
    events.insert("tool_call".to_owned());
    events.insert("tool_result".to_owned());

    let mut obj = serde_json::Map::new();
    obj.insert("events".to_owned(), serde_json::json!(events.iter().collect::<Vec<_>>()));
    for (key, iface) in [
        ("api", "ExtensionAPI"),
        ("ui", "ExtensionUIContext"),
        ("context", "ExtensionContext"),
        ("command_context", "ExtensionCommandContext"),
    ] {
        let members = interface_members(&types, iface)?;
        obj.insert(key.to_owned(), serde_json::json!(members));
    }

    let rules: BTreeSet<&str> = [
        "direct .lua files",
        "subdirectory init.lua entry",
        "project-local before global",
        "configured/CLI path resolution",
        "resolved-path deduplication",
        "project trust gate",
        "disable discovery but retain CLI paths",
        "isolated load failures",
        "async extension initialization",
        "per-extension attribution",
        "tool/flag conflict diagnostics",
        "first-registration tool precedence",
        "duplicate command disambiguation",
    ]
    .into_iter()
    .collect();
    obj.insert("loader_rules".to_owned(), serde_json::json!(rules.iter().collect::<Vec<_>>()));

    // Anchor checks: fail if the pinned implementation anchors backing the
    // rule inventory move.
    let anchors: &[(&str, &[&str])] = &[
        (
            "loader.ts",
            &[
                "async function loadExtension(",
                "await factory(api)",
                "export async function loadExtensions(",
                "function discoverExtensionsInDir(",
                "const localExtDir",
                "const globalExtDir",
            ],
        ),
        (
            "resource-loader.ts",
            &[
                "this.noExtensions",
                "loadProjectTrustExtensions",
                "addExtensionConflictDiagnostics",
                "detectExtensionConflicts",
                "loadFinalExtensionSet",
            ],
        ),
        (
            "runner.ts",
            &[
                "getAllRegisteredTools()",
                "getShortcuts(",
                "resolveRegisteredCommands()",
                "emitToolCall(",
                "emitToolResult(",
            ],
        ),
    ];
    for (file_name, needles) in anchors {
        let haystack = match *file_name {
            "loader.ts" => &loader,
            "resource-loader.ts" => &resource,
            _ => &runner,
        };
        for needle in *needles {
            if !haystack.contains(needle) {
                return fail(format!("missing pinned {file_name} anchor: {needle}"));
            }
        }
    }

    let examples_dir = root.join("ref/pi/packages/coding-agent/examples/extensions");
    let mut examples: Vec<String> = Vec::new();
    collect_ts_examples(&examples_dir, &examples_dir, &mut examples)?;
    examples.sort();
    obj.insert("examples".to_owned(), serde_json::json!(examples));

    Ok(serde_json::Value::Object(obj))
}

fn collect_ts_examples(dir: &Path, base: &Path, out: &mut Vec<String>) -> Result<()> {
    let mut entries: Vec<PathBuf> = std::fs::read_dir(dir)?.filter_map(|e| e.ok().map(|e| e.path())).collect();
    entries.sort();
    for entry in entries {
        let ft = entry.metadata()?;
        if ft.is_dir() {
            collect_ts_examples(&entry, base, out)?;
        } else if let Some(ext) = entry.extension()
            && ext == "ts"
            && entry.file_name().and_then(|n| n.to_str()).map(|n| !n.ends_with(".test.ts") && n != "test.ts").unwrap_or(false)
        {
            let rel = entry.strip_prefix(base).map_err(|_| ExtInvError::Message("strip_prefix".into()))?;
            out.push(rel.components().map(|c| c.as_os_str().to_string_lossy()).collect::<Vec<_>>().join("/"));
        }
    }
    Ok(())
}

fn as_obj<'a>(value: &'a serde_json::Value, what: &str) -> Result<&'a serde_json::Map<String, serde_json::Value>> {
    value
        .as_object()
        .ok_or_else(|| ExtInvError::Message(format!("{what}: expected an object")))
}

/**
 * Extract the surface: load the checked pinned-surface.json and, when the
 * `ref/pi` checkout is present, cross-check it against `extract_from_ref`.
 */
pub fn extract(root: &Path) -> Result<serde_json::Value> {
    let fixture_path = root.join("tests/extension-inventory/pinned-surface.json");
    let fixture: serde_json::Value = serde_json::from_slice(&std::fs::read(&fixture_path)?)?;
    if fixture.get("oracle").and_then(|v| v.as_str()) != Some(ORACLE) {
        return fail("pinned surface oracle must be Pi v0.79.0 c5582102");
    }
    let surface = fixture
        .get("surface")
        .cloned()
        .ok_or_else(|| ExtInvError::Message("pinned surface fixture has no surface object".into()))?;
    let types_path = root.join("ref/pi/packages/coding-agent/src/core/extensions/types.ts");
    if types_path.exists() {
        let actual = extract_from_ref(root)?;
        if !py_obj_equal(&actual, &surface) {
            return fail("pinned surface fixture differs from ref/pi; regenerate from the pinned revision");
        }
    }
    Ok(surface)
}

/// Python-dict-style equality (ignore object key order).
fn py_obj_equal(a: &serde_json::Value, b: &serde_json::Value) -> bool {
    match (a, b) {
        (serde_json::Value::Object(m1), serde_json::Value::Object(m2)) => {
            if m1.len() != m2.len() {
                return false;
            }
            m1.iter().all(|(k, v)| m2.get(k).map(|w| py_obj_equal(v, w)).unwrap_or(false))
        }
        (serde_json::Value::Array(a1), serde_json::Value::Array(a2)) => {
            a1.len() == a2.len() && a1.iter().zip(a2.iter()).all(|(x, y)| py_obj_equal(x, y))
        }
        _ => a == b,
    }
}

/// Return the array stored under `value[key]`, or an empty slice when absent.
fn key_slice<'a>(value: &'a serde_json::Value, key: &str) -> &'a [serde_json::Value] {
    static EMPTY: Vec<serde_json::Value> = Vec::new();
    value.get(key).and_then(|v| v.as_array()).map(|a| a.as_slice()).unwrap_or(&EMPTY)
}

/// Return the array stored under `map[key]`, or an empty slice when absent.
fn map_key_slice<'a>(map: &'a serde_json::Map<String, serde_json::Value>, key: &str) -> &'a [serde_json::Value] {
    static EMPTY: Vec<serde_json::Value> = Vec::new();
    map.get(key).and_then(|v| v.as_array()).map(|a| a.as_slice()).unwrap_or(&EMPTY)
}

fn as_arr<'a>(value: &'a serde_json::Value, what: &str) -> Result<&'a Vec<serde_json::Value>> {
    value
        .as_array()
        .ok_or_else(|| ExtInvError::Message(format!("{what}: expected an array")))
}

fn str_field<'a>(map: &'a serde_json::Map<String, serde_json::Value>, key: &str, what: &str) -> Result<&'a str> {
    map.get(key)
        .and_then(|v| v.as_str())
        .ok_or_else(|| ExtInvError::Message(format!("{what}: field {key:?} must be a string")))
}

pub fn validate(root: &Path, extracted: &serde_json::Value, manifest: &serde_json::Value) -> Result<()> {
    if manifest.get("oracle").and_then(|v| v.as_str()) != Some(ORACLE) {
        return fail("manifest oracle must be Pi v0.79.0 c5582102");
    }
    let categories = manifest
        .get("categories")
        .and_then(|v| v.as_object())
        .ok_or_else(|| ExtInvError::Message("manifest has no categories".into()))?;
    let extracted_obj = as_obj(extracted, "extracted")?;
    let extracted_keys: BTreeSet<&str> = extracted_obj.keys().map(|k| k.as_str()).collect();
    let cat_keys: BTreeSet<&str> = categories.keys().map(|k| k.as_str()).collect();
    if extracted_keys != cat_keys {
        return fail(format!(
            "manifest categories differ: expected {:?}, got {:?}",
            extracted_keys.iter().collect::<Vec<_>>(),
            cat_keys.iter().collect::<Vec<_>>()
        ));
    }
    let allowed = re(r"^(implemented|planned 9\.[1-8]|DESIGN exception [0-9]+)$")?;
    for (category, names_value) in extracted_obj {
        let names = as_arr(names_value, category)?;
        let names_set: BTreeSet<&str> = names.iter().filter_map(|v| v.as_str()).collect();
        let groups = categories
            .get(category)
            .and_then(|v| v.as_array())
            .ok_or_else(|| ExtInvError::Message(format!("{category}: expected status groups")))?;
        let mut rows: BTreeMap<&str, (&str, String)> = BTreeMap::new();
        for group in groups {
            let gmap = as_obj(group, "status group")?;
            let status = str_field(gmap, "status", "status group")?;
            let evidence = str_field(gmap, "evidence", "status group")?;
            if !allowed.is_match(status).map_err(|e| ExtInvError::Message(format!("regex: {e}")))? {
                return fail(format!("{category}: invalid status {status:?}"));
            }
            if evidence.trim().is_empty() {
                return fail(format!("{category}: evidence/target is required"));
            }
            let items = gmap
                .get("items")
                .and_then(|v| v.as_array())
                .ok_or_else(|| ExtInvError::Message(format!("{category}: expected items array")))?
                .iter()
                .filter_map(|i| i.as_str())
                .collect::<Vec<_>>();
            for item in items {
                if rows.contains_key(item) {
                    return fail(format!("{category}.{item}: classified more than once"));
                }
                rows.insert(item, (status, evidence.to_owned()));
            }
        }
        let row_set: BTreeSet<&str> = rows.keys().copied().collect();
        if row_set != names_set {
            let missing: Vec<&str> = names_set.difference(&row_set).copied().collect();
            let stale: Vec<&str> = row_set.difference(&names_set).copied().collect();
            return fail(format!("{category}: missing={missing:?}, stale={stale:?}"));
        }
    }

    let translations = manifest
        .get("translations")
        .and_then(|v| v.as_array())
        .ok_or_else(|| ExtInvError::Message("translations: expected a list".into()))?;
    let examples = extracted_obj
        .get("examples")
        .and_then(|v| v.as_array())
        .ok_or_else(|| ExtInvError::Message("no examples".into()))?;
    let example_set: BTreeSet<&str> = examples.iter().filter_map(|v| v.as_str()).collect();
    let mut translated: BTreeMap<String, &serde_json::Value> = BTreeMap::new();
    for row in translations {
        let rmap = as_obj(row, "translation")?;
        let source = str_field(rmap, "source", "translation")?;
        let lua_path = str_field(rmap, "lua", "translation")?;
        let evidence = str_field(rmap, "evidence", "translation")?;
        if translated.contains_key(source) {
            return fail(format!("translations.{source}: classified more than once"));
        }
        if !example_set.contains(source) {
            return fail(format!("translations.{source}: stale source"));
        }
        if !lua_path.starts_with("examples/extensions/") || !lua_path.ends_with(".lua") {
            return fail(format!("translations.{source}: invalid Lua path {lua_path:?}"));
        }
        let lua_full = root.join(lua_path);
        if !lua_full.is_file() {
            return fail(format!("translations.{source}: missing {lua_path}"));
        }
        if evidence.trim().is_empty() {
            return fail(format!("translations.{source}: CI evidence is required"));
        }
        translated.insert(source.to_owned(), row);
        let tests_source = std::fs::read_to_string(root.join("crates/pi-rs-app/tests/extension_loading.rs")).unwrap_or_default();
        let needle = format!("fn {evidence}(");
        if !tests_source.contains(&needle) {
            return fail(format!("translations.{source}: missing integration test {evidence}"));
        }
    }
    let implemented: BTreeSet<String> = map_key_slice(categories, "examples")
        .iter()
        .filter(|g| g.get("status").and_then(|v| v.as_str()) == Some("implemented"))
        .flat_map(|g| key_slice(g, "items").iter().filter_map(|i| i.as_str().map(str::to_owned)))
        .collect();
    let translated_set: BTreeSet<String> = translated.keys().cloned().collect();
    if translated_set != implemented {
        let missing: Vec<String> = implemented.difference(&translated_set).cloned().collect();
        let stale: Vec<String> = translated_set.difference(&implemented).cloned().collect();
        return fail(format!("translations: missing={missing:?}, non-implemented={stale:?}"));
    }
    Ok(())
}

fn esc(value: &str) -> String {
    value.replace('|', "\\|")
}

/// Render `EXTENSION_INVENTORY.md`.
pub fn render(extracted: &serde_json::Value, manifest: &serde_json::Value) -> Result<String> {
    let titles: BTreeMap<&str, &str> = [
        ("events", "Events"),
        ("api", "ExtensionAPI members"),
        ("ui", "Extension UI operations"),
        ("context", "ExtensionContext fields/actions"),
        ("command_context", "ExtensionCommandContext-only actions"),
        ("loader_rules", "Loader/resource rules"),
        ("examples", "Reference examples"),
    ]
    .into_iter()
    .collect();
    let extracted_obj = as_obj(extracted, "extracted")?;
    let categories = as_obj(manifest.get("categories").ok_or_else(|| ExtInvError::Message("no categories".into()))?, "categories")?;
    let mut lines: Vec<String> = vec![
        "# Extension parity inventory".to_owned(),
        String::new(),
        "Generated by `scripts/extension-inventory` from pinned Pi v0.79.0".to_owned(),
        "(`ref/pi` @ `c5582102`). Edit `tests/extension-inventory/manifest.json`,".to_owned(),
        "then regenerate; `--check` fails for any unclassified source member/example.".to_owned(),
        String::new(),
        "Statuses are closed: `implemented`, `planned 9.x`, or an explicit DESIGN exception.".to_owned(),
        String::new(),
    ];
    let mut counts: Vec<String> = Vec::new();
    for (category, names_value) in extracted_obj.iter() {
        let names = as_arr(names_value, category)?;
        counts.push(format!("{category}={}", names.len()));
        lines.push(format!("## {}", titles.get(category.as_str()).unwrap_or(&"?")));
        lines.push(String::new());
        lines.push("| Surface | Status | Evidence / target |".to_owned());
        lines.push("|---|---|---|".to_owned());
        let mut rows: BTreeMap<&str, (&str, String)> = BTreeMap::new();
        for group in map_key_slice(categories, category) {
            let gmap = as_obj(group, "status group")?;
            let status = str_field(gmap, "status", "status group")?;
            let evidence = str_field(gmap, "evidence", "status group")?;
            for item in map_key_slice(gmap, "items") {
                if let Some(name) = item.as_str() {
                    rows.insert(name, (status, evidence.to_owned()));
                }
            }
        }
        for name in names.iter().filter_map(|v| v.as_str()) {
            let (status, evidence) = rows.get(name).cloned().unwrap_or(("", String::new()));
            lines.push(format!(
                "| `{}` | {status} | {} |",
                esc(name),
                esc(&evidence)
            ));
        }
        lines.push(String::new());
    }
    lines.push(format!("Inventory counts: {}.", counts.join(", ")));
    lines.push(String::new());
    Ok(lines.join("\n"))
}

fn snake_case(name: &str) -> String {
    let mut out = String::new();
    let chars: Vec<char> = name.chars().collect();
    for (i, ch) in chars.iter().enumerate() {
        if ch.is_uppercase() {
            if i != 0 {
                out.push('_');
            }
            out.push(ch.to_ascii_lowercase());
        } else {
            out.push(*ch);
        }
    }
    out
}

fn rows_by_name<'a>(
    manifest: &'a serde_json::Value,
    category: &str,
) -> Result<BTreeMap<&'a str, (&'a str, &'a str)>> {
    let categories = as_obj(manifest.get("categories").ok_or_else(|| ExtInvError::Message("no categories".into()))?, "categories")?;
    let mut rows = BTreeMap::new();
    for group in map_key_slice(categories, category) {
        let gmap = as_obj(group, "status group")?;
        let status = str_field(gmap, "status", "status group")?;
        let evidence = str_field(gmap, "evidence", "status group")?;
        for item in map_key_slice(gmap, "items") {
            if let Some(name) = item.as_str() {
                rows.insert(name, (status, evidence));
            }
        }
    }
    Ok(rows)
}

/// Render `docs/lua-extension-api.md`.
pub fn render_api_docs(extracted: &serde_json::Value, manifest: &serde_json::Value) -> Result<String> {
    let extracted_obj = as_obj(extracted, "extracted")?;
    let mut rows_maps: BTreeMap<String, BTreeMap<&str, (&str, &str)>> = BTreeMap::new();
    for category in extracted_obj.keys() {
        rows_maps.insert(category.clone(), rows_by_name(manifest, category)?);
    }
    let mut translations: BTreeMap<&str, &serde_json::Value> = BTreeMap::new();
    for row in key_slice(manifest, "translations") {
        if let Some(src) = row.get("source").and_then(|v| v.as_str()) {
            translations.insert(src, row);
        }
    }
    let mut lines: Vec<String> = vec![
        "# Lua extension API".to_owned(),
        String::new(),
        "Generated by `scripts/extension-inventory` from the checked Pi v0.79.0 surface fixture.".to_owned(),
        "Do not edit this file directly.".to_owned(),
        String::new(),
        "An extension is a Lua file whose chunk receives `pi` as its first argument:".to_owned(),
        String::new(),
        "```lua".to_owned(),
        "local pi = ...".to_owned(),
        "pi.on(\"tool_call\", function(event, ctx) return nil end)".to_owned(),
        "```".to_owned(),
        String::new(),
        "Top-level Pi API names use Lua `snake_case`. Event names and Pi-compatible".to_owned(),
        "event/context/UI table fields retain their pinned spellings. `implemented` means".to_owned(),
        "the public path has executable evidence; `planned 9.x` remains unavailable or".to_owned(),
        "incomplete and must not be treated as a compatibility promise yet.".to_owned(),
        String::new(),
        "All callers share one public surface. Embedded packages receive no private API.".to_owned(),
        "Low-level additive mechanisms (`pi.fs`, `pi.tui`, `pi.ai`, etc.) are separate".to_owned(),
        "Lua-native capabilities and do not change this Pi-compatible vocabulary.".to_owned(),
        String::new(),
        "## ExtensionAPI".to_owned(),
        String::new(),
        "| Pi member | Lua | Status |".to_owned(),
        "|---|---|---|".to_owned(),
    ];
    let api = map_key_slice(extracted_obj, "api");
    for name in api.iter().filter_map(|v| v.as_str()) {
        let status = rows_maps.get("api").and_then(|m| m.get(name)).map(|(s, _)| *s).unwrap_or("");
        lines.push(format!("| `{name}` | `pi.{}` | {status} |", snake_case(name)));
    }
    lines.push(String::new());
    lines.push("## Events".to_owned());
    lines.push(String::new());
    lines.push("| Event | Status |".to_owned());
    lines.push("|---|---|".to_owned());
    for name in map_key_slice(extracted_obj, "events").iter().filter_map(|v| v.as_str()) {
        let status = rows_maps.get("events").and_then(|m| m.get(name)).map(|(s, _)| *s).unwrap_or("");
        lines.push(format!("| `{name}` | {status} |"));
    }
    for (category, title, prefix) in [
        ("context", "ExtensionContext", "ctx"),
        ("command_context", "ExtensionCommandContext-only", "ctx"),
        ("ui", "Extension UI", "ctx.ui"),
    ] {
        lines.push(String::new());
        lines.push(format!("## {title}"));
        lines.push(String::new());
        lines.push("| Pi-compatible field/action | Status |".to_owned());
        lines.push("|---|---|".to_owned());
        for name in map_key_slice(extracted_obj, category).iter().filter_map(|v| v.as_str()) {
            let status = rows_maps.get(category).and_then(|m| m.get(name)).map(|(s, _)| *s).unwrap_or("");
            lines.push(format!("| `{prefix}.{name}` | {status} |"));
        }
    }
    lines.push(String::new());
    lines.push("## Reference translation matrix".to_owned());
    lines.push(String::new());
    lines.push("Every pinned source is classified. A Lua path is published only after the".to_owned());
    lines.push("translation loads and its representative behavior executes in CI.".to_owned());
    lines.push(String::new());
    lines.push("| Pinned TypeScript | Lua translation | Status | Evidence / dependency |".to_owned());
    lines.push("|---|---|---|---|".to_owned());
    let examples = map_key_slice(extracted_obj, "examples");
    for name in examples.iter().filter_map(|v| v.as_str()) {
        let status = rows_maps.get("examples").and_then(|m| m.get(name)).map(|(s, _)| *s).unwrap_or("");
        let row = translations.get(name);
        let lua_path = row.and_then(|r| r.get("lua")).and_then(|v| v.as_str()).map(|l| format!("`{l}`")).unwrap_or_else(|| "—".to_owned());
        let evidence = row
            .and_then(|r| r.get("evidence").and_then(|v| v.as_str()))
            .map(|e| e.to_owned())
            .unwrap_or_else(|| rows_maps.get("examples").and_then(|m| m.get(name)).map(|(_, e)| e.to_string()).unwrap_or_default());
        lines.push(format!("| `{name}` | {lua_path} | {status} | {evidence} |"));
    }
    lines.push(String::new());
    lines.push(format!(
        "Matrix counts: translated={}, pinned={}.",
        translations.len(),
        examples.len()
    ));
    lines.push(String::new());
    Ok(lines.join("\n"))
}

/// Run the extension-inventory workflow against the repo at `root`.
pub fn run(root: &Path, check: bool, print_extracted: bool) -> Result<()> {
    let extracted = extract(root)?;
    if print_extracted {
        println!("{}", serde_json::to_string_pretty(&extracted)?);
        return Ok(());
    }
    let manifest: serde_json::Value = serde_json::from_slice(&std::fs::read(
        root.join("tests/extension-inventory/manifest.json"),
    )?)?;
    validate(root, &extracted, &manifest)?;
    let generated = render(&extracted, &manifest)?;
    let api_doc = render_api_docs(&extracted, &manifest)?;
    if check {
        let mut stale: Vec<String> = Vec::new();
        for (path, expected) in [
            (root.join("EXTENSION_INVENTORY.md"), &generated),
            (root.join("docs/lua-extension-api.md"), &api_doc),
        ] {
            let current = std::fs::read_to_string(&path).unwrap_or_default();
            if current != *expected {
                stale.push(path.display().to_string());
            }
        }
        if !stale.is_empty() {
            return fail(format!("stale generated files: {}", stale.join(", ")));
        }
        println!("extension inventory, translation matrix, and Lua API docs are complete and current");
    } else {
        std::fs::write(root.join("EXTENSION_INVENTORY.md"), &generated)?;
        let docs = root.join("docs");
        std::fs::create_dir_all(&docs)?;
        std::fs::write(docs.join("lua-extension-api.md"), &api_doc)?;
        println!("wrote EXTENSION_INVENTORY.md and docs/lua-extension-api.md");
    }
    Ok(())
}
