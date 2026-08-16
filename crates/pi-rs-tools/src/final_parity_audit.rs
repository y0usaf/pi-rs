//! A.3 Rust owner for the closed Pi v0.79.0 final surface audit.
//!
//! Faithful port of the former `scripts/final-parity-audit` (Python). Normal
//! checks consume the checked reference graph and manifest and need no git or
//! Python: `check` validates `tests/final-parity-audit/{reference,manifest}.json`
//! and renders/compares `FINAL_PARITY_AUDIT.md`; `selftest` runs the same
//! fail-closed negative controls as the Python port. `--update-ref` /
//! `--verify-ref` mechanically re-extract the pinned commit through git.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use fancy_regex::Regex;
use sha2::{Digest, Sha256};

#[derive(Debug, thiserror::Error)]
pub enum AuditError {
    #[error("final parity audit: {0}")]
    Message(String),
    #[error("final parity audit: {0}")]
    Json(#[from] serde_json::Error),
    #[error("final parity audit: {0}")]
    Io(#[from] std::io::Error),
}

type Result<T> = std::result::Result<T, AuditError>;

pub const ORACLE: &str = "Pi v0.79.0 c5582102";
pub const ORACLE_COMMIT: &str = "c5582102f51b143fadc05180e0f8aed050e923b3";
pub const PACKAGES: &[&str] = &["coding-agent", "ai", "agent", "tui"];

fn fail<T>(msg: impl Into<String>) -> Result<T> {
    Err(AuditError::Message(msg.into()))
}

pub fn digest(value: &serde_json::Value) -> String {
    let raw = serde_json::to_string(value).unwrap_or_default();
    let mut hasher = Sha256::new();
    hasher.update(raw.as_bytes());
    hasher.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

fn load_json(path: &Path) -> Result<serde_json::Value> {
    let value: serde_json::Value = serde_json::from_slice(&std::fs::read(path)?)?;
    if !value.is_object() {
        return fail(format!("{}: expected JSON object", path.display()));
    }
    Ok(value)
}

fn as_obj<'a>(value: &'a serde_json::Value, what: &str) -> Result<&'a serde_json::Map<String, serde_json::Value>> {
    value
        .as_object()
        .ok_or_else(|| AuditError::Message(format!("{what}: expected an object")))
}

fn as_arr<'a>(value: &'a serde_json::Value, what: &str) -> Result<&'a Vec<serde_json::Value>> {
    value
        .as_array()
        .ok_or_else(|| AuditError::Message(format!("{what}: expected an array")))
}

fn str_field<'a>(map: &'a serde_json::Map<String, serde_json::Value>, key: &str, what: &str) -> Result<&'a str> {
    map.get(key)
        .and_then(|v| v.as_str())
        .ok_or_else(|| AuditError::Message(format!("{what}: field {key:?} must be a string")))
}

fn re(pattern: &str) -> Result<Regex> {
    Regex::new(pattern).map_err(|e| AuditError::Message(format!("invalid regex {pattern:?}: {e}")))
}

