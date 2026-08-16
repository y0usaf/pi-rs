//! A.3 Rust owner for the pinned pi-flake external-extension capability
//! inventory.
//!
//! Faithful port of the former `scripts/external-extension-inventory`
//! (Python). Normal checks load and hash-validate the checked fixtures under
//! `tests/external-extension-inventory/fixtures/`, lex the TypeScript for
//! Pi-API use, Node ambient capabilities, lifetimes, and system needs,
//! validate the fail-closed manifest classification, and render/compare
//! `EXTERNAL_EXTENSION_INVENTORY.md`. `--refresh-fixtures` re-extracts the
//! pinned `pi-flake` tree through git (opt-in regeneration).

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use fancy_regex::Regex;
use sha2::{Digest, Sha256};

#[derive(Debug, thiserror::Error)]
pub enum ExtExtError {
    #[error("external extension inventory: {0}")]
    Message(String),
    #[error("external extension inventory: {0}")]
    Json(#[from] serde_json::Error),
    #[error("external extension inventory: {0}")]
    Io(#[from] std::io::Error),
}

type Result<T> = std::result::Result<T, ExtExtError>;

/// capability category -> capability item -> owning extensions
pub type RowMap = BTreeMap<String, BTreeMap<String, BTreeSet<String>>>;

pub const REVISION: &str = "94694da7321ce74aa7b82c13db7e60e28c0caba6";
const REPOSITORY: &str = "https://github.com/y0usaf/pi-flake";
const EXTENSIONS_TREE: &str = "c4a04dfe88314b5e48ebb200ccfd546645c3af9e";
const FIXTURE_DIGEST: &str = "7bf4166a0b4d3e2a7db7dd57faa381e04996849c464dcef0a34c3e93445856d2";
const SELECTION: &str = "package.json plus production TypeScript; tests/ and scripts/ excluded";

pub const EXPECTED_EXTENSIONS: &[&str] = &[
    "earendil_pi-review",
    "pi-codex-fast",
    "pi-compact",
    "pi-context-janitor",
    "pi-gecko-websearch",
    "pi-hashline",
    "pi-minimal-editor",
    "pi-morph",
    "pi-pomodoro",
    "pi-rlm",
    "pi-rtk",
    "pi-tool-management",
    "pi-webfetch",
    "pi-working-indicator",
    "sting8k_pi-vcc",
];

pub const CATEGORIES: &[&str] = &[
    "pi_api",
    "package_imports",
    "node_ambient",
    "lifetimes",
    "system_needs",
    "private_pi",
];

const CONTEXT_ROOTS: &[&str] = &[
    "abort", "compact", "cwd", "getContextUsage", "getSystemPrompt",
    "getSystemPromptOptions", "hasPendingMessages", "hasUI", "isIdle",
    "isProjectTrusted", "mode", "model", "modelRegistry", "navigateTree",
    "newSession", "reload", "sessionManager", "shutdown", "signal",
    "switchSession", "ui", "waitForIdle",
];

const PUBLIC_PI_CONCRETE: &[&str] = &["BorderedLoader", "CustomEditor", "DynamicBorder"];

fn fail<T>(msg: impl Into<String>) -> Result<T> {
    Err(ExtExtError::Message(msg.into()))
}

fn re(pattern: &str) -> Result<Regex> {
    Regex::new(pattern).map_err(|e| ExtExtError::Message(format!("invalid regex {pattern:?}: {e}")))
}

pub fn sha256_hex(data: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(data);
    hasher.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

/// `records_digest`: hash of the sorted `name/relative\0digest` lines.
pub fn records_digest(records: &BTreeMap<String, BTreeMap<String, String>>) -> String {
    let mut entries: Vec<String> = Vec::new();
    for (extension, files) in records {
        for (relative, digest) in files {
            entries.push(format!("{extension}/{relative}\0{digest}"));
        }
    }
    // (records already sorted when caller normalizes; join in order)
    sha256_hex(entries.join("\n").as_bytes())
}

fn is_private_pi_import(symbol: &str, type_only: bool) -> bool {
    if type_only || PUBLIC_PI_CONCRETE.contains(&symbol) {
        return false;
    }
    symbol.ends_with("Component") || symbol.ends_with("Manager")
        || matches!(symbol, "DefaultResourceLoader" | "createAgentSession")
}

/// Mask comments/literals in TS source while preserving line structure.
fn code_only(source: &str) -> Result<String> {
    let pat = re(r#""(?:\\.|[^"\\\n])*"|'(?:\\.|[^'\\\n])*'|//[^\n]*|/\*[\s\S]*?\*/"#)?;
    let mut out = String::new();
    let mut last = 0usize;
    for m in pat.find_iter(source) {
        let m = m.map_err(|e| ExtExtError::Message(format!("regex: {e}")))?;
        let start = m.start();
        let end = m.end();
        out.push_str(&source[last..start]);
        for ch in source[start..end].chars() {
            if ch=='\n'{
                out.push('\n');
            } else {
                out.push(' ');
            }
        }
        last = end;
    }
    out.push_str(&source[last..]);
    Ok(out)
}

/// `(imported, local, type_only)` rows parsed from an import clause.
fn parse_import_clause(clause: &str) -> Result<Vec<(String, String, bool)>> {
    let mut clause = clause.trim().to_owned();
    let mut whole_type = false;
    if clause.starts_with("type ") {
        whole_type = true;
        clause = clause[5..].trim().to_owned();
    }
    let mut result: Vec<(String, String, bool)> = Vec::new();
    // named {...}: Python removes the matched span and parses its members.
    let named_re = re(r"\{([\s\S]*?)\}")?;
    if let Some(caps) = named_re
        .captures(&clause)
        .map_err(|e| ExtExtError::Message(format!("regex: {e}")))?
    {
        let m = caps
            .get(0)
            .ok_or_else(|| ExtExtError::Message("named import match missing".into()))?;
        let inner = caps.get(1).map(|x| x.as_str()).unwrap_or("");
        for part in inner.split(',') {
            let part = part.trim();
            if part.is_empty() {
                continue;
            }
            let mut item_type = whole_type;
            let mut p = part.to_owned();
            if p.starts_with("type ") {
                item_type = true;
                p = p[5..].trim().to_owned();
            }
            let bits: Vec<&str> = if p.contains(" as ") {
                p.splitn(2, " as ").collect()
            } else {
                vec![p.as_str()]
            };
            let imported = bits[0].trim().to_owned();
            let local = bits.last().copied().unwrap_or_default().trim().to_owned();
            result.push((imported, local, item_type));
        }
        clause = format!("{}{}", &clause[..m.start()], &clause[m.end()..]);
    }
    // namespace `* as X`: Python removes only the matched span.
    let ns_re = re(r"\*\s+as\s+([A-Za-z_$][\w$]*)")?;
    if let Some(caps) = ns_re
        .captures(&clause)
        .map_err(|e| ExtExtError::Message(format!("regex: {e}")))?
    {
        let m = caps
            .get(0)
            .ok_or_else(|| ExtExtError::Message("namespace import match missing".into()))?;
        let name = caps.get(1).map(|x| x.as_str()).unwrap_or("").to_owned();
        result.push(("*".to_owned(), name, whole_type));
        clause = format!("{}{}", &clause[..m.start()], &clause[m.end()..]);
    }
    // default
    let default = clause.trim().trim_matches(',').trim().to_owned();
    if !default.is_empty() {
        let def_re = re(r"([A-Za-z_$][\w$]*)")?;
        if let Some(caps) = def_re
            .captures(&default)
            .map_err(|e| ExtExtError::Message(format!("regex: {e}")))?
            && let Some(m) = caps.get(1)
        {
            result.push(("default".to_owned(), m.as_str().to_owned(), whole_type));
        }
    }
    Ok(result)
}

/// lexically extract capability rows from the checked fixtures.
pub fn extract(sources: &BTreeMap<String, Vec<PathBuf>>) -> Result<BTreeMap<String, BTreeMap<String, BTreeSet<String>>>> {
    let mut rows: BTreeMap<String, BTreeMap<String, BTreeSet<String>>> = BTreeMap::new();
    for cat in CATEGORIES {
        rows.insert((*cat).to_owned(), BTreeMap::new());
    }
    let import_re = re(
        "(?:^|\\n)\\s*import\\s+(?!\\()(?:(?P<clause>[\\s\\S]*?)\\s+from\\s+)?[\"'](?P<module>[^\"']+)[\"']\\s*;?",
    )?;
    let dynamic_re = re(r#"\bimport\(\s*["']([^"']+)["']\s*\)"#)?;
    let pi_member_re = re(r"\bpi\.([A-Za-z_$][\w$]*)")?;
    let pi_on_re = re(r#"\bpi\.on\(\s*["']([^"']+)["']"#)?;
    let ctx_re = re(r"\bctx\.([A-Za-z_$][\w$]*)")?;
    let ctx_ui_re = re(r"\bctx\.ui\.([A-Za-z_$][\w$]*)")?;
    let ctx_sm_re = re(r"\bctx\.sessionManager\.([A-Za-z_$][\w$]*)")?;
    let ctx_mr_re = re(r"\bctx\.modelRegistry\.([A-Za-z_$][\w$]*)")?;
    let env_re = re(r"\bprocess\.env\.([A-Za-z_$][\w$]*)")?;
    let buf_re = re(r"\bBuffer\.([A-Za-z_$][\w$]*)")?;

    for (extension, paths) in sources {
        for path in paths {
            let text = std::fs::read_to_string(path).unwrap_or_default();
            let code = code_only(&text)?;
            let add = |rows: &mut BTreeMap<String, BTreeMap<String, BTreeSet<String>>>, category: &str, item: &str| {
                rows.entry(category.to_owned())
                    .or_default()
                    .entry(item.to_owned())
                    .or_default()
                    .insert(extension.clone());
            };
            let mut bindings: Vec<(String, String, String, bool)> = Vec::new();
            for caps in import_re.captures_iter(&text) {
                let caps = caps.map_err(|e| ExtExtError::Message(format!("regex: {e}")))?;
                let clause = caps.name("clause").map(|m| m.as_str().to_owned());
                let module = caps.name("module").map(|m| m.as_str().to_owned()).unwrap_or_default();
                if module.starts_with('.') || module.starts_with('/') {
                    continue;
                }
                let parsed = match &clause {
                    Some(c) => parse_import_clause(c)?,
                    None => vec![("side-effect".to_owned(), String::new(), false)],
                };
                for (imported, local, type_only) in parsed {
                    let marker = if type_only { format!("type:{imported}") } else { imported.clone() };
                    add(&mut rows, "package_imports", &format!("{module}#{marker}"));
                    bindings.push((module.clone(), imported.clone(), local.clone(), type_only));
                    if module == "@earendil-works/pi-coding-agent" && is_private_pi_import(&imported, type_only) {
                        add(&mut rows, "private_pi", &imported);
                    }
                }
            }
            for caps in dynamic_re.captures_iter(&text) {
                let caps = caps.map_err(|e| ExtExtError::Message(format!("regex: {e}")))?;
                if let Some(m) = caps.get(1) {
                    let module = m.as_str();
                    if !module.starts_with('.') && !module.starts_with('/') {
                        add(&mut rows, "package_imports", &format!("{module}#dynamic"));
                    }
                }
            }
            for caps in pi_member_re.captures_iter(&code) {
                let caps = caps.map_err(|e| ExtExtError::Message(format!("regex: {e}")))?;
                if let Some(m) = caps.get(1) {
                    add(&mut rows, "pi_api", &format!("ExtensionAPI.{}", m.as_str()));
                }
            }
            for caps in pi_on_re.captures_iter(&text) {
                let caps = caps.map_err(|e| ExtExtError::Message(format!("regex: {e}")))?;
                if let Some(m) = caps.get(1) {
                    add(&mut rows, "pi_api", &format!("event.{}", m.as_str()));
                }
            }
            for caps in ctx_re.captures_iter(&code) {
                let caps = caps.map_err(|e| ExtExtError::Message(format!("regex: {e}")))?;
                if let Some(m) = caps.get(1) {
                    let member = m.as_str();
                    if CONTEXT_ROOTS.contains(&member) {
                        add(&mut rows, "pi_api", &format!("ExtensionContext.{member}"));
                    }
                }
            }
            for caps in ctx_ui_re.captures_iter(&code) {
                let caps = caps.map_err(|e| ExtExtError::Message(format!("regex: {e}")))?;
                if let Some(m) = caps.get(1) {
                    add(&mut rows, "pi_api", &format!("ExtensionUI.{}", m.as_str()));
                }
            }
            for caps in ctx_sm_re.captures_iter(&code) {
                let caps = caps.map_err(|e| ExtExtError::Message(format!("regex: {e}")))?;
                if let Some(m) = caps.get(1) {
                    add(&mut rows, "pi_api", &format!("SessionManager.{}", m.as_str()));
                }
            }
            for caps in ctx_mr_re.captures_iter(&code) {
                let caps = caps.map_err(|e| ExtExtError::Message(format!("regex: {e}")))?;
                if let Some(m) = caps.get(1) {
                    add(&mut rows, "pi_api", &format!("ModelRegistry.{}", m.as_str()));
                }
            }
            let ambient_patterns: &[(&str, &str)] = &[
                ("process.cwd", r"\bprocess\.cwd\s*\("),
                ("process.platform", r"\bprocess\.platform\b"),
                ("process.argv", r"\bprocess\.argv\b"),
                ("fetch", r"\bfetch\s*\("),
                ("AbortController", r"\bAbortController\b"),
                ("AbortSignal", r"\bAbortSignal\b"),
                ("URL", r"\bnew\s+URL\s*\("),
                ("Bun.hash.xxHash32", r"\bBun\?\.hash\?\.xxHash32\b"),
            ];
            for (item, pattern) in ambient_patterns {
                if re(pattern)?.is_match(&code).map_err(|e| ExtExtError::Message(format!("regex: {e}")))? {
                    add(&mut rows, "node_ambient", item);
                }
            }
            for caps in env_re.captures_iter(&code) {
                let caps = caps.map_err(|e| ExtExtError::Message(format!("regex: {e}")))?;
                if let Some(m) = caps.get(1) {
                    add(&mut rows, "node_ambient", &format!("process.env.{}", m.as_str()));
                }
            }
            if re(r"\bprocess\.env\s*\[")?.is_match(&code).map_err(|e| ExtExtError::Message(format!("regex: {e}")))? {
                add(&mut rows, "node_ambient", "process.env[dynamic]");
            }
            for caps in buf_re.captures_iter(&code) {
                let caps = caps.map_err(|e| ExtExtError::Message(format!("regex: {e}")))?;
                if let Some(m) = caps.get(1) {
                    add(&mut rows, "node_ambient", &format!("Buffer.{}", m.as_str()));
                }
            }
            for timer in ["setTimeout", "clearTimeout", "setInterval", "clearInterval"] {
                if re(&format!(r"\b{timer}\s*\("))?.is_match(&code).map_err(|e| ExtExtError::Message(format!("regex: {e}")))? {
                    add(&mut rows, "node_ambient", timer);
                    add(&mut rows, "lifetimes", &format!("timer.{timer}"));
                }
            }
            let nm: BTreeMap<&str, &str> = [
                ("fs", "filesystem"),
                ("fs/promises", "filesystem"),
                ("child_process", "process"),
                ("net", "socket"),
                ("crypto", "crypto"),
            ]
            .into_iter()
            .collect();
            for (module, imported, local, type_only) in &bindings {
                let normalized = module.strip_prefix("node:").unwrap_or(module);
                let need = match nm.get(normalized) {
                    Some(n) => n,
                    None => continue,
                };
                if *type_only {
                    continue;
                }
                if imported == "*" || (normalized == "fs" && imported == "promises") {
                    let pat = format!(r"\b{}\.([A-Za-z_$][\w$]*)", fancy_regex::escape(local));
                    for caps in re(&pat)?.captures_iter(&code) {
                        let caps = caps.map_err(|e| ExtExtError::Message(format!("regex: {e}")))?;
                        if let Some(m) = caps.get(1) {
                            add(&mut rows, "system_needs", &format!("{need}.{}", m.as_str()));
                        }
                    }
                } else if imported != "side-effect" {
                    add(&mut rows, "system_needs", &format!("{need}.{imported}"));
                }
            }
            if re(r"\bpi\.exec\s*\(")?.is_match(&code).map_err(|e| ExtExtError::Message(format!("regex: {e}")))? {
                add(&mut rows, "system_needs", "process.pi.exec");
            }
            if re(r"\bfetch\s*\(")?.is_match(&code).map_err(|e| ExtExtError::Message(format!("regex: {e}")))? {
                add(&mut rows, "system_needs", "network.http");
            }
            if re(r"\bprocess\.env\b")?.is_match(&code).map_err(|e| ExtExtError::Message(format!("regex: {e}")))? {
                add(&mut rows, "system_needs", "process.environment");
            }
            let has_child_process = bindings.iter().any(|(module, _i, _l, type_only)| {
                let normalized = module.strip_prefix("node:").unwrap_or(module);
                normalized == "child_process" && !type_only
            });
            if has_child_process && re(r"\bstdio\s*:|\.(?:stdin|stdout|stderr)\b")?.is_match(&code).map_err(|e| ExtExtError::Message(format!("regex: {e}")))? {
                add(&mut rows, "system_needs", "process.stdio_pipes");
            }
            if has_child_process && re(r"\.kill\s*\(")?.is_match(&code).map_err(|e| ExtExtError::Message(format!("regex: {e}")))? {
                add(&mut rows, "system_needs", "process.kill");
            }
            if re(r#"\bcreateHash\s*\(\s*["']sha256["']"#)?.is_match(&text).map_err(|e| ExtExtError::Message(format!("regex: {e}")))? {
                add(&mut rows, "system_needs", "crypto.sha256");
            }
            if re(r"\bBun\?\.hash\?\.xxHash32\b")?.is_match(&code).map_err(|e| ExtExtError::Message(format!("regex: {e}")))? {
                add(&mut rows, "system_needs", "crypto.xxHash32");
            }
        }

        // lifetimes, computed per extension
        let add = |rows: &mut BTreeMap<String, BTreeMap<String, BTreeSet<String>>>, category: &str, item: &str| {
            rows.entry(category.to_owned())
                .or_default()
                .entry(item.to_owned())
                .or_default()
                .insert(extension.clone());
        };
        let lifetime_items = rows.get("lifetimes").cloned().unwrap_or_default();
        let has_timer_or_resource = lifetime_items.values().any(|owners| owners.contains(extension));
        let system = rows.get("system_needs").cloned().unwrap_or_default();
        let wants_child = system.iter().any(|(item, owners)| owners.contains(extension) && item.starts_with("process.spawn"));
        let wants_socket = system.iter().any(|(item, owners)| owners.contains(extension) && item.starts_with("socket."));
        let wants_watch = system.iter().any(|(item, owners)| owners.contains(extension) && item == "filesystem.watchFile");
        if wants_child {
            add(&mut rows, "lifetimes", "resource.child_process");
        }
        if wants_socket {
            add(&mut rows, "lifetimes", "resource.tcp_socket");
        }
        if wants_watch {
            add(&mut rows, "lifetimes", "resource.file_watcher");
        }
        // recompute has_resource from the *current* lifetimes (after the adds)
        let lifetimes_current = rows.get("lifetimes").cloned().unwrap_or_default();
        let has_resource = lifetimes_current.iter().any(|(item, owners)| {
            owners.contains(extension) && item.starts_with("resource.")
        });
        let events = rows.get("pi_api").cloned().unwrap_or_default();
        if (has_timer_or_resource || has_resource)
            && events.get("event.session_shutdown").map(|o| o.contains(extension)).unwrap_or(false)
        {
            add(&mut rows, "lifetimes", "cleanup.session_shutdown");
        }
    }
    Ok(rows)
}

/// Normalize extract() output for JSON printing (sorted owner lists).
pub fn rows_serializable(
    rows: &BTreeMap<String, BTreeMap<String, BTreeSet<String>>>,
) -> serde_json::Value {
    let mut obj = serde_json::Map::new();
    for category in CATEGORIES.iter().copied() {
        let values = rows.get(category);
        let mut catmap = serde_json::Map::new();
        if let Some(values) = values {
            for (item, owners) in values {
                catmap.insert(item.clone(), serde_json::json!(owners.iter().cloned().collect::<Vec<_>>()));
            }
        }
        obj.insert(category.to_string(), serde_json::Value::Object(catmap));
    }
    serde_json::Value::Object(obj)
}

/// Load and hash-validate the checked fixtures against `provenance.json`.
/// Returns `(provenance, sources)` where `sources` maps extension -> .ts paths.
pub fn load_and_validate_fixtures(base: &Path) -> Result<(serde_json::Value, BTreeMap<String, Vec<PathBuf>>)> {
    let provenance_path = base.join("provenance.json");
    let provenance: serde_json::Value = serde_json::from_slice(&std::fs::read(&provenance_path)?)?;
    let expected_header = [
        ("schema_version", serde_json::json!(1)),
        ("source_repository", serde_json::json!(REPOSITORY)),
        ("revision", serde_json::json!(REVISION)),
        ("extensions_tree", serde_json::json!(EXTENSIONS_TREE)),
        ("selection", serde_json::json!(SELECTION)),
        ("fixture_digest", serde_json::json!(FIXTURE_DIGEST)),
        ("extension_count", serde_json::json!(15)),
    ];
    for (key, expected) in expected_header {
        if provenance.get(key) != Some(&expected) {
            return fail(format!("provenance {key} must be {expected:?}"));
        }
    }
    let files = provenance
        .get("files")
        .and_then(|v| v.as_object())
        .ok_or_else(|| ExtExtError::Message("provenance has no files".into()))?;
    let expected: BTreeSet<&str> = EXPECTED_EXTENSIONS.iter().copied().collect();
    let actual_ext: BTreeSet<&str> = files.keys().map(|k| k.as_str()).collect();
    if actual_ext != expected {
        return fail("provenance must contain exactly the 15 pinned extensions");
    }
    // recompute records_digest from provenance
    let mut records: BTreeMap<String, BTreeMap<String, String>> = BTreeMap::new();
    for (extension, rec) in files {
        let mut m = BTreeMap::new();
        for (relative, digest) in rec.as_object().unwrap_or(&serde_json::Map::new()) {
            m.insert(relative.clone(), digest.as_str().unwrap_or("").to_owned());
        }
        records.insert(extension.clone(), m);
    }
    let expected_digest = records_digest(&records);
    if expected_digest != FIXTURE_DIGEST {
        return fail("provenance file inventory digest differs from pinned fixture");
    }

    let mut actual_paths: BTreeSet<String> = BTreeSet::new();
    let mut sources: BTreeMap<String, Vec<PathBuf>> = BTreeMap::new();
    for (extension, rec) in files {
        let rec_map = rec
            .as_object()
            .ok_or_else(|| ExtExtError::Message(format!("{extension}: invalid provenance record")))?;
        if !rec_map.contains_key("package.json") {
            return fail(format!("{extension}: package.json missing from provenance"));
        }
        let mut source_paths: Vec<PathBuf> = Vec::new();
        for (relative, expected_hash) in rec_map {
            let fixture_relative = format!("{extension}/{relative}");
            actual_paths.insert(fixture_relative.clone());
            let path = base.join("fixtures").join(extension).join(relative);
            if !path.is_file() {
                return fail(format!("fixture missing: {fixture_relative}"));
            }
            let bytes = std::fs::read(&path)?;
            let actual_hash = sha256_hex(&bytes);
            if actual_hash != expected_hash.as_str().unwrap_or("") {
                return fail(format!("fixture hash mismatch: {fixture_relative}"));
            }
            let is_ts = path.extension().and_then(|e| e.to_str()) == Some("ts");
            if is_ts {
                source_paths.push(path);
            }
        }
        if source_paths.is_empty() {
            return fail(format!("{extension}: no production TypeScript fixture"));
        }
        source_paths.sort();
        sources.insert(extension.clone(), source_paths);
    }
    // fixture file set exactly matches provenance paths
    let mut disk_paths: BTreeSet<String> = BTreeSet::new();
    walk_fixtures(&base.join("fixtures"), &base.join("fixtures"), &mut disk_paths)?;
    if disk_paths != actual_paths {
        let missing: Vec<String> = actual_paths.difference(&disk_paths).cloned().collect();
        let extra: Vec<String> = disk_paths.difference(&actual_paths).cloned().collect();
        return fail(format!("fixture file set differs: missing={missing:?}, extra={extra:?}"));
    }
    Ok((provenance, sources))
}

fn walk_fixtures(dir: &Path, base: &Path, out: &mut BTreeSet<String>) -> Result<()> {
    let mut entries: Vec<PathBuf> = std::fs::read_dir(dir)?.filter_map(|e| e.ok().map(|e| e.path())).collect();
    entries.sort();
    for entry in entries {
        let ft = entry.metadata()?;
        if ft.is_dir() {
            walk_fixtures(&entry, base, out)?;
        } else {
            let rel = entry.strip_prefix(base).map_err(|_| ExtExtError::Message("strip_prefix".into()))?;
            out.insert(rel.components().map(|c| c.as_os_str().to_string_lossy()).collect::<Vec<_>>().join("/"));
        }
    }
    Ok(())
}

fn as_obj<'a>(value: &'a serde_json::Value, what: &str) -> Result<&'a serde_json::Map<String, serde_json::Value>> {
    value
        .as_object()
        .ok_or_else(|| ExtExtError::Message(format!("{what}: expected an object")))
}

fn str_field<'a>(map: &'a serde_json::Map<String, serde_json::Value>, key: &str, what: &str) -> Result<&'a str> {
    map.get(key)
        .and_then(|v| v.as_str())
        .ok_or_else(|| ExtExtError::Message(format!("{what}: field {key:?} must be a string")))
}

fn key_slice<'a>(value: &'a serde_json::Value, key: &str) -> &'a [serde_json::Value] {
    static EMPTY: Vec<serde_json::Value> = Vec::new();
    value.get(key).and_then(|v| v.as_array()).map(|a| a.as_slice()).unwrap_or(&EMPTY)
}

