//! Rust port of the deleted `tests/construction-inventory/test_checker.py`
//! (PLAN A.3): fail-closed negative controls for the first-party
//! construction-inventory checks, reimplemented in Rust. The checks
//! (embedded source coverage, declarations, Rust entrypoints, anchors,
//! seams, launch calls, named open rows) mirror `scripts/construction-inventory`
//! so the inventory workflow has a Rust owner that runs under
//! `cargo test --workspace`.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use fancy_regex::Regex;
use serde_json::{Map, Value};

const REQUIRED_FIELDS: &[&str] = &[
    "id", "kind", "title", "packages", "public_declaration", "disable_path",
    "replacement_evidence", "status", "rung", "coverage", "anchors", "declarations",
    "rust_entrypoints",
];
const ALLOWED_STATUS: &[&str] = &["implemented", "open", "DESIGN exception"];

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

// ---------------------------------------------------------------------
// Checker logic (port of scripts/construction-inventory)
// ---------------------------------------------------------------------

fn object<'a>(value: &'a Value, where_: &str) -> Result<&'a Map<String, Value>, String> {
    match value {
        Value::Object(map) => Ok(map),
        _ => Err(format!("{where_}: expected an object")),
    }
}

fn load_json(root: &Path, relative: &str) -> Result<Map<String, Value>, String> {
    let path = root.join(relative);
    let text = fs::read_to_string(&path).map_err(|error| format!("{relative}: cannot read: {error}"))?;
    let value: Value = serde_json::from_str(&text).map_err(|error| format!("{relative}: parse error: {error}"))?;
    Ok(object(&value, relative)?.clone())
}

fn const_block(source: &str, name: &str) -> Result<String, String> {
    let pattern = format!(r"\bpub\s+const\s+{}\b[^=]*=", fancy_regex::escape(name));
    let re = Regex::new(&pattern).map_err(|error| format!("const pattern: {error}"))?;
    let Some(mat) = re.find(source).map_err(|error| format!("const search: {error}"))? else {
        return Err(format!("missing embedded-pack constant {name}"));
    };
    let start = source[mat.end()..]
        .find('{')
        .map(|offset| mat.end() + offset)
        .ok_or_else(|| format!("missing body for embedded-pack constant {name}"))?;
    let mut depth = 0usize;
    for (offset, ch) in source[start..].char_indices() {
        match ch {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return Ok(source[start + 1..start + offset].to_string());
                }
            }
            _ => {}
        }
    }
    Err(format!("unterminated embedded-pack constant {name}"))
}

// Extracted embedded packs (package id -> unit names) plus per-package source literals.
type Extraction = (HashMap<String, HashSet<String>>, HashMap<String, String>);