/// Validate the checked reference + manifest and classify every source file.
/// Returns a map from file path to its classification row (`group`).
pub fn validate(
    root: &Path,
    reference: &serde_json::Value,
    manifest: &serde_json::Value,
    check_evidence: bool,
) -> Result<BTreeMap<String, serde_json::Value>> {
    if reference.get("oracle").and_then(|v| v.as_str()) != Some(ORACLE)
        || reference.get("commit").and_then(|v| v.as_str()) != Some(ORACLE_COMMIT)
    {
        return fail("reference fixture has the wrong oracle/commit");
    }
    let files = reference
        .get("files")
        .and_then(|v| v.as_array())
        .ok_or_else(|| AuditError::Message("reference fixture has no files".into()))?;
    if files.is_empty() {
        return fail("reference fixture has no files");
    }
    let paths: Vec<&str> = files
        .iter()
        .map(|r| r.get("path").and_then(|v| v.as_str()).unwrap_or(""))
        .collect();
    if paths.iter().any(|p| p.is_empty()) {
        return fail("reference fixture has an invalid file path");
    }
    let unique: BTreeSet<&str> = paths.iter().copied().collect();
    if unique.len() != paths.len() {
        return fail("reference fixture has duplicate file paths");
    }
    let mut sorted_theirs = paths.clone();
    sorted_theirs.sort();
    if paths != sorted_theirs {
        return fail("reference fixture file paths are not sorted");
    }
    if manifest.get("schema").and_then(|v| v.as_i64()) != Some(1)
        || manifest.get("oracle").and_then(|v| v.as_str()) != Some(ORACLE)
    {
        return fail("manifest has the wrong schema/oracle");
    }
    let expected_digest = digest(reference);
    if manifest.get("reference_sha256").and_then(|v| v.as_str()) != Some(expected_digest.as_str()) {
        return fail("manifest reference_sha256 is stale; regenerate/review the pinned reference fixture");
    }
    let differences = manifest
        .get("differences")
        .and_then(|v| v.as_array())
        .ok_or_else(|| AuditError::Message("manifest has no differences".into()))?;
    let expected_nums: Vec<i64> = (1..=6).collect();
    let got_nums: Vec<i64> = differences.iter().map(|r| r.get("number").and_then(|v| v.as_i64()).unwrap_or(0)).collect();
    if got_nums != expected_nums {
        return fail("manifest must classify each DESIGN difference 1-6 exactly once");
    }
    let plan_re = re(r"PLAN (?:9(?:\.[0-9]+[ab]?)?|11)")?;
    for row in differences {
        let map = as_obj(row, "difference")?;
        let number = map.get("number").and_then(|v| v.as_i64()).ok_or_else(|| AuditError::Message("difference has no number".into()))?;
        let status = str_field(map, "status", "difference")?;
        let owner = str_field(map, "owner", "difference")?;
        let finding = str_field(map, "finding", "difference")?;
        let evidence = map.get("evidence").and_then(|v| v.as_array()).ok_or_else(|| AuditError::Message(format!("DESIGN difference {number}: evidence is required")))?;
        if !matches!(status, "bounded" | "open") || owner.trim().is_empty() || finding.trim().is_empty() {
            return fail(format!("DESIGN difference {number}: status, owner, and finding are required"));
        }
        if status == "bounded" && owner != format!("DESIGN difference {number}") {
            return fail(format!("DESIGN difference {number}: bounded owner is invalid"));
        }
        if status == "open" {
            let ok = plan_re.is_match(owner).map_err(|e| AuditError::Message(format!("regex: {e}")))?;
            if !ok {
                return fail(format!("DESIGN difference {number}: open owner must name PLAN 9.x/11"));
            }
        }
        if evidence.is_empty() {
            return fail(format!("DESIGN difference {number}: evidence is required"));
        }
        if check_evidence {
            for item in evidence {
                let emap = as_obj(item, "evidence")?;
                let extra: Vec<&str> = emap.keys().map(|k| k.as_str()).filter(|k| !matches!(*k, "path" | "contains")).collect();
                if !extra.is_empty() {
                    return fail(format!("DESIGN difference {number}: invalid evidence entry"));
                }
                let evidence_path = root.join(str_field(emap, "path", "evidence")?);
                if !evidence_path.is_file() {
                    return fail(format!("DESIGN difference {number}: missing evidence {}", evidence_path.display()));
                }
                if let Some(needle) = emap.get("contains").and_then(|v| v.as_str()) {
                    let text = std::fs::read_to_string(&evidence_path).unwrap_or_default();
                    if !text.contains(needle) {
                        return fail(format!("DESIGN difference {number}: stale evidence anchor {needle:?}"));
                    }
                }
            }
        }
    }
    let groups = manifest
        .get("groups")
        .and_then(|v| v.as_array())
        .ok_or_else(|| AuditError::Message("manifest groups must be a non-empty list".into()))?;
    if groups.is_empty() {
        return fail("manifest groups must be a non-empty list");
    }
    let allowed = ["parity", "open", "design", "out-of-scope"];
    let mut classified: BTreeMap<String, serde_json::Value> = BTreeMap::new();
    let mut group_ids: BTreeSet<String> = BTreeSet::new();
    let open_owner_re = re(r"PLAN (?:8|9(?:\.[0-9]+[ab]?)?|10|11)")?;
    for group in groups {
        let gmap = as_obj(group, "group")?;
        let group_id = gmap.get("id").and_then(|v| v.as_str()).ok_or_else(|| AuditError::Message("missing/duplicate manifest group id".into()))?;
        if group_id.trim().is_empty() || !group_ids.insert(group_id.to_owned()) {
            return fail(format!("missing/duplicate manifest group id {group_id:?}"));
        }
        let status = str_field(gmap, "status", "group")?;
        if !allowed.contains(&status) {
            return fail(format!("{group_id}: invalid status {status:?}"));
        }
        let owner = str_field(gmap, "owner", "group")?;
        let rationale = str_field(gmap, "rationale", "group")?;
        let evidence = gmap.get("evidence").and_then(|v| v.as_array()).ok_or_else(|| AuditError::Message(format!("{group_id}: owner, rationale, and evidence are required")))?;
        let refs = gmap.get("refs").and_then(|v| v.as_array()).ok_or_else(|| AuditError::Message(format!("{group_id}: refs must be a non-empty list")))?;
        if owner.trim().is_empty() || rationale.trim().is_empty() || evidence.is_empty() {
            return fail(format!("{group_id}: owner, rationale, and evidence are required"));
        }
        if status == "open" {
            let ok = open_owner_re.is_match(owner).map_err(|e| AuditError::Message(format!("regex: {e}")))?;
            if !ok {
                return fail(format!("{group_id}: open owner must name a PLAN 8/9.x/10/11 item"));
            }
        }
        if status == "design" {
            let d_re = re(r"DESIGN difference [1-6]")?;
            let ok = d_re.is_match(owner).map_err(|e| AuditError::Message(format!("regex: {e}")))?;
            if !ok {
                return fail(format!("{group_id}: design owner must name an exhaustive DESIGN difference"));
            }
        }
        if status == "out-of-scope" && owner != "DESIGN product boundary" {
            return fail(format!("{group_id}: out-of-scope owner must be DESIGN product boundary"));
        }
        if refs.is_empty() {
            return fail(format!("{group_id}: refs must be a non-empty list"));
        }
        for rref in refs {
            let ref_str = rref.as_str().ok_or_else(|| AuditError::Message(format!("{group_id}: invalid reference row")))?;
            if ref_str.trim().is_empty() {
                return fail(format!("{group_id}: invalid reference row"));
            }
            if classified.contains_key(ref_str) {
                let prev = classified.get(ref_str).and_then(|r| r.get("id")).and_then(|v| v.as_str()).unwrap_or("?");
                return fail(format!("{ref_str}: classified by both {prev} and {group_id}"));
            }
            classified.insert(ref_str.to_owned(), serde_json::json!({ "id": group_id, "status": status }));
        }
        if check_evidence {
            for item in evidence {
                let emap = as_obj(item, "evidence")?;
                let extra: Vec<&str> = emap.keys().map(|k| k.as_str()).filter(|k| !matches!(*k, "path" | "contains")).collect();
                if !extra.is_empty() {
                    return fail(format!("{group_id}: invalid evidence entry"));
                }
                let evidence_path = root.join(str_field(emap, "path", "evidence")?);
                if !evidence_path.is_file() {
                    return fail(format!("{group_id}: missing evidence {}", evidence_path.display()));
                }
                if let Some(needle) = emap.get("contains").and_then(|v| v.as_str()) {
                    let text = std::fs::read_to_string(&evidence_path).unwrap_or_default();
                    if !text.contains(needle) {
                        return fail(format!("{group_id}: stale evidence anchor {}: {needle:?}", evidence_path.display()));
                    }
                }
            }
        }
    }
    let expected: BTreeSet<&str> = paths.iter().copied().collect();
    let actual: BTreeSet<&str> = classified.keys().map(|k| k.as_str()).collect();
    let missing: Vec<&str> = expected.difference(&actual).copied().collect();
    let stale: Vec<&str> = actual.difference(&expected).copied().collect();
    if !missing.is_empty() || !stale.is_empty() {
        return fail(format!("classification closure failed: missing={missing:?}, stale={stale:?}"));
    }
    Ok(classified)
}