/// Validate the manifest classification against the extracted rows.
pub fn validate_manifest(
    rows: &BTreeMap<String, BTreeMap<String, BTreeSet<String>>>,
    manifest: &serde_json::Value,
) -> Result<BTreeMap<String, BTreeMap<String, serde_json::Value>>> {
    let expected_oracle = format!("pi-flake {REVISION}");
    if manifest.get("oracle").and_then(|v| v.as_str()) != Some(expected_oracle.as_str()) {
        return fail(format!("manifest oracle must be pi-flake {REVISION}"));
    }
    let categories = manifest
        .get("categories")
        .and_then(|v| v.as_object())
        .ok_or_else(|| ExtExtError::Message("manifest has no categories".into()))?;
    let cat_keys: BTreeSet<&str> = categories.keys().map(|k| k.as_str()).collect();
    let expected_cats: BTreeSet<&str> = CATEGORIES.iter().copied().collect();
    if cat_keys != expected_cats {
        return fail(format!("manifest categories must be {CATEGORIES:?}"));
    }
    let allowed = re(r"^(implemented|planned 9\.(?:[2-9]|10|11)|DESIGN exception [0-9]+)$")?;
    let mut classified: BTreeMap<String, BTreeMap<String, serde_json::Value>> = BTreeMap::new();
    let categories_value = serde_json::Value::Object(categories.clone());
    for category in CATEGORIES.iter().copied() {
        let groups = key_slice(&categories_value, category);
        let mut found: BTreeMap<String, serde_json::Value> = BTreeMap::new();
        for group in groups {
            let gmap = as_obj(group, "status group")?;
            let status = str_field(gmap, "status", "status group")?;
            let evidence = str_field(gmap, "evidence", "status group")?;
            let items = key_slice(group, "items");
            if !allowed.is_match(status).map_err(|e| ExtExtError::Message(format!("regex: {e}")))? {
                return fail(format!("{category}: invalid status {status:?}"));
            }
            if evidence.trim().is_empty() {
                return fail(format!("{category}: evidence/target is required"));
            }
            if items.is_empty() {
                return fail(format!("{category}: each group needs non-empty items"));
            }
            for item in items {
                let item_name = item.as_str().ok_or_else(|| ExtExtError::Message(format!("{category}: invalid item")))?;
                if found.contains_key(item_name) {
                    return fail(format!("{category}.{item_name}: classified more than once"));
                }
                found.insert(item_name.to_owned(), serde_json::json!({"status": status, "evidence": evidence}));
            }
        }
        let expected: BTreeSet<String> = rows.get(category).map(|m| m.keys().cloned().collect()).unwrap_or_default();
        let actual: BTreeSet<String> = found.keys().cloned().collect();
        if actual != expected {
            let missing: Vec<String> = expected.difference(&actual).cloned().collect();
            let stale: Vec<String> = actual.difference(&expected).cloned().collect();
            return fail(format!("{category}: missing={missing:?}, stale={stale:?}"));
        }
        classified.insert(category.to_owned(), found);
    }
    Ok(classified)
}