fn extract_embedded(
    root: &Path,
    provenance: &Map<String, Value>,
) -> Result<Extraction, String> {
    let name_re = Regex::new(r#"\bname\s*:\s*"([^"]+)""#).unwrap();
    let include_re = Regex::new(r#"include_(?:str|bytes)!\("([^"]+)"\)"#).unwrap();
    let mut memberships: HashMap<String, HashSet<String>> = HashMap::new();
    let mut package_names: HashMap<String, String> = HashMap::new();
    let packages = provenance
        .get("embedded_packages")
        .and_then(Value::as_array)
        .ok_or_else(|| "provenance has no embedded_packages array".to_string())?;
    for package in packages {
        let package = object(package, "embedded package")?;
        let package_id = package["id"].as_str().ok_or("package id missing")?.to_string();
        let descriptor = package["descriptor"].as_str().ok_or("descriptor missing")?;
        let constant = package["constant"].as_str().ok_or("constant missing")?;
        let source_name = package["source_name"].as_str().ok_or("source_name missing")?;
        let descriptor_path = root.join(descriptor);
        let source = fs::read_to_string(&descriptor_path)
            .map_err(|error| format!("{package_id}: cannot read descriptor {descriptor}: {error}"))?;
        let block = const_block(&source, constant)?;
        let actual_name = name_re
            .captures(&block)
            .map_err(|error| format!("{package_id}: name search: {error}"))?
            .and_then(|captures| captures.get(1))
            .map(|matched| matched.as_str().to_string())
            .ok_or_else(|| format!("{package_id}: embedded pack has no literal name"))?;
        if actual_name != source_name {
            return Err(format!(
                "{package_id}: source name changed: expected {source_name:?}, got {actual_name:?}"
            ));
        }
        if package_names.contains_key(&package_id) {
            return Err(format!("duplicate embedded package id {package_id}"));
        }
        package_names.insert(package_id.clone(), actual_name);
        let mut found = false;
        for captures in include_re.captures_iter(&block) {
            let captures = captures.map_err(|error| format!("{package_id}: include match: {error}"))?;
            let include = captures.get(1).unwrap().as_str();
            found = true;
            let resolved = descriptor_path
                .parent()
                .unwrap_or_else(|| Path::new("."))
                .join(include);
            let canonical = resolved
                .canonicalize()
                .map_err(|error| format!("{package_id}: cannot resolve embedded source {include}: {error}"))?;
            let root_canonical = root
                .canonicalize()
                .map_err(|error| format!("root canonicalize: {error}"))?;
            let relative = canonical
                .strip_prefix(&root_canonical)
                .map_err(|_| format!("{package_id}: source escapes repository: {include}"))?;
            let relative = relative.to_string_lossy().replace('\\', "/");
            if !canonical.is_file() {
                return Err(format!("{package_id}: missing embedded source {relative}"));
            }
            memberships.entry(relative).or_default().insert(package_id.clone());
        }
        if !found {
            return Err(format!("{package_id}: embedded pack has no source units"));
        }
    }
    if package_names.is_empty() {
        return Err("provenance has no embedded packages".to_string());
    }
    Ok((memberships, package_names))
}

fn extract_declarations(
    root: &Path,
    embedded: &HashMap<String, HashSet<String>>,
) -> Result<HashSet<String>, String> {
    let decl_re = Regex::new(r#"pi\.register_(command|tool|role)\s*\(\s*(?:["']([^"']+)["']|\{)"#).unwrap();
    let field_re = Regex::new(r#"\b(role|name)\s*=\s*["']([^"']+)["']"#).unwrap();
    let mut declarations = HashSet::new();
    let mut sorted: Vec<&String> = embedded.keys().collect();
    sorted.sort();
    for relative in sorted {
        if !relative.ends_with(".lua") {
            continue;
        }
        let source = fs::read_to_string(root.join(relative))
            .map_err(|error| format!("{relative}: cannot read: {error}"))?;
        for captures in decl_re.captures_iter(&source) {
            let captures = captures.map_err(|error| format!("{relative}: declaration match: {error}"))?;
            let kind = captures.get(1).unwrap().as_str();
            let literal = captures.get(2).map(|matched| matched.as_str().to_string());
            let name = if kind == "command" {
                literal.ok_or_else(|| format!("{relative}: register_command must use a literal name for inventory"))?
            } else {
                let tail = &source[captures.get(0).unwrap().end()..];
                let field = if kind == "role" { "role" } else { "name" };
                field_re
                    .captures(tail)
                    .map_err(|error| format!("{relative}: field search: {error}"))?
                    .and_then(|captures| captures.get(2))
                    .map(|matched| matched.as_str().to_string())
                    .ok_or_else(|| format!("{relative}: register_{kind} must have a literal {field} for inventory"))?
            };
            let key = format!("{kind}:{name}");
            if !declarations.insert(key.clone()) {
                return Err(format!("duplicate embedded declaration {key}"));
            }
        }
    }
    Ok(declarations)
}

fn extract_rust_entrypoints(
    root: &Path,
    provenance: &Map<String, Value>,
) -> Result<Vec<(String, usize)>, String> {
    let pattern = provenance["rust_entrypoint_pattern"]
        .as_str()
        .ok_or("rust_entrypoint_pattern missing")?;
    let re = Regex::new(pattern).map_err(|error| format!("entrypoint pattern: {error}"))?;
    let mut found: HashMap<String, usize> = HashMap::new();
    let roots = provenance["rust_source_roots"]
        .as_array()
        .ok_or("rust_source_roots missing")?;
    for relative in roots {
        let relative = relative.as_str().ok_or("rust_source_root not a string")?;
        let base = root.join(relative);
        let mut files: Vec<PathBuf> = Vec::new();
        collect_rs(&base, &mut files);
        files.sort();
        for path in files {
            let source = fs::read_to_string(&path)
                .map_err(|error| format!("cannot read {}: {error}", path.display()))?;
            for captures in re.captures_iter(&source) {
                let captures = captures.map_err(|error| format!("entrypoint match: {error}"))?;
                if let Some(group) = captures.get(1) {
                    *found.entry(group.as_str().to_string()).or_insert(0) += 1;
                }
            }
        }
    }
    let mut list: Vec<(String, usize)> = found.into_iter().collect();
    list.sort();
    Ok(list)
}

fn collect_rs(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_rs(&path, out);
        } else if path.extension().and_then(|ext| ext.to_str()) == Some("rs") {
            out.push(path);
        }
    }
}

fn validate_anchors(root: &Path, rows: &[Value]) -> Result<(), String> {
    for row in rows {
        let row_id = row["id"].as_str().unwrap_or_default();
        let anchors = row["anchors"]
            .as_array()
            .ok_or_else(|| format!("{row_id}: anchors must be an array"))?;
        for anchor in anchors {
            let anchor = object(anchor, &format!("{row_id}: anchor"))?;
            let keys: HashSet<&str> = anchor.keys().map(|key| key.as_str()).collect();
            let expected: HashSet<&str> = ["path", "text", "count"].into_iter().collect();
            if keys != expected {
                return Err(format!("{row_id}: anchor fields must be path/text/count"));
            }
            let path = anchor["path"].as_str().ok_or("anchor path missing")?;
            let text = anchor["text"].as_str().ok_or("anchor text missing")?;
            let count = anchor["count"].as_u64().ok_or("anchor count missing")? as usize;
            let source = fs::read_to_string(root.join(path))
                .map_err(|error| format!("{row_id}: cannot read anchor {path}: {error}"))?;
            let actual = source.matches(text).count();
            if actual != count {
                return Err(format!(
                    "{row_id}: stale anchor {path}:{text:?}; expected {count}, got {actual}"
                ));
            }
        }
    }
    Ok(())
}

fn validate_scans(root: &Path, provenance: &Map<String, Value>, row_ids: &HashSet<String>) -> Result<(), String> {
    let mut seen = HashSet::new();
    for scan in provenance
        .get("rust_seams")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let scan = object(scan, "rust seam")?;
        let scan_id = scan["id"].as_str().ok_or("seam id missing")?;
        if !seen.insert(scan_id.to_string()) {
            return Err(format!("duplicate Rust seam provenance {scan_id}"));
        }
        let row = scan["row"].as_str().ok_or("seam row missing")?;
        if !row_ids.contains(row) {
            return Err(format!("Rust seam {scan_id}: missing row {row}"));
        }
        let path = scan["path"].as_str().ok_or("seam path missing")?;
        let text = scan["text"].as_str().ok_or("seam text missing")?;
        let count = scan["count"].as_u64().ok_or("seam count missing")? as usize;
        let source = fs::read_to_string(root.join(path))
            .map_err(|error| format!("Rust seam {scan_id}: cannot read {path}: {error}"))?;
        let actual = source.matches(text).count();
        if actual != count {
            return Err(format!(
                "Rust seam {scan_id} is stale: expected {count} occurrences of {text:?}, got {actual}"
            ));
        }
    }

    let mut expected_calls: HashMap<(String, String), usize> = HashMap::new();
    for call in provenance
        .get("rust_call_inventory")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let call = object(call, "rust call")?;
        let row = call["row"].as_str().ok_or("call row missing")?;
        if !row_ids.contains(row) {
            return Err(format!(
                "Rust call {}:{}: missing row {}",
                call["path"].as_str().unwrap_or_default(),
                call["operation"].as_str().unwrap_or_default(),
                row
            ));
        }
        let key = (
            call["path"].as_str().ok_or("call path missing")?.to_string(),
            call["operation"].as_str().ok_or("call operation missing")?.to_string(),
        );
        if expected_calls.contains_key(&key) {
            return Err(format!("duplicate Rust call inventory {}:{}", key.0, key.1));
        }
        expected_calls.insert(key, call["count"].as_u64().ok_or("call count missing")? as usize);
    }
    let operations: Vec<String> = provenance
        .get("rust_call_operations")
        .and_then(Value::as_array)
        .map(|ops| ops.iter().filter_map(Value::as_str).map(str::to_string).collect())
        .unwrap_or_default();
    let mut actual_calls: HashMap<(String, String), usize> = HashMap::new();
    for relative in provenance
        .get("rust_source_roots")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let relative = relative.as_str().ok_or("rust_source_root not a string")?;
        let base = root.join(relative);
        let mut files: Vec<PathBuf> = Vec::new();
        collect_rs(&base, &mut files);
        files.sort();
        for path in files {
            let source = fs::read_to_string(&path)
                .map_err(|error| format!("cannot read {}: {error}", path.display()))?;
            let relative_path = path
                .strip_prefix(root)
                .map(|p| p.to_string_lossy().replace('\\', "/"))
                .unwrap_or_default();
            for operation in &operations {
                let count = source.matches(&format!(".{operation}(")).count();
                if count > 0 {
                    *actual_calls.entry((relative_path.clone(), operation.clone())).or_insert(0) += count;
                }
            }
        }
    }
    if actual_calls != expected_calls {
        let printable = |values: &HashMap<(String, String), usize>| -> Vec<String> {
            let mut items: Vec<((&String, &String), &usize)> = values.iter().map(|((a, b), c)| ((a, b), c)).collect();
            items.sort_by(|a, b| a.0.cmp(&b.0));
            items
                .iter()
                .map(|((path, operation), count)| format!("{path}:{operation}: {count}"))
                .collect()
        };
        return Err(format!(
            "Rust launch/composition calls differ: extracted={:?}, inventoried={:?}",
            printable(&actual_calls),
            printable(&expected_calls)
        ));
    }
    Ok(())
}