fn esc(value: &str) -> String {
    value.replace('|', "\\|")
}

/// Render `FINAL_PARITY_AUDIT.md` byte-for-byte from the reference + manifest.
pub fn render(
    _root: &Path,
    reference: &serde_json::Value,
    manifest: &serde_json::Value,
    classified: &BTreeMap<String, serde_json::Value>,
) -> Result<String> {
    let files = as_arr(reference.get("files").ok_or_else(|| AuditError::Message("no files".into()))?, "files")?;
    let public = as_arr(reference.get("public_exports").ok_or_else(|| AuditError::Message("no public_exports".into()))?, "public_exports")?;
    let mut package_counts: BTreeMap<String, usize> = BTreeMap::new();
    let mut status_counts: BTreeMap<&str, usize> = BTreeMap::new();
    let mut public_status: BTreeMap<&str, usize> = BTreeMap::new();
    let mut unreachable = 0usize;
    for row in files {
        let map = as_obj(row, "file")?;
        let path = str_field(map, "path", "file")?;
        let package = path.split('/').nth(1).unwrap_or("").to_owned();
        *package_counts.entry(package).or_insert(0) += 1;
        let status = classified.get(path).and_then(|c| c.get("status")).and_then(|v| v.as_str()).unwrap_or("");
        *status_counts.entry(status).or_insert(0) += 1;
        let reached = map.get("reached_by").and_then(|v| v.as_array()).map(|a| a.is_empty()).unwrap_or(true);
        if reached {
            unreachable += 1;
        }
    }
    for row in public {
        let map = as_obj(row, "public")?;
        let origin = str_field(map, "origin", "public")?;
        let status = classified.get(origin).and_then(|c| c.get("status")).and_then(|v| v.as_str()).unwrap_or("");
        *public_status.entry(status).or_insert(0) += 1;
    }

    let package_summary: Vec<String> = package_counts
        .iter()
        .map(|(k, v)| format!("{k}={v}"))
        .collect();
    let mut lines: Vec<String> = vec![
        "# Final parity audit".to_owned(),
        String::new(),
        format!("Oracle: `{ORACLE}` (`{ORACLE_COMMIT}`). Generated by"),
        "`scripts/final-parity-audit`; edit the checked manifest, then regenerate.".to_owned(),
        "Normal checks are offline: the checked reference graph carries pinned hashes;".to_owned(),
        "`--verify-ref PATH` mechanically reproduces it from the vendored git revision.".to_owned(),
        String::new(),
        "## Scope and closure".to_owned(),
        String::new(),
        "- Every file under the four pinned `src/` trees is hashed and classified.".to_owned(),
        "- Public exports are resolved from package entrypoints; executable and import".to_owned(),
        "  reachability is traversed across local and Pi-package imports.".to_owned(),
        "- Missing, stale, duplicate, invalid, or evidence-less classifications fail closed.".to_owned(),
        "- `parity` means cited differential/public evidence exists; it is not inferred".to_owned(),
        "  from a similarly named Rust module. `open` rows remain release blockers.".to_owned(),
        "- `design` rows are authorized only by the exhaustive difference list;".to_owned(),
        "  `out-of-scope` rows cite the product boundary rather than pretending parity.".to_owned(),
        String::new(),
        format!(
            "Files: **{}** ({}); public export rows: **{}**; source files with no public/bin reachability: **{}**.",
            files.len(),
            package_summary.join(", "),
            public.len(),
            unreachable
        ),
        String::new(),
        "| Classification | Source files | Public exports |".to_owned(),
        "|---|---:|---:|".to_owned(),
    ];
    for status in ["parity", "open", "design", "out-of-scope"] {
        lines.push(format!(
            "| `{status}` | {} | {} |",
            status_counts.get(status).unwrap_or(&0),
            public_status.get(status).unwrap_or(&0)
        ));
    }
    lines.push(String::new());
    lines.push("> This audit intentionally does **not** close PLAN 11: open prerequisite rows".to_owned());
    lines.push("> remain in PLAN 8/9/10, and PLAN 11 retains the final live side-by-side gate.".to_owned());
    lines.push(String::new());
    lines.push("## Exhaustive DESIGN differences".to_owned());
    lines.push(String::new());
    lines.push("| Difference | Audit status | Owner | Width finding |".to_owned());
    lines.push("|---:|---|---|---|".to_owned());
    let differences = as_arr(manifest.get("differences").ok_or_else(|| AuditError::Message("no differences".into()))?, "differences")?;
    for row in differences {
        let map = as_obj(row, "difference")?;
        let number = map.get("number").and_then(|v| v.as_i64()).unwrap_or(0);
        let status = str_field(map, "status", "difference")?;
        let owner = str_field(map, "owner", "difference")?;
        let finding = esc(str_field(map, "finding", "difference")?);
        lines.push(format!("| {number} | `{status}` | {owner} | {finding} |"));
    }
    lines.push(String::new());
    lines.push("## Classified source groups".to_owned());
    lines.push(String::new());
    lines.push("| Group | Status | Owner | Rows | Evidence / finding |".to_owned());
    lines.push("|---|---|---|---:|---|".to_owned());
    let groups = as_arr(manifest.get("groups").ok_or_else(|| AuditError::Message("no groups".into()))?, "groups")?;
    for group in groups {
        let gmap = as_obj(group, "group")?;
        let group_id = str_field(gmap, "id", "group")?;
        let status = str_field(gmap, "status", "group")?;
        let owner = str_field(gmap, "owner", "group")?;
        let rationale = esc(str_field(gmap, "rationale", "group")?);
        let empty_evidence = Vec::new();
        let evidence = gmap.get("evidence").and_then(|v| v.as_array()).unwrap_or(&empty_evidence);
        let evidence_strs: Vec<String> = evidence
            .iter()
            .filter_map(|e| e.get("path").and_then(|v| v.as_str()))
            .map(|p| format!("`{p}`"))
            .collect();
        let refs = gmap.get("refs").and_then(|v| v.as_array()).map(|a| a.len()).unwrap_or(0);
        let evidence_line = evidence_strs.join(", ");
        lines.push(format!(
            "| `{group_id}` | `{status}` | {owner} | {refs} | {rationale} Evidence: {evidence_line}. |"
        ));
    }
    lines.push(String::new());
    lines.push("## Open gaps".to_owned());
    lines.push(String::new());
    let open_groups: Vec<&serde_json::Value> = groups.iter().filter(|g| g.get("status").and_then(|v| v.as_str()) == Some("open")).collect();
    if open_groups.is_empty() {
        lines.push("None.".to_owned());
    }
    for group in &open_groups {
        let gmap = as_obj(group, "group")?;
        let group_id = str_field(gmap, "id", "group")?;
        let owner = str_field(gmap, "owner", "group")?;
        let rationale = str_field(gmap, "rationale", "group")?;
        let empty_refs = Vec::new();
        let refs = gmap.get("refs").and_then(|v| v.as_array()).unwrap_or(&empty_refs);
        lines.extend(vec![
            format!("### {owner} — `{group_id}`"),
            String::new(),
            rationale.to_owned(),
            String::new(),
            format!("Reference rows ({}):", refs.len()),
            String::new(),
        ]);
        lines.extend(refs.iter().filter_map(|r| r.as_str()).map(|p: &str| format!("- `{p}`")));
        lines.push(String::new());
    }
    lines.push("## Public entrypoints".to_owned());
    lines.push(String::new());
    let mut by_root: BTreeMap<&str, Vec<&serde_json::Value>> = BTreeMap::new();
    for row in public {
        let map = as_obj(row, "public")?;
        let r = str_field(map, "root", "public")?;
        by_root.entry(r).or_default().push(row);
    }
    let roots = as_arr(reference.get("roots").ok_or_else(|| AuditError::Message("no roots".into()))?, "roots")?;
    for root_row in roots {
        let rmap = as_obj(root_row, "root")?;
        if rmap.get("kind").and_then(|v| v.as_str()) != Some("public") {
            continue;
        }
        let label = str_field(rmap, "label", "root")?;
        let path = str_field(rmap, "path", "root")?;
        let rows = by_root.get(label).cloned().unwrap_or_default();
        let count = rows.len();
        let mut statuses: BTreeMap<&str, usize> = BTreeMap::new();
        for row in &rows {
            let map = as_obj(row, "public")?;
            let origin = str_field(map, "origin", "public")?;
            let status = classified.get(origin).and_then(|c| c.get("status")).and_then(|v| v.as_str()).unwrap_or("");
            *statuses.entry(status).or_insert(0) += 1;
        }
        let summary: Vec<String> = statuses.iter().map(|(k, v)| format!("{k}={v}")).collect();
        lines.push(format!("- `{label}` → `{path}`: {count} exports ({})", summary.join(", ")));
    }
    lines.push(String::new());
    Ok(lines.join("\n"))
}

