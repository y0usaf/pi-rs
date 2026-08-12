//! A.3 Rust owner for the first-party construction inventory.
//!
//! Faithful port of the former `scripts/construction-inventory` (Python). It
//! generates/checks `CONSTRUCTION_INVENTORY.md` from the checked provenance
//! and manifest under `tests/construction-inventory/`, failing closed for
//! missing/stale/duplicate embedded sources, unclassified declarations, Rust
//! launch/composition seams, hardcoded product entrypoints, and named open
//! rows. `nix flake check` runs the Rust binary, so the workflow needs no
//! repo-owned Python.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use fancy_regex::Regex;

#[derive(Debug, thiserror::Error)]
pub enum InventoryError {
    #[error("construction inventory: {0}")]
    Message(String),
    #[error("construction inventory: {0}")]
    Json(#[from] serde_json::Error),
    #[error("construction inventory: {0}")]
    Io(#[from] std::io::Error),
}

type Result<T> = std::result::Result<T, InventoryError>;

fn fail<T>(msg: impl Into<String>) -> Result<T> {
    Err(InventoryError::Message(msg.into()))
}

fn re(pattern: &str) -> Result<Regex> {
    Regex::new(pattern).map_err(|e| InventoryError::Message(format!("invalid regex {pattern:?}: {e}")))
}

fn load_json(path: &Path) -> Result<serde_json::Value> {
    let value: serde_json::Value = serde_json::from_slice(&std::fs::read(path)?)?;
    if !value.is_object() {
        return fail(format!("{}: expected an object", path.display()));
    }
    Ok(value)
}

fn as_obj<'a>(value: &'a serde_json::Value, what: &str) -> Result<&'a serde_json::Map<String, serde_json::Value>> {
    value
        .as_object()
        .ok_or_else(|| InventoryError::Message(format!("{what}: expected an object")))
}

fn as_arr<'a>(value: &'a serde_json::Value, what: &str) -> Result<&'a Vec<serde_json::Value>> {
    value
        .as_array()
        .ok_or_else(|| InventoryError::Message(format!("{what}: expected an array")))
}

fn str_field<'a>(map: &'a serde_json::Map<String, serde_json::Value>, key: &str, what: &str) -> Result<&'a str> {
    map.get(key)
        .and_then(|v| v.as_str())
        .ok_or_else(|| InventoryError::Message(format!("{what}: field {key:?} must be a string")))
}

fn int_field(map: &serde_json::Map<String, serde_json::Value>, key: &str, what: &str) -> Result<i64> {
    map.get(key)
        .and_then(|v| v.as_i64())
        .ok_or_else(|| InventoryError::Message(format!("{what}: field {key:?} must be an integer")))
}

fn str_val<'a>(v: &'a serde_json::Value, what: &str) -> Result<&'a str> {
    v.as_str()
        .ok_or_else(|| InventoryError::Message(format!("{what}: expected a string")))
}

/// Collect every `Vec<&serde_json::Value>` stored under `key`; empty when absent.
fn vec_field<'a>(value: &'a serde_json::Value, key: &str) -> &'a [serde_json::Value] {
    value.get(key).and_then(|v| v.as_array()).map(|a| a.as_slice()).unwrap_or(&[])
}

/// Extract the brace-delimited body of an embedded-pack constant
/// (`pub const NAME: ... = { ... }`) as in the Python `const_block`.
fn const_block(source: &str, name: &str) -> Result<String> {
    let pat = re(&format!(r"\bpub\s+const\s+{}\b[^=]*=", fancy_regex::escape(name)))?;
    let caps = pat
        .captures(source)
        .map_err(|e| InventoryError::Message(format!("regex: {e}")))?
        .ok_or_else(|| InventoryError::Message(format!("missing embedded-pack constant {name}")))?;
    let m = caps
        .get(0)
        .ok_or_else(|| InventoryError::Message("internal captures".into()))?;
    let after = &source[m.end()..];
    let start_offset = after
        .find('{')
        .ok_or_else(|| InventoryError::Message(format!("missing body for embedded-pack constant {name}")))?;
    let start = m.end() + start_offset;
    let mut depth = 0usize;
    for (i, ch) in source[start..].char_indices() {
        match ch {
            '{' => depth += 1,
            '}' => {
                depth = depth
                    .checked_sub(1)
                    .ok_or_else(|| InventoryError::Message(format!("unterminated embedded-pack constant {name}")))?;
                if depth == 0 {
                    return Ok(source[start + 1..start + i].to_owned());
                }
            }
            _ => {}
        }
    }
    fail(format!("unterminated embedded-pack constant {name}"))
}