struct Extracted {
    embedded: HashMap<String, HashSet<String>>,
}

fn validate(root: &Path, provenance: &Map<String, Value>, manifest: &Map<String, Value>) -> Result<Extracted, String> {
    if provenance.get("schema").and_then(Value::as_u64) != Some(1)
        || manifest.get("schema").and_then(Value::as_u64) != Some(1)
    {
        return Err("provenance/manifest schema must be 1".to_string());
    }
    if provenance.get("oracle").and_then(Value::as_str) != Some("Pi v0.79.0 c5582102") {
        return Err("provenance oracle must be Pi v0.79.0 c5582102".to_string());
    }
    let rows = manifest
        .get("rows")
        .and_then(Value::as_array)
        .filter(|rows| !rows.is_empty())
        .ok_or_else(|| "manifest rows must be a non-empty array".to_string())?;

    let mut ids = HashSet::new();
    let id_re = Regex::new(r"^[a-z0-9][a-z0-9._-]*$").unwrap();
    for row in rows {
        let fields: HashSet<String> = row
            .as_object()
            .map(|map| map.keys().cloned().collect())
            .unwrap_or_default();
        let required: HashSet<String> = REQUIRED_FIELDS.iter().map(|key| key.to_string()).collect();
        if fields != required {
            let missing: Vec<&str> = REQUIRED_FIELDS
                .iter()
                .filter(|key| !fields.contains(**key))
                .copied()
                .collect();
            let extra: Vec<&String> = fields.difference(&required).collect();
            return Err(format!(
                "{}: row fields differ; missing={missing:?}, extra={extra:?}",
                row["id"].as_str().unwrap_or("<unknown>")
            ));
        }
        let row_id = row["id"].as_str().ok_or("row id missing")?;
        if !id_re.is_match(row_id).map_err(|error| format!("row id regex: {error}"))? {
            return Err(format!("invalid row id {row_id:?}"));
        }
        if !ids.insert(row_id.to_string()) {
            return Err(format!("duplicate row id {row_id}"));
        }
        for field in ["kind", "title", "public_declaration", "disable_path", "replacement_evidence", "rung"] {
            let value = row[field].as_str().map(str::trim).unwrap_or_default();
            if value.is_empty() {
                return Err(format!("{row_id}: {field} is required"));
            }
        }
        if !ALLOWED_STATUS.contains(&row["status"].as_str().unwrap_or_default()) {
            return Err(format!("{row_id}: invalid status {:?}", row["status"]));
        }
        let packages = row["packages"].as_array().filter(|packages| !packages.is_empty());
        if packages.is_none() {
            return Err(format!("{row_id}: packages must be non-empty"));
        }
        for field in ["coverage", "anchors", "declarations", "rust_entrypoints"] {
            if !row[field].is_array() {
                return Err(format!("{row_id}: {field} must be an array"));
            }
        }
    }

    let required_open: HashSet<String> = provenance
        .get("required_open_rows")
        .and_then(Value::as_array)
        .map(|rows| rows.iter().filter_map(Value::as_str).map(str::to_string).collect())
        .unwrap_or_default();
    let missing_open: Vec<&String> = required_open.iter().filter(|id| !ids.contains(*id)).collect();
    if !missing_open.is_empty() {
        let mut sorted = missing_open.clone();
        sorted.sort();
        return Err(format!("missing named open rows: {sorted:?}"));
    }
    for row in rows {
        let row_id = row["id"].as_str().unwrap_or_default();
        if required_open.contains(row_id) && row["status"].as_str() != Some("open") {
            return Err(format!("{row_id}: named row must remain open until its owning rung closes it"));
        }
    }

    let (embedded, package_names) = extract_embedded(root, provenance)?;
    let mut known_packages: HashSet<String> = package_names.keys().cloned().collect();
    if let Some(non_embedded) = provenance.get("non_embedded_packages").and_then(Value::as_array) {
        for id in non_embedded.iter().filter_map(Value::as_str) {
            known_packages.insert(id.to_string());
        }
    }
    let mut coverage: HashMap<String, Vec<String>> = HashMap::new();
    for row in rows {
        let row_id = row["id"].as_str().unwrap_or_default();
        let row_packages: HashSet<String> = row["packages"]
            .as_array()
            .map(|packages| packages.iter().filter_map(Value::as_str).map(str::to_string).collect())
            .unwrap_or_default();
        for package in &row_packages {
            if !known_packages.contains(package) {
                return Err(format!("{row_id}: unknown packages {package}"));
            }
        }
        for source in row["coverage"].as_array().into_iter().flatten() {
            let source = source.as_str().ok_or("coverage entry not a string")?;
            coverage.entry(source.to_string()).or_default().push(row_id.to_string());
        }
    }
    let mut missing_sources: Vec<String> = embedded
        .keys()
        .filter(|source| !coverage.contains_key(*source))
        .cloned()
        .collect();
    missing_sources.sort();
    let mut stale_sources: Vec<String> = coverage
        .keys()
        .filter(|source| !embedded.contains_key(*source))
        .cloned()
        .collect();
    stale_sources.sort();
    let duplicate_sources: HashMap<String, Vec<String>> = coverage
        .iter()
        .filter(|(_, owners)| owners.len() != 1)
        .map(|(source, owners)| (source.clone(), owners.clone()))
        .collect();
    if !missing_sources.is_empty() || !stale_sources.is_empty() || !duplicate_sources.is_empty() {
        return Err(format!(
            "embedded source coverage differs: missing={missing_sources:?}, stale={stale_sources:?}, duplicates={duplicate_sources:?}"
        ));
    }
    let by_id: HashMap<&str, &Value> = rows
        .iter()
        .map(|row| (row["id"].as_str().unwrap_or_default(), row))
        .collect();
    for (source, packages) in &embedded {
        let owner = &coverage[source][0];
        let row = by_id[owner.as_str()];
        let row_packages: HashSet<String> = row["packages"]
            .as_array()
            .map(|packages| packages.iter().filter_map(Value::as_str).map(str::to_string).collect())
            .unwrap_or_default();
        let expected: HashSet<String> = packages.iter().cloned().collect();
        if row_packages != expected {
            let mut expected_sorted: Vec<&String> = expected.iter().collect();
            let mut actual_sorted: Vec<&String> = row_packages.iter().collect();
            expected_sorted.sort();
            actual_sorted.sort();
            return Err(format!(
                "{owner}: package membership for {source} differs: expected {expected_sorted:?}, got {actual_sorted:?}"
            ));
        }
    }

    let extracted_declarations = extract_declarations(root, &embedded)?;
    let mut declared: HashMap<String, Vec<String>> = HashMap::new();
    for row in rows {
        let row_id = row["id"].as_str().unwrap_or_default();
        for declaration in row["declarations"].as_array().into_iter().flatten() {
            let declaration = declaration.as_str().ok_or("declaration not a string")?;
            declared.entry(declaration.to_string()).or_default().push(row_id.to_string());
        }
    }
    let mut missing_declarations: Vec<String> = extracted_declarations
        .iter()
        .filter(|key| !declared.contains_key(*key))
        .cloned()
        .collect();
    missing_declarations.sort();
    let mut stale_declarations: Vec<String> = declared
        .keys()
        .filter(|key| !extracted_declarations.contains(*key))
        .cloned()
        .collect();
    stale_declarations.sort();
    let duplicate_declarations: HashMap<String, Vec<String>> = declared
        .iter()
        .filter(|(_, owners)| owners.len() != 1)
        .map(|(key, owners)| (key.clone(), owners.clone()))
        .collect();
    if !missing_declarations.is_empty() || !stale_declarations.is_empty() || !duplicate_declarations.is_empty() {
        return Err(format!(
            "embedded declarations differ: missing={missing_declarations:?}, stale={stale_declarations:?}, duplicates={duplicate_declarations:?}"
        ));
    }

    let extracted_entrypoints = extract_rust_entrypoints(root, provenance)?;
    let mut inventoried: HashMap<String, usize> = HashMap::new();
    for row in rows {
        let row_id = row["id"].as_str().unwrap_or_default();
        for entry in row["rust_entrypoints"].as_array().into_iter().flatten() {
            let entry = object(entry, &format!("{row_id}: Rust entrypoint"))?;
            let keys: HashSet<&str> = entry.keys().map(|key| key.as_str()).collect();
            if keys != HashSet::from(["name", "count"]) || !entry["count"].is_u64() {
                return Err(format!("{row_id}: Rust entrypoint must have name/count"));
            }
            *inventoried
                .entry(entry["name"].as_str().unwrap_or_default().to_string())
                .or_insert(0) += entry["count"].as_u64().unwrap_or_default() as usize;
        }
    }
    let mut inventoried_sorted: Vec<(String, usize)> = inventoried.into_iter().collect();
    inventoried_sorted.sort();
    if extracted_entrypoints != inventoried_sorted {
        return Err(format!(
            "hardcoded Rust product entrypoints differ: extracted={extracted_entrypoints:?}, inventoried={inventoried_sorted:?}"
        ));
    }

    validate_anchors(root, rows)?;
    validate_scans(root, provenance, &ids)?;
    let _ = (&extracted_declarations, &extracted_entrypoints);
    Ok(Extracted { embedded })
}