/// Extend a manifest `groups`/`differences`-style value with a deep copy,
/// used by the negative controls (mirrors Python `copy.deepcopy`).
fn deep_clone(v: &serde_json::Value) -> serde_json::Value {
    v.clone()
}

fn expect_failure(root: &Path, reference: &serde_json::Value, manifest: &serde_json::Value, needle: &str) -> Result<()> {
    let err = match validate(root, reference, manifest, false) {
        Ok(_) => return fail(format!("self-test expected failure containing {needle:?}")),
        Err(e) => e.to_string(),
    };
    if !err.contains(needle) {
        return fail(format!("self-test expected {needle:?}, got {err}"));
    }
    Ok(())
}

/// Offline fail-closed negative controls (mirrors the Python `self_test`).
pub fn run_selftest(root: &Path, reference: &serde_json::Value, manifest: &serde_json::Value) -> Result<()> {
    validate(root, reference, manifest, true)
        .map_err(|e| AuditError::Message(format!("checked manifest must validate: {e}")))?;

    // missing classification (`missing=`)
    {
        let mut m = deep_clone(manifest);
        let groups = m["groups"].as_array_mut().ok_or_else(|| AuditError::Message("groups".into()))?;
        let refs = groups[0]["refs"].as_array_mut().ok_or_else(|| AuditError::Message("refs".into()))?;
        refs.pop();
        expect_failure(root, reference, &m, "missing=")?;
    }

    // duplicate classification (`classified by both`)
    {
        let mut m = deep_clone(manifest);
        let dup = m["groups"][0]["refs"][0].clone();
        m["groups"][1]["refs"].as_array_mut().ok_or_else(|| AuditError::Message("refs".into()))?.push(dup);
        expect_failure(root, reference, &m, "classified by both")?;
    }

    // stale classification (`stale=`)
    {
        let mut m = deep_clone(manifest);
        let mut added = m["groups"][0]["refs"].clone();
        added.as_array_mut().ok_or_else(|| AuditError::Message("refs".into()))?.push(serde_json::json!("packages/coding-agent/src/not-real.ts"));
        m["groups"][0]["refs"] = added;
        expect_failure(root, reference, &m, "stale=")?;
    }

    // invalid status
    {
        let mut m = deep_clone(manifest);
        m["groups"][0]["status"] = serde_json::json!("approximately-done");
        expect_failure(root, reference, &m, "invalid status")?;
    }

    // missing DESIGN difference
    {
        let mut m = deep_clone(manifest);
        m["differences"].as_array_mut().ok_or_else(|| AuditError::Message("differences".into()))?.pop();
        expect_failure(root, reference, &m, "classify each DESIGN difference")?;
    }

    // stale reference digest
    {
        let mut m = deep_clone(manifest);
        m["reference_sha256"] = serde_json::json!("0".repeat(64));
        expect_failure(root, reference, &m, "reference_sha256 is stale")?;
    }
    Ok(())
}