type Memberships = BTreeMap<String, BTreeSet<String>>;

fn extract_embedded(root: &Path, provenance: &serde_json::Value) -> Result<(Memberships, BTreeMap<String, String>)> {
    let packages = as_arr(provenance.get("embedded_packages").ok_or_else(|| InventoryError::Message("provenance has no embedded_packages".into()))?, "embedded_packages")?;
    let includes_re = re(r#"include_(?:str|bytes)!\("([^"]+)"\)"#)?;
    let name_re = re(r#"\bname\s*:\s*"([^"]+)""#)?;
    let root_canon = root.canonicalize()?;
    let mut memberships: Memberships = BTreeMap::new();
    let mut package_names: BTreeMap<String, String> = BTreeMap::new();
    for package in packages {
        let map = as_obj(package, "embedded package")?;
        let package_id = str_field(map, "id", "embedded package")?.to_owned();
        let descriptor = root.join(str_field(map, "descriptor", "embedded package")?);
        let constant = str_field(map, "constant", "embedded package")?;
        let source_name = str_field(map, "source_name", "embedded package")?;
        let source = std::fs::read_to_string(&descriptor)?;
        let block = const_block(&source, constant)?;
        let name_caps = name_re
            .captures(&block)
            .map_err(|e| InventoryError::Message(format!("regex: {e}")))?
            .ok_or_else(|| InventoryError::Message(format!("{package_id}: embedded pack has no literal name")))?;
        let actual_name = name_caps
            .get(1)
            .map(|m| m.as_str())
            .ok_or_else(|| InventoryError::Message(format!("{package_id}: embedded pack has no literal name")))?;
        if actual_name != source_name {
            return fail(format!(
                "{package_id}: source name changed: expected {source_name:?}, got {actual_name:?}"
            ));
        }
        if package_names.contains_key(&package_id) {
            return fail(format!("duplicate embedded package id {package_id}"));
        }
        package_names.insert(package_id.clone(), actual_name.to_owned());
        let parent = descriptor
            .parent()
            .ok_or_else(|| InventoryError::Message(format!("{package_id}: no parent dir")))?;
        let mut found_any = false;
        for m in includes_re.captures_iter(&block) {
            let m = m.map_err(|e| InventoryError::Message(format!("regex: {e}")))?;
            let include = m
                .get(1)
                .map(|x| x.as_str())
                .ok_or_else(|| InventoryError::Message(format!("{package_id}: include path missing")))?;
            found_any = true;
            let resolved = parent.join(include).canonicalize()?;
            let Ok(relative) = resolved.strip_prefix(&root_canon) else {
                return fail(format!("{package_id}: source escapes repository: {include}"));
            };
            let relative_posix = path_posix(relative);
            if !resolved.is_file() {
                return fail(format!("{package_id}: missing embedded source {relative_posix}"));
            }
            memberships.entry(relative_posix).or_default().insert(package_id.clone());
        }
        if !found_any {
            return fail(format!("{package_id}: embedded pack has no source units"));
        }
    }
    if package_names.is_empty() {
        return fail("provenance has no embedded packages");
    }
    Ok((memberships, package_names))
}

fn path_posix(p: &Path) -> String {
    p.components()
        .collect::<PathBuf>()
        .to_string_lossy()
        .replace('\\', "/")
}

const DECLARATION_RE: &str = r#"pi\.register_(command|tool|role)\s*\(\s*(?:["']([^"']+)["']|\{)"#;

fn extract_declarations(root: &Path, embedded: &Memberships) -> Result<BTreeSet<String>> {
    let decl_re = re(DECLARATION_RE)?;
    let mut declarations: BTreeSet<String> = BTreeSet::new();
    for relative in embedded.keys() {
        if !relative.ends_with(".lua") {
            continue;
        }
        let source = std::fs::read_to_string(root.join(relative))?;
        for m in decl_re.captures_iter(&source) {
            let m = m.map_err(|e| InventoryError::Message(format!("regex: {e}")))?;
            let kind = m
                .get(1)
                .map(|x| x.as_str())
                .ok_or_else(|| InventoryError::Message(format!("{relative}: no declaration kind")))?;
            let literal = m.get(2).map(|x| x.as_str());
            let name: String = if kind == "command" {
                literal
                    .map(|s| s.to_owned())
                    .ok_or_else(|| InventoryError::Message(format!("{relative}: register_command must use a literal name for inventory")))?
            } else {
                let field = if kind == "role" { "role" } else { "name" };
                let end = m.get(0).map(|x| x.end()).unwrap_or(0);
                let block = &source[end..];
                let field_re = re(&format!(r#"\b{}\s*=\s*["']([^"']+)["']"#, fancy_regex::escape(field)))?;
                let caps = field_re
                    .captures(block)
                    .map_err(|e| InventoryError::Message(format!("regex: {e}")))?
                    .ok_or_else(|| {
                        InventoryError::Message(format!("{relative}: register_{kind} must have a literal {field} for inventory"))
                    })?;
                caps.get(1)
                    .map(|x| x.as_str().to_owned())
                    .ok_or_else(|| {
                        InventoryError::Message(format!("{relative}: register_{kind} must have a literal {field} for inventory"))
                    })?
            };
            let key = format!("{kind}:{name}");
            if declarations.contains(&key) {
                return fail(format!("duplicate embedded declaration {key}"));
            }
            declarations.insert(key);
        }
    }
    Ok(declarations)
}

fn collect_rs(dir: &Path, out: &mut Vec<PathBuf>) -> Result<()> {
    if dir.is_file() {
        if dir.extension().and_then(|e| e.to_str()) == Some("rs") {
            out.push(dir.to_path_buf());
        }
        return Ok(());
    }
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let p = entry.path();
        if p.is_dir() {
            collect_rs(&p, out)?;
        } else if p.extension().and_then(|e| e.to_str()) == Some("rs") {
            out.push(p);
        }
    }
    Ok(())
}

fn extract_rust_entrypoints(root: &Path, provenance: &serde_json::Value) -> Result<BTreeMap<String, usize>> {
    let provenance_obj = as_obj(provenance, "provenance")?;
    let pattern_str = str_field(provenance_obj, "rust_entrypoint_pattern", "provenance")?;
    let pattern = re(pattern_str)?;
    let roots = as_arr(provenance.get("rust_source_roots").ok_or_else(|| InventoryError::Message("provenance has no rust_source_roots".into()))?, "rust_source_roots")?;
    let mut found: BTreeMap<String, usize> = BTreeMap::new();
    for root_rel in roots {
        let base = root.join(str_val(root_rel, "rust_source_root")?);
        let mut paths: Vec<PathBuf> = Vec::new();
        collect_rs(&base, &mut paths)?;
        paths.sort();
        for path in paths {
            let source = std::fs::read_to_string(&path)?;
            for m in pattern.captures_iter(&source) {
                let m = m.map_err(|e| InventoryError::Message(format!("regex: {e}")))?;
                if let Some(g) = m.get(1) {
                    *found.entry(g.as_str().to_owned()).or_insert(0) += 1;
                }
            }
        }
    }
    Ok(found)
}

fn validate_anchors(root: &Path, rows: &[serde_json::Value]) -> Result<()> {
    for row in rows {
        let map = as_obj(row, "row")?;
        let row_id = str_field(map, "id", "row")?;
        for anchor in vec_field(row, "anchors") {
            let amap = as_obj(anchor, "anchor")?;
            let mut keys: BTreeSet<String> = amap.keys().cloned().collect();
            keys.remove("count");
            keys.remove("path");
            keys.remove("text");
            if !keys.is_empty() {
                return fail(format!("{row_id}: anchor fields must be path/text/count"));
            }
            if !(amap.contains_key("path") && amap.contains_key("text") && amap.contains_key("count")) {
                return fail(format!("{row_id}: anchor fields must be path/text/count"));
            }
            let path = str_field(amap, "path", "anchor")?;
            let text = str_field(amap, "text", "anchor")?;
            let count = int_field(amap, "count", "anchor")?;
            let source = std::fs::read_to_string(root.join(path))?;
            let actual = source.matches(text).count() as i64;
            if actual != count {
                return fail(format!(
                    "{row_id}: stale anchor {path}:{text:?}; expected {count}, got {actual}"
                ));
            }
        }
    }
    Ok(())
}

fn validate_scans(root: &Path, provenance: &serde_json::Value, row_ids: &BTreeSet<String>) -> Result<()> {
    let mut seen: BTreeSet<String> = BTreeSet::new();
    for scan in vec_field(provenance, "rust_seams") {
        let smap = as_obj(scan, "rust seam")?;
        let scan_id = str_field(smap, "id", "rust seam")?;
        if !seen.insert(scan_id.to_owned()) {
            return fail(format!("duplicate Rust seam provenance {scan_id}"));
        }
        let row = str_field(smap, "row", "rust seam")?;
        if !row_ids.contains(row) {
            return fail(format!("Rust seam {scan_id}: missing row {row}"));
        }
        let path = str_field(smap, "path", "rust seam")?;
        let text = str_field(smap, "text", "rust seam")?;
        let count = int_field(smap, "count", "rust seam")?;
        let source = std::fs::read_to_string(root.join(path))?;
        let actual = source.matches(text).count() as i64;
        if actual != count {
            return fail(format!(
                "Rust seam {scan_id} is stale: expected {count} occurrences of {text:?}, got {actual}"
            ));
        }
    }

    let mut expected_calls: BTreeMap<(String, String), i64> = BTreeMap::new();
    for call in vec_field(provenance, "rust_call_inventory") {
        let cmap = as_obj(call, "rust call")?;
        let row = str_field(cmap, "row", "rust call")?;
        let path = str_field(cmap, "path", "rust call")?;
        let operation = str_field(cmap, "operation", "rust call")?;
        if !row_ids.contains(row) {
            return fail(format!("Rust call {path}:{operation}: missing row {row}"));
        }
        let key = (path.to_owned(), operation.to_owned());
        if expected_calls.contains_key(&key) {
            return fail(format!("duplicate Rust call inventory {path}:{operation}"));
        }
        expected_calls.insert(key, int_field(cmap, "count", "rust call")?);
    }
    let operations: Vec<String> = vec_field(provenance, "rust_call_operations")
        .iter()
        .map(|v| str_val(v, "rust_call_operation").map(|s| s.to_owned()))
        .collect::<Result<Vec<_>>>()?;
    let roots: Vec<String> = vec_field(provenance, "rust_source_roots")
        .iter()
        .map(|v| str_val(v, "rust_source_root").map(|s| s.to_owned()))
        .collect::<Result<Vec<_>>>()?;
    let mut actual_calls: BTreeMap<(String, String), usize> = BTreeMap::new();
    for source_root in roots {
        let mut paths: Vec<PathBuf> = Vec::new();
        collect_rs(&root.join(&source_root), &mut paths)?;
        paths.sort();
        for path in paths {
            let source = std::fs::read_to_string(&path)?;
            let Ok(relative) = path.strip_prefix(root) else { continue };
            let relative_posix = path_posix(relative);
            for operation in &operations {
                let needle = format!(".{operation}(");
                let count = source.matches(&needle).count();
                if count > 0 {
                    *actual_calls.entry((relative_posix.clone(), operation.clone())).or_insert(0) += count;
                }
            }
        }
    }
    let actual_normalized: BTreeMap<(String, String), i64> =
        actual_calls.into_iter().map(|(k, v)| (k, v as i64)).collect();
    if actual_normalized != expected_calls {
        let fmt =
            |map: &BTreeMap<(String, String), i64>| -> Vec<String> {
                map.iter().map(|((p, o), c)| format!("{p}:{o} ({c})")).collect()
            };
        return fail(format!(
            "Rust launch/composition calls differ: extracted={}, inventoried={}",
            fmt(&actual_normalized).join(", "),
            fmt(&expected_calls).join(", ")
        ));
    }
    Ok(())
}

const REQUIRED_FIELDS: &[&str] = &[
    "anchors",
    "coverage",
    "declarations",
    "disable_path",
    "id",
    "kind",
    "packages",
    "public_declaration",
    "replacement_evidence",
    "rust_entrypoints",
    "rung",
    "status",
    "title",
];

fn validate(
    root: &Path,
    provenance: &serde_json::Value,
    manifest: &serde_json::Value,
) -> Result<(Memberships, BTreeSet<String>, BTreeMap<String, usize>)> {
    if provenance.get("schema").and_then(|v| v.as_i64()) != Some(1) || manifest.get("schema").and_then(|v| v.as_i64()) != Some(1) {
        return fail("provenance/manifest schema must be 1");
    }
    if provenance.get("oracle").and_then(|v| v.as_str()) != Some("Pi v0.79.0 c5582102") {
        return fail("provenance oracle must be Pi v0.79.0 c5582102");
    }
    let rows = as_arr(manifest.get("rows").ok_or_else(|| InventoryError::Message("manifest has no rows".into()))?, "manifest rows")?;
    if rows.is_empty() {
        return fail("manifest rows must be a non-empty array");
    }
    let id_re = re(r"[a-z0-9][a-z0-9._-]*")?;
    let mut ids: BTreeSet<String> = BTreeSet::new();
    for row in rows {
        let map = as_obj(row, "row")?;
        let keyset: BTreeSet<&str> = map.keys().map(|k| k.as_str()).collect();
        let required: BTreeSet<&str> = REQUIRED_FIELDS.iter().copied().collect();
        if keyset != required {
            let missing: Vec<&str> = required.difference(&keyset).copied().collect();
            let extra: Vec<&str> = keyset.difference(&required).copied().collect();
            let id = map.get("id").and_then(|v| v.as_str()).ok_or_else(|| InventoryError::Message("row has no id".into()))?;
            return fail(format!("{id}: row fields differ; missing={missing:?}, extra={extra:?}"));
        }
        let row_id = str_field(map, "id", "row")?;
        if !id_re.is_match(row_id).map_err(|e| InventoryError::Message(format!("regex: {e}")))? {
            return fail(format!("invalid row id {row_id:?}"));
        }
        if !ids.insert(row_id.to_owned()) {
            return fail(format!("duplicate row id {row_id}"));
        }
        for field in ["kind", "title", "public_declaration", "disable_path", "replacement_evidence", "rung"] {
            let ok = map.get(field).and_then(|x| x.as_str()).map(|s| !s.trim().is_empty()).unwrap_or(false);
            if !ok {
                return fail(format!("{row_id}: {field} is required"));
            }
        }
        let status = str_field(map, "status", "row")?;
        if !["implemented", "open", "DESIGN exception"].contains(&status) {
            return fail(format!("{row_id}: invalid status {status:?}"));
        }
        let packages = vec_field(row, "packages");
        if packages.is_empty() {
            return fail(format!("{row_id}: packages must be non-empty"));
        }
        for field in ["coverage", "anchors", "declarations", "rust_entrypoints"] {
            if map.get(field).and_then(|v| v.as_array()).is_none() {
                return fail(format!("{row_id}: {field} must be an array"));
            }
        }
    }

    let required_open: BTreeSet<String> = vec_field(provenance, "required_open_rows")
        .iter()
        .map(|v| str_val(v, "required_open_row").map(|s| s.to_owned()))
        .collect::<Result<Vec<_>>>()?
        .into_iter()
        .collect();
    let missing_open: Vec<&String> = required_open.iter().filter(|r| !ids.contains(*r)).collect();
    if !missing_open.is_empty() {
        let names: Vec<&str> = missing_open.iter().map(|s| s.as_str()).collect();
        return fail(format!("missing named open rows: {names:?}"));
    }
    for row in rows {
        let map = as_obj(row, "row")?;
        let row_id = str_field(map, "id", "row")?;
        if required_open.contains(row_id) && str_field(map, "status", "row")? != "open" {
            return fail(format!("{row_id}: named row must remain open until its owning rung closes it"));
        }
    }

    let (embedded, package_names) = extract_embedded(root, provenance)?;
    let mut known_packages: BTreeSet<String> = package_names.keys().cloned().collect();
    known_packages.extend(vec_field(provenance, "non_embedded_packages").iter().map(|v| str_val(v, "non_embedded_package").map(|s| s.to_owned())).collect::<Result<Vec<_>>>()?);

    let mut coverage: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for row in rows {
        let map = as_obj(row, "row")?;
        let row_id = str_field(map, "id", "row")?;
        let unknown: Vec<String> = vec_field(row, "packages")
            .iter()
            .map(|v| str_val(v, "package").map(|s| s.to_owned()))
            .collect::<Result<Vec<_>>>()?
            .into_iter()
            .filter(|p| !known_packages.contains(p))
            .collect();
        if !unknown.is_empty() {
            return fail(format!("{row_id}: unknown packages {unknown:?}"));
        }
        for source in vec_field(row, "coverage") {
            let source = str_val(source, "coverage source")?.to_owned();
            coverage.entry(source).or_default().push(row_id.to_owned());
        }
    }
    let embedded_set: BTreeSet<&String> = embedded.keys().collect();
    let coverage_set: BTreeSet<&String> = coverage.keys().collect();
    let missing_sources: Vec<&String> = embedded_set.difference(&coverage_set).copied().collect();
    let stale_sources: Vec<&String> = coverage_set.difference(&embedded_set).copied().collect();
    let duplicate_sources: Vec<(String, usize)> = coverage
        .iter()
        .filter(|(_, v)| v.len() != 1)
        .map(|(k, v)| (k.clone(), v.len()))
        .collect();
    if !missing_sources.is_empty() || !stale_sources.is_empty() || !duplicate_sources.is_empty() {
        return fail(format!(
            "embedded source coverage differs: missing={missing_sources:?}, stale={stale_sources:?}, duplicates={duplicate_sources:?}"
        ));
    }
    for (source, packages) in &embedded {
        let owners = coverage.get(source).ok_or_else(|| InventoryError::Message("internal coverage".into()))?;
        let row_id = &owners[0];
        let row = rows
            .iter()
            .find(|r| as_obj(r, "row").ok().and_then(|m| m.get("id")).and_then(|v| v.as_str()) == Some(row_id.as_str()))
            .ok_or_else(|| InventoryError::Message("internal row lookup".into()))?;
        let row_packages: BTreeSet<&str> = vec_field(row, "packages").iter().filter_map(|x| x.as_str()).collect();
        let expected: BTreeSet<&str> = packages.iter().map(|s| s.as_str()).collect();
        if row_packages != expected {
            return fail(format!(
                "{row_id}: package membership for {source} differs: expected {expected:?}, got {row_packages:?}"
            ));
        }
    }

    let extracted_declarations = extract_declarations(root, &embedded)?;
    let mut declared: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for row in rows {
        let map = as_obj(row, "row")?;
        let row_id = str_field(map, "id", "row")?.to_owned();
        for declaration in vec_field(row, "declarations") {
            declared.entry(str_val(declaration, "declaration")?.to_owned()).or_default().push(row_id.clone());
        }
    }
    let declared_set: BTreeSet<&String> = declared.keys().collect();
    let missing_declarations: Vec<&String> = extracted_declarations.iter().filter(|k| !declared_set.contains(k)).collect();
    let stale_declarations: Vec<&String> = declared_set.iter().copied().filter(|k| !extracted_declarations.contains(*k)).collect();
    let duplicate_declarations: Vec<String> = declared.iter().filter(|(_, v)| v.len() != 1).map(|(k, _)| k.clone()).collect();
    if !missing_declarations.is_empty() || !stale_declarations.is_empty() || !duplicate_declarations.is_empty() {
        return fail(format!(
            "embedded declarations differ: missing={missing_declarations:?}, stale={stale_declarations:?}, duplicates={duplicate_declarations:?}"
        ));
    }

    let extracted_entrypoints = extract_rust_entrypoints(root, provenance)?;
    let mut inventoried: BTreeMap<String, usize> = BTreeMap::new();
    for row in rows {
        let map = as_obj(row, "row")?;
        let row_id = str_field(map, "id", "row")?;
        for entry in vec_field(row, "rust_entrypoints") {
            let emap = as_obj(entry, "rust entrypoint")?;
            let name = emap.get("name").and_then(|v| v.as_str());
            match name {
                Some(n) => *inventoried.entry(n.to_owned()).or_insert(0) += int_field(emap, "count", "rust entrypoint")? as usize,
                None => return fail(format!("{row_id}: Rust entrypoint must have name/count")),
            }
        }
    }
    if extracted_entrypoints != inventoried {
        return fail(format!(
            "hardcoded Rust product entrypoints differ: extracted={extracted_entrypoints:?}, inventoried={inventoried:?}"
        ));
    }

    validate_anchors(root, rows)?;
    validate_scans(root, provenance, &ids)?;
    Ok((embedded, extracted_declarations, extracted_entrypoints))
}

fn esc(value: &str) -> String {
    value.replace('|', "\\|").replace('\n', " ")
}

pub fn render(provenance: &serde_json::Value, manifest: &serde_json::Value, embedded: &Memberships) -> Result<String> {
    let prov = as_obj(provenance, "provenance")?;
    let oracle = str_field(prov, "oracle", "provenance")?;
    let audit_base = str_field(prov, "audit_base", "provenance")?;
    let rows = as_arr(manifest.get("rows").ok_or_else(|| InventoryError::Message("manifest has no rows".into()))?, "rows")?;
    let mut lines: Vec<String> = vec![
        "# First-party construction inventory".to_owned(),
        String::new(),
        "Generated by `pi-rs-tools construction-inventory` from checked provenance and the".to_owned(),
        "embedded pack descriptors. Edit `tests/construction-inventory/manifest.json`,".to_owned(),
        "then regenerate. `--check` fails closed for missing/stale/duplicate sources,".to_owned(),
        "declarations, Rust seams, hardcoded product entrypoints, or named open rows.".to_owned(),
        String::new(),
        format!("Oracle: `{oracle}`. Audit base: `{audit_base}`."),
        String::new(),
        "| ID | Kind | Unit | Package(s) | Public declaration | Disable path | Replacement evidence | Status | Rung |".to_owned(),
        "|---|---|---|---|---|---|---|---|---|".to_owned(),
    ];
    let mut sorted_rows: Vec<(String, &serde_json::Value)> = rows
        .iter()
        .map(|r| {
            let id = str_field(as_obj(r, "row")?, "id", "row")?.to_owned();
            Ok::<_, InventoryError>((id, r))
        })
        .collect::<Result<Vec<_>>>()?;
    sorted_rows.sort_by(|a, b| a.0.cmp(&b.0));
    for (_, row) in sorted_rows {
        let map = as_obj(row, "row")?;
        let cell = |key: &str| -> Result<String> { Ok(esc(str_field(map, key, "row")?)) };
        let packages: Vec<String> = vec_field(row, "packages").iter().filter_map(|p| p.as_str()).map(|p| format!("`{}`", esc(p))).collect();
        let values = vec![
            format!("`{}`", esc(str_field(map, "id", "row")?)),
            cell("kind")?,
            cell("title")?,
            packages.join(", "),
            cell("public_declaration")?,
            cell("disable_path")?,
            cell("replacement_evidence")?,
            cell("status")?,
            cell("rung")?,
        ];
        lines.push(format!("| {} |", values.join(" | ")));
    }
    let mut status_counts: BTreeMap<&str, usize> = BTreeMap::new();
    for row in rows {
        let map = as_obj(row, "row")?;
        *status_counts.entry(str_field(map, "status", "row")?).or_insert(0) += 1;
    }
    let counts: Vec<String> = status_counts.iter().map(|(k, v)| format!("{k}={v}")).collect();
    lines.push(String::new());
    lines.push(format!(
        "Rows: {}; embedded source units: {}; {}.",
        rows.len(),
        embedded.len(),
        counts.join("; ")
    ));
    lines.push(String::new());
    Ok(lines.join("\n"))
}

/// Run the construction-inventory workflow against the repo at `root`.
/// With `check` the committed output must match the generated one.
pub fn run(root: &Path, check: bool, print_extracted: bool) -> Result<()> {
    let provenance = load_json(&root.join("tests/construction-inventory/provenance.json"))?;
    let manifest = load_json(&root.join("tests/construction-inventory/manifest.json"))?;
    let (embedded, declarations, entrypoints) = validate(root, &provenance, &manifest)?;
    if print_extracted {
        let json = serde_json::json!({
            "embedded": embedded.iter().map(|(k, v)| (k, v.iter().cloned().collect::<Vec<_>>())).collect::<BTreeMap<_, _>>(),
            "declarations": declarations.iter().cloned().collect::<Vec<_>>(),
            "rust_entrypoints": entrypoints,
        });
        println!("{}", serde_json::to_string_pretty(&json)?);
        return Ok(());
    }
    let generated = render(&provenance, &manifest, &embedded)?;
    let output = root.join("CONSTRUCTION_INVENTORY.md");
    if check {
        let current = std::fs::read_to_string(&output).unwrap_or_default();
        if current != generated {
            eprintln!("CONSTRUCTION_INVENTORY.md is stale; regenerate");
            return fail("CONSTRUCTION_INVENTORY.md is stale");
        }
        println!("construction inventory is complete and current");
    } else {
        std::fs::write(&output, &generated)?;
        println!("wrote CONSTRUCTION_INVENTORY.md");
    }
    Ok(())
}