fn render(
    provenance: &Map<String, Value>,
    manifest: &Map<String, Value>,
    embedded: &HashMap<String, HashSet<String>>,
) -> String {
    let rows = manifest["rows"].as_array().unwrap();
    let mut lines = vec![
        "# First-party construction inventory".to_string(),
        String::new(),
        "Generated by `scripts/construction-inventory` from checked provenance and the".to_string(),
        "embedded pack descriptors. Edit `tests/construction-inventory/manifest.json`,".to_string(),
        "then regenerate. `--check` fails closed for missing/stale/duplicate sources,".to_string(),
        "declarations, Rust seams, hardcoded product entrypoints, or named open rows.".to_string(),
        String::new(),
        format!(
            "Oracle: `{}`. Audit base: `{}`.",
            provenance["oracle"].as_str().unwrap_or_default(),
            provenance["audit_base"].as_str().unwrap_or_default()
        ),
        String::new(),
        "| ID | Kind | Unit | Package(s) | Public declaration | Disable path | Replacement evidence | Status | Rung |".to_string(),
        "|---|---|---|---|---|---|---|---|---|".to_string(),
    ];
    let mut sorted_rows = rows.clone();
    sorted_rows.sort_by(|a, b| a["id"].as_str().cmp(&b["id"].as_str()));
    for row in &sorted_rows {
        let values = [
            format!("`{}`", row["id"].as_str().unwrap_or_default()),
            row["kind"].as_str().unwrap_or_default().to_string(),
            row["title"].as_str().unwrap_or_default().to_string(),
            row["packages"]
                .as_array()
                .map(|packages| {
                    packages
                        .iter()
                        .filter_map(Value::as_str)
                        .map(|package| format!("`{package}`"))
                        .collect::<Vec<_>>()
                        .join(", ")
                })
                .unwrap_or_default(),
            row["public_declaration"].as_str().unwrap_or_default().to_string(),
            row["disable_path"].as_str().unwrap_or_default().to_string(),
            row["replacement_evidence"].as_str().unwrap_or_default().to_string(),
            row["status"].as_str().unwrap_or_default().to_string(),
            row["rung"].as_str().unwrap_or_default().to_string(),
        ];
        lines.push(format!(
            "| {} |",
            values
                .iter()
                .map(|value| value.replace('|', "\\|").replace('\n', " "))
                .collect::<Vec<_>>()
                .join(" | ")
        ));
    }
    let mut status_counts: HashMap<&str, usize> = HashMap::new();
    for row in rows {
        *status_counts.entry(row["status"].as_str().unwrap_or_default()).or_insert(0) += 1;
    }
    let mut statuses: Vec<&str> = status_counts.keys().copied().collect();
    statuses.sort();
    let summary = statuses
        .iter()
        .map(|status| format!("{status}={}", status_counts[status]))
        .collect::<Vec<_>>()
        .join("; ");
    lines.push(String::new());
    lines.push(format!(
        "Rows: {}; embedded source units: {}; {summary}.",
        rows.len(),
        embedded.len()
    ));
    lines.push(String::new());
    lines.join("\n")
}