/// Run the final-parity-audit workflow against the repo at `root`.
pub fn run(root: &Path, check: bool, self_test: bool) -> Result<()> {
    let reference = load_json(&root.join("tests/final-parity-audit/reference.json"))?;
    let manifest = load_json(&root.join("tests/final-parity-audit/manifest.json"))?;
    if self_test {
        run_selftest(root, &reference, &manifest)?;
        println!("final parity audit fail-closed self-tests passed");
        return Ok(());
    }
    let classified = validate(root, &reference, &manifest, true)?;
    let generated = render(root, &reference, &manifest, &classified)?;
    let output = root.join("FINAL_PARITY_AUDIT.md");
    if check {
        let current = std::fs::read_to_string(&output).unwrap_or_default();
        if current != generated {
            eprintln!("{} is stale; regenerate", output.display());
            return fail("FINAL_PARITY_AUDIT.md is stale");
        }
        let package_counts: Vec<String> = {
            let files = reference.get("files").and_then(|v| v.as_array()).map(|a| a.len()).unwrap_or(0);
            let public = reference.get("public_exports").and_then(|v| v.as_array()).map(|a| a.len()).unwrap_or(0);
            vec![format!("{files}"), format!("{public}")]
        };
        println!(
            "final parity audit is current: {} files, {} public exports",
            package_counts[0], package_counts[1]
        );
    } else {
        std::fs::write(&output, &generated)?;
        println!("wrote FINAL_PARITY_AUDIT.md");
    }
    Ok(())
}