fn esc(value: &str) -> String {
    value.replace('|', "\\|")
}

/// Render `EXTERNAL_EXTENSION_INVENTORY.md`.
pub fn render(
    provenance: &serde_json::Value,
    rows: &BTreeMap<String, BTreeMap<String, BTreeSet<String>>>,
    classified: &BTreeMap<String, BTreeMap<String, serde_json::Value>>,
) -> Result<String> {
    let titles: BTreeMap<&str, &str> = [
        ("pi_api", "Pi API use"),
        ("package_imports", "Package imports"),
        ("node_ambient", "Node ambient capabilities"),
        ("lifetimes", "Timers and background lifetimes"),
        ("system_needs", "Process / socket / filesystem / crypto needs"),
        ("private_pi", "Private Pi implementation dependencies"),
    ]
    .into_iter()
    .collect();
    let extensions_tree = provenance.get("extensions_tree").and_then(|v| v.as_str()).unwrap_or("");
    let mut lines: Vec<String> = vec![
        "# External extension capability inventory".to_owned(),
        String::new(),
        format!("Generated from checked fixtures for all 15 extensions at `pi-flake` `{REVISION}`"),
        format!("(`extensions` tree `{extensions_tree}`). The source fixture hashes live in"),
        "`tests/external-extension-inventory/provenance.json`; normal generation/checking is offline.".to_owned(),
        String::new(),
        "Statuses are closed: `implemented`, a specific `planned 9.x` rung, or an explicit DESIGN exception.".to_owned(),
        String::new(),
    ];
    let mut all_counts: Vec<String> = Vec::new();
    let mut file_counts: BTreeMap<String, usize> = BTreeMap::new();
    let files = provenance.get("files").and_then(|v| v.as_object());
    for (extension, rec) in files.unwrap_or(&serde_json::Map::new()) {
        file_counts.insert(extension.clone(), rec.as_object().map(|m| m.len()).unwrap_or(0));
    }
    for category in CATEGORIES.iter().copied() {
        lines.extend(vec![
            format!("## {}", titles.get(category).unwrap_or(&"????")),
            String::new(),
            "| Capability | Extensions | Status | Evidence / target |".to_owned(),
            "|---|---|---|---|".to_owned(),
        ]);
        let values = &rows[category];
        for item in values.keys() {
            let owners: Vec<String> = values[item].iter().map(|n| format!("`{n}`")).collect();
            let record = classified.get(category).and_then(|m| m.get(item)).cloned().unwrap_or_else(|| serde_json::json!({"status": "", "evidence": ""}));
            let status = record.get("status").and_then(|v| v.as_str()).unwrap_or("");
            let evidence = record.get("evidence").and_then(|v| v.as_str()).unwrap_or("");
            lines.push(format!(
                "| `{}` | {} | {status} | {} |",
                esc(item),
                owners.join(", "),
                esc(evidence)
            ));
        }
        all_counts.push(format!("{category}={}", values.len()));
        lines.push(String::new());
    }
    lines.extend([
        "## Per-extension coverage".to_owned(),
        String::new(),
        "| Extension | Source files | Capability rows |".to_owned(),
        "|---|---:|---:|".to_owned(),
    ]);
    for extension in EXPECTED_EXTENSIONS {
        let count = rows
            .values()
            .map(|m| m.values().filter(|o| o.contains(*extension)).count())
            .sum::<usize>();
        let sf = file_counts.get(*extension).map(|c| c.saturating_sub(1)).unwrap_or(0);
        lines.push(format!("| `{extension}` | {sf} | {count} |"));
    }
    lines.push(String::new());
    let total_files: usize = file_counts.values().map(|c| c.saturating_sub(1)).sum();
    lines.push(format!(
        "Inventory counts: extensions=15, source_files={total_files}, {}.",
        all_counts.join(", ")
    ));
    lines.push(String::new());
    Ok(lines.join("\n"))
}

/// Run the external-extension-inventory workflow against the repo at `root`.
pub fn run(root: &Path, base: &Path, check: bool, print_extracted: bool) -> Result<()> {
    let (provenance, sources) = load_and_validate_fixtures(base)?;
    let rows = extract(&sources)?;
    if print_extracted {
        println!("{}", serde_json::to_string_pretty(&rows_serializable(&rows))?);
        return Ok(());
    }
    let manifest: serde_json::Value = serde_json::from_slice(&std::fs::read(base.join("manifest.json"))?)?;
    let classified = validate_manifest(&rows, &manifest)?;
    let generated = render(&provenance, &rows, &classified)?;
    let output = root.join("EXTERNAL_EXTENSION_INVENTORY.md");
    if check {
        let current = std::fs::read_to_string(&output).unwrap_or_default();
        if current != generated {
            eprintln!("{} is stale; regenerate", output.display());
            return fail("EXTERNAL_EXTENSION_INVENTORY.md is stale");
        }
        println!("external extension inventory is complete and current");
    } else {
        std::fs::write(&output, &generated)?;
        println!("wrote EXTERNAL_EXTENSION_INVENTORY.md");
    }
    Ok(())
}