fn check(root: &Path) -> Result<(), String> {
    let provenance = load_json(root, "tests/construction-inventory/provenance.json")?;
    let manifest = load_json(root, "tests/construction-inventory/manifest.json")?;
    let extracted = validate(root, &provenance, &manifest)?;
    let generated = render(&provenance, &manifest, &extracted.embedded);
    let output_path = root.join("CONSTRUCTION_INVENTORY.md");
    let current = fs::read_to_string(&output_path).unwrap_or_default();
    if current != generated {
        return Err("CONSTRUCTION_INVENTORY.md is stale; run scripts/construction-inventory".to_string());
    }
    Ok(())
}

// ---------------------------------------------------------------------
// Test scaffolding (mirror of the deleted python negative controls)
// ---------------------------------------------------------------------

struct Sandbox {
    _temp: tempfile::TempDir,
    root: PathBuf,
}

fn copy_tree(from: &Path, to: &Path) {
    fs::create_dir_all(to).unwrap();
    for entry in fs::read_dir(from).unwrap().flatten() {
        let target = to.join(entry.file_name());
        let path = entry.path();
        if path.is_dir() {
            copy_tree(&path, &target);
        } else {
            fs::copy(&path, &target).unwrap();
        }
    }
}

fn sandbox() -> Sandbox {
    let repo = repo_root();
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().to_path_buf();
    for relative in [
        "PLAN.md",
        "CONSTRUCTION_INVENTORY.md",
        "tests/construction-inventory/provenance.json",
        "tests/construction-inventory/manifest.json",
        "crates/pi-rs-agent/src/lib.rs",
        "crates/pi-rs-agent/lua",
        "crates/pi-rs-app/src",
        "crates/pi-rs-host/src/lib.rs",
    ] {
        let source = repo.join(relative);
        let destination = root.join(relative);
        fs::create_dir_all(destination.parent().unwrap()).unwrap();
        if source.is_dir() {
            copy_tree(&source, &destination);
        } else {
            fs::copy(&source, &destination).unwrap();
        }
    }
    Sandbox { _temp: temp, root }
}

fn assert_rejected(root: &Path, expected: &str) {
    match check(root) {
        Ok(()) => panic!("expected rejection containing {expected:?}, but check passed"),
        Err(error) => assert!(
            error.contains(expected),
            "expected {expected:?} in error, got: {error}"
        ),
    }
}

fn read_manifest(root: &Path) -> Map<String, Value> {
    let text = fs::read_to_string(root.join("tests/construction-inventory/manifest.json")).unwrap();
    serde_json::from_str(&text).unwrap()
}

fn write_manifest(root: &Path, manifest: &Map<String, Value>) {
    let text = format!("{}\n", serde_json::to_string_pretty(manifest).unwrap());
    fs::write(root.join("tests/construction-inventory/manifest.json"), text).unwrap();
}

// ---------------------------------------------------------------------
// Negative controls
// ---------------------------------------------------------------------

#[test]
fn generation_is_byte_idempotent() {
    let sb = sandbox();
    let provenance = load_json(&sb.root, "tests/construction-inventory/provenance.json").unwrap();
    let manifest = load_json(&sb.root, "tests/construction-inventory/manifest.json").unwrap();
    let first = validate(&sb.root, &provenance, &manifest).unwrap();
    let first_text = render(&provenance, &manifest, &first.embedded);
    let second = validate(&sb.root, &provenance, &manifest).unwrap();
    let second_text = render(&provenance, &manifest, &second.embedded);
    assert_eq!(first_text, second_text);
    let checked = fs::read_to_string(sb.root.join("CONSTRUCTION_INVENTORY.md")).unwrap();
    assert_eq!(first_text, checked);
    check(&sb.root).unwrap();
}

#[test]
fn unclassified_embedded_source_is_rejected() {
    let sb = sandbox();
    fs::write(
        sb.root.join("crates/pi-rs-app/src/builtins/tools/new-policy.lua"),
        "local new_policy = true\n",
    )
    .unwrap();
    let descriptor = sb.root.join("crates/pi-rs-app/src/builtins/mod.rs");
    let source = fs::read_to_string(&descriptor).unwrap();
    let patched = source.replace(
        "include_str!(\"tools/prelude.lua\"),",
        "include_str!(\"tools/new-policy.lua\"),\n        include_str!(\"tools/prelude.lua\"),",
    );
    assert_ne!(source, patched, "anchor line not found in builtins/mod.rs");
    fs::write(&descriptor, patched).unwrap();
    assert_rejected(&sb.root, "embedded source coverage differs");
}

#[test]
fn unclassified_public_declaration_is_rejected() {
    let sb = sandbox();
    let frontend = sb.root.join("crates/pi-rs-app/src/builtins/interactive.lua");
    let source = fs::read_to_string(&frontend).unwrap();
    fs::write(
        &frontend,
        format!("{source}\npi.register_command(\"unclassified-policy\", {{ handler = function() end }})\n"),
    )
    .unwrap();
    assert_rejected(&sb.root, "embedded declarations differ");
}

#[test]
fn duplicate_declaration_owner_is_rejected() {
    let sb = sandbox();
    let mut manifest = read_manifest(&sb.root);
    let rows = manifest["rows"].as_array_mut().unwrap();
    let row = rows
        .iter_mut()
        .find(|row| row["id"] == "tool.bash")
        .expect("tool.bash row missing");
    row["declarations"]
        .as_array_mut()
        .unwrap()
        .push(Value::String("tool:read".to_string()));
    write_manifest(&sb.root, &manifest);
    assert_rejected(&sb.root, "duplicates=");
}

#[test]
fn stale_source_row_is_rejected() {
    let sb = sandbox();
    let mut manifest = read_manifest(&sb.root);
    let rows = manifest["rows"].as_array_mut().unwrap();
    let row = rows
        .iter_mut()
        .find(|row| row["id"] == "tool.read")
        .expect("tool.read row missing");
    row["coverage"] = serde_json::json!(["crates/pi-rs-app/src/builtins/tools/removed.lua"]);
    write_manifest(&sb.root, &manifest);
    assert_rejected(&sb.root, "stale=");
}

#[test]
fn hardcoded_product_entrypoint_is_rejected() {
    let sb = sandbox();
    let main = sb.root.join("crates/pi-rs-app/src/main.rs");
    let source = fs::read_to_string(&main).unwrap();
    fs::write(&main, format!("{source}\nconst BAD: &str = \"pi-rs-run\";\n")).unwrap();
    assert_rejected(&sb.root, "hardcoded Rust product entrypoints differ");
}

#[test]
fn stale_rust_seam_is_rejected() {
    let sb = sandbox();
    let main = sb.root.join("crates/pi-rs-app/src/main.rs");
    let source = fs::read_to_string(&main).unwrap();
    let patched = source.replacen("let app_mode = if", "let selected_app_mode = if", 1);
    assert_ne!(source, patched, "stale-anchor line not found");
    fs::write(&main, patched).unwrap();
    assert_rejected(&sb.root, "stale anchor");
}

#[test]
fn unclassified_rust_launch_call_is_rejected() {
    let sb = sandbox();
    let main = sb.root.join("crates/pi-rs-app/src/main.rs");
    let source = fs::read_to_string(&main).unwrap();
    fs::write(
        &main,
        format!("{source}\n// inventory negative control: host.call_command(name, args);\n"),
    )
    .unwrap();
    assert_rejected(&sb.root, "Rust launch/composition calls differ");
}

#[test]
fn missing_named_open_row_is_rejected() {
    let sb = sandbox();
    let mut manifest = read_manifest(&sb.root);
    manifest["rows"]
        .as_array_mut()
        .unwrap()
        .retain(|row| row["id"] != "modules.chunk-local-helpers");
    write_manifest(&sb.root, &manifest);
    assert_rejected(&sb.root, "missing named open rows");
}
