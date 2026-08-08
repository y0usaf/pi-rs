#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! Normalize Pi's generated model catalog into pi-rs's reviewed data snapshot
//! (Rust port of the deleted `scripts/update-model-catalog.ts`; PLAN A.3 —
//! the model-catalog workflow is owned by Rust).
//!
//! Network/source discovery lives here, never in the runtime registry.

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

use serde_json::{Map, Value};
use sha2::{Digest, Sha256};

const NPM_REGISTRY: &str = "https://registry.npmjs.org";
const NPM_PACKAGE: &str = "@earendil-works/pi-ai";
const DEFAULT_NPM_VERSION: &str = "latest";
const DEFAULT_SOURCE_PATH: &str = "dist/providers/data/*.json";
const DEFAULT_OUTPUT: &str = "crates/pi-rs-ai/data/models.json";
const DEFAULT_PROVENANCE: &str = "crates/pi-rs-ai/data/models.provenance.json";
const DEFAULT_OVERRIDES: &str = "scripts/model-catalog-overrides.json";

fn accepted_apis() -> HashSet<&'static str> {
    [
        "openai-completions",
        "mistral-conversations",
        "openai-responses",
        "azure-openai-responses",
        "openai-codex-responses",
        "anthropic-messages",
        "bedrock-converse-stream",
        "google-generative-ai",
        "google-vertex",
    ]
    .into_iter()
    .collect()
}

fn model_keys() -> HashSet<&'static str> {
    [
        "id", "name", "api", "provider", "baseUrl", "reasoning", "thinkingLevelMap", "input",
        "cost", "contextWindow", "maxTokens", "headers", "compat",
    ]
    .into_iter()
    .collect()
}

fn cost_rate_keys() -> HashSet<&'static str> {
    ["input", "output", "cacheRead", "cacheWrite"].into_iter().collect()
}

fn cost_keys() -> HashSet<&'static str> {
    let mut keys = cost_rate_keys();
    keys.insert("tiers");
    keys
}

fn cost_tier_keys() -> HashSet<&'static str> {
    let mut keys = cost_rate_keys();
    keys.insert("inputTokensAbove");
    keys
}

fn thinking_keys() -> HashSet<&'static str> {
    ["off", "minimal", "low", "medium", "high", "xhigh", "max"]
        .into_iter()
        .collect()
}

// ---------------------------------------------------------------------
// CLI
// ---------------------------------------------------------------------

struct Options {
    source: Option<String>,
    version: String,
    revision: Option<String>,
    source_path: String,
    output: String,
    provenance: String,
    overrides: String,
    summary_output: Option<String>,
}

fn usage() -> ! {
    eprintln!(
        "usage: update-model-catalog [options]\n\n  \
         --source PATH          local generated .ts file, extracted npm package,\n  \
                                or provider data dir (offline)\n  \
         --version V            npm package version ({DEFAULT_NPM_VERSION})\n  \
         --revision REV         provenance revision label (npm mode defaults to the resolved version)\n  \
         --source-path PATH     provenance path label ({DEFAULT_SOURCE_PATH})\n  \
         --output PATH          normalized catalog output ({DEFAULT_OUTPUT})\n  \
         --provenance PATH      provenance output ({DEFAULT_PROVENANCE})\n  \
         --overrides PATH       reviewed metadata overrides ({DEFAULT_OVERRIDES})\n  \
         --summary-output PATH  write PR-ready inventory summary"
    );
    std::process::exit(2);
}

fn parse_args(args: &[String]) -> Options {
    let mut options = Options {
        source: None,
        version: DEFAULT_NPM_VERSION.to_string(),
        revision: None,
        source_path: DEFAULT_SOURCE_PATH.to_string(),
        output: DEFAULT_OUTPUT.to_string(),
        provenance: DEFAULT_PROVENANCE.to_string(),
        overrides: DEFAULT_OVERRIDES.to_string(),
        summary_output: None,
    };
    let mut index = 0;
    while index < args.len() {
        let arg = &args[index];
        if arg == "--help" || arg == "-h" {
            usage();
        }
        let value = args.get(index + 1).cloned();
        match arg.as_str() {
            "--source" => options.source = value,
            "--version" => options.version = match value { Some(v) => v, None => usage() },
            "--revision" => options.revision = value,
            "--source-path" => options.source_path = match value { Some(v) => v, None => usage() },
            "--output" => options.output = match value { Some(v) => v, None => usage() },
            "--provenance" => options.provenance = match value { Some(v) => v, None => usage() },
            "--overrides" => options.overrides = match value { Some(v) => v, None => usage() },
            "--summary-output" => options.summary_output = value,
            other => {
                eprintln!("unknown option: {other}");
                usage();
            }
        }
        index += 2;
    }
    options
}

// ---------------------------------------------------------------------
// Validation helpers
// ---------------------------------------------------------------------

fn fail<T>(message: impl AsRef<str>) -> Result<T, String> {
    Err(format!("model catalog: {}", message.as_ref()))
}

fn object<'a>(value: &'a Value, where_: &str) -> Result<&'a Map<String, Value>, String> {
    match value {
        Value::Object(map) => Ok(map),
        _ => fail(format!("{where_} must be an object")),
    }
}

fn string<'a>(value: &'a Value, where_: &str) -> Result<&'a str, String> {
    match value {
        Value::String(text) if !text.is_empty() => Ok(text),
        _ => fail(format!("{where_} must be a non-empty string")),
    }
}

fn finite_number(value: &Value, where_: &str) -> Result<f64, String> {
    match value {
        Value::Number(number) => {
            let float = number.as_f64().unwrap_or(f64::NAN);
            if float.is_finite() {
                Ok(float)
            } else {
                fail(format!("{where_} must be a finite number"))
            }
        }
        _ => fail(format!("{where_} must be a finite number")),
    }
}

fn reject_unknown_keys(
    value: &Map<String, Value>,
    accepted: &HashSet<&str>,
    where_: &str,
) -> Result<(), String> {
    let unknown: Vec<&String> = value
        .keys()
        .filter(|key| !accepted.contains(key.as_str()))
        .collect();
    if unknown.is_empty() {
        Ok(())
    } else {
        fail(format!(
            "{where_} has unknown field(s): {}",
            unknown
                .iter()
                .map(|key| key.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        ))
    }
}

fn validate_model(model_value: &Value, provider_key: &str, model_key: &str) -> Result<Value, String> {
    let where_ = format!("{provider_key}/{model_key}");
    let model = object(model_value, &where_)?;
    reject_unknown_keys(model, &model_keys(), &where_)?;
    if string(&model["id"], &format!("{where_}.id"))? != model_key {
        return fail(format!("{where_}.id does not match its catalog key"));
    }
    string(&model["name"], &format!("{where_}.name"))?;
    let api = string(&model["api"], &format!("{where_}.api"))?;
    if !accepted_apis().contains(api) {
        return fail(format!("{where_}.api uses unsupported wire protocol {api}"));
    }
    if string(&model["provider"], &format!("{where_}.provider"))? != provider_key {
        return fail(format!("{where_}.provider does not match its catalog key"));
    }
    if !model["baseUrl"].is_string() {
        return fail(format!("{where_}.baseUrl must be a string"));
    }
    if !model["reasoning"].is_boolean() {
        return fail(format!("{where_}.reasoning must be boolean"));
    }
    let input = model
        .get("input")
        .and_then(Value::as_array)
        .filter(|array| !array.is_empty())
        .ok_or_else(|| format!("{where_}.input must be a non-empty array"))?;
    for modality in input {
        let text = modality.as_str().unwrap_or_default();
        if text != "text" && text != "image" {
            return fail(format!("{where_}.input has unknown modality {text}"));
        }
    }
    let cost = object(&model["cost"], &format!("{where_}.cost"))?;
    reject_unknown_keys(cost, &cost_keys(), &format!("{where_}.cost"))?;
    for key in cost_rate_keys() {
        finite_number(&cost[key], &format!("{where_}.cost.{key}"))?;
    }
    if let Some(tiers) = cost.get("tiers") {
        let tiers = tiers
            .as_array()
            .ok_or_else(|| format!("{where_}.cost.tiers must be an array"))?;
        for (index, raw_tier) in tiers.iter().enumerate() {
            let tier_where = format!("{where_}.cost.tiers[{index}]");
            let tier = object(raw_tier, &tier_where)?;
            reject_unknown_keys(tier, &cost_tier_keys(), &tier_where)?;
            for key in cost_rate_keys() {
                finite_number(&tier[key], &format!("{tier_where}.{key}"))?;
            }
            let threshold =
                finite_number(&tier["inputTokensAbove"], &format!("{tier_where}.inputTokensAbove"))?;
            if !(threshold.is_finite() && threshold >= 0.0 && threshold.fract() == 0.0) {
                return fail(format!(
                    "{tier_where}.inputTokensAbove must be a non-negative safe integer"
                ));
            }
        }
    }
    for key in ["contextWindow", "maxTokens"] {
        let value = finite_number(&model[key], &format!("{where_}.{key}"))?;
        if !(value.is_finite() && value > 0.0 && value.fract() == 0.0) {
            return fail(format!("{where_}.{key} must be a positive safe integer"));
        }
    }
    if let Some(map) = model.get("thinkingLevelMap") {
        let map = object(map, &format!("{where_}.thinkingLevelMap"))?;
        reject_unknown_keys(map, &thinking_keys(), &format!("{where_}.thinkingLevelMap"))?;
        for (key, value) in map {
            if !value.is_null() && !value.is_string() {
                return fail(format!("{where_}.thinkingLevelMap.{key} must be string or null"));
            }
        }
    }
    if let Some(headers) = model.get("headers") {
        for (key, value) in object(headers, &format!("{where_}.headers"))? {
            if key.is_empty() || !value.is_string() {
                return fail(format!("{where_}.headers must map non-empty names to strings"));
            }
        }
    }
    if let Some(compat) = model.get("compat") {
        object(compat, &format!("{where_}.compat"))?;
    }
    Ok(model_value.clone())
}

fn normalize(raw: &Value) -> Result<Vec<(String, Vec<Value>)>, String> {
    let providers = object(raw, "MODELS")?;
    let mut catalog: Vec<(String, Vec<Value>)> = Vec::new();
    let mut provider_ids = HashSet::new();
    for (provider, models_value) in providers {
        if provider.is_empty() {
            return fail("provider id must be a non-empty string");
        }
        if !provider_ids.insert(provider.clone()) {
            return fail(format!("duplicate provider {provider}"));
        }
        let models = object(models_value, provider)?;
        let mut ids = HashSet::new();
        let mut normalized: Vec<Value> = Vec::new();
        for (id, model) in models {
            if !ids.insert(id.clone()) {
                return fail(format!("duplicate model {provider}/{id}"));
            }
            normalized.push(validate_model(model, provider, id)?);
        }
        catalog.push((provider.clone(), normalized));
    }
    Ok(catalog)
}

fn validate_catalog(catalog: &[(String, Vec<Value>)]) -> Result<(), String> {
    let mut providers = HashSet::new();
    for (provider, models) in catalog {
        if !providers.insert(provider.clone()) {
            return fail(format!("duplicate provider {provider}"));
        }
        let mut ids = HashSet::new();
        for model in models {
            let id = string(&model["id"], &format!("{provider} model id"))?.to_string();
            if !ids.insert(id.clone()) {
                return fail(format!("duplicate model {provider}/{id}"));
            }
            validate_model(model, provider, &id)?;
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------
// Overrides
// ---------------------------------------------------------------------

#[derive(Default)]
struct Override {
    provider: String,
    model: String,
    set: Option<Map<String, Value>>,
    remove: Vec<String>,
}

fn load_overrides(path: &Path) -> Result<(Vec<Override>, String), String> {
    let bytes = fs::read_to_string(path)
        .map_err(|error| format!("model catalog: cannot read {}: {error}", path.display()))?;
    let root: Value = serde_json::from_str(&bytes)
        .map_err(|error| format!("model catalog: overrides parse error: {error}"))?;
    let root = object(&root, "overrides")?;
    reject_unknown_keys(root, &HashSet::from(["schemaVersion", "overrides"]), "overrides")?;
    if root["schemaVersion"] != 1 {
        return fail("overrides must use schemaVersion 1 and an overrides array");
    }
    let rows = root["overrides"].as_array().ok_or_else(|| {
        "model catalog: overrides must use schemaVersion 1 and an overrides array".to_string()
    })?;
    let mut overrides = Vec::new();
    for (index, raw) in rows.iter().enumerate() {
        let item = object(raw, &format!("overrides[{index}]"))?;
        reject_unknown_keys(
            item,
            &HashSet::from(["provider", "model", "reason", "set", "remove"]),
            &format!("overrides[{index}]"),
        )?;
        // reason is human review metadata; validate its type but do not carry it.
        string(&item["reason"], &format!("overrides[{index}].reason"))?;
        let mut parsed = Override {
            provider: string(&item["provider"], &format!("overrides[{index}].provider"))?.to_string(),
            model: string(&item["model"], &format!("overrides[{index}].model"))?.to_string(),
            ..Default::default()
        };
        if let Some(set) = item.get("set") {
            parsed.set = Some(object(set, &format!("overrides[{index}].set"))?.clone());
        }
        if let Some(remove) = item.get("remove") {
            let remove = remove
                .as_array()
                .filter(|array| array.iter().all(Value::is_string))
                .ok_or_else(|| format!("overrides[{index}].remove must be an array of field names"))?;
            parsed.remove = remove.iter().filter_map(Value::as_str).map(str::to_string).collect();
        }
        overrides.push(parsed);
    }
    Ok((overrides, bytes))
}

fn apply_overrides(
    catalog: &mut [(String, Vec<Value>)],
    overrides: &[Override],
) -> Result<(), String> {
    let mut seen = HashSet::new();
    for over in overrides {
        let target = format!("{}/{}", over.provider, over.model);
        if !seen.insert(target.clone()) {
            return fail(format!("multiple overrides target {target}"));
        }
        let Some(provider) = catalog.iter_mut().find(|(provider, _)| *provider == over.provider)
        else {
            return fail(format!("override target does not exist: {target}"));
        };
        let Some(model) = provider.1.iter_mut().find(|row| row["id"] == over.model) else {
            return fail(format!("override target does not exist: {target}"));
        };
        let model_obj = model
            .as_object_mut()
            .ok_or_else(|| format!("model catalog: override target is not an object: {target}"))?;
        if let Some(set) = &over.set {
            for (key, value) in set {
                model_obj.insert(key.clone(), value.clone());
            }
        }
        for key in &over.remove {
            model_obj.remove(key);
        }
    }
    validate_catalog(catalog)
}

// ---------------------------------------------------------------------
// Hashing + inventory
// ---------------------------------------------------------------------

fn sha256(value: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(value.as_bytes());
    hex(&hasher.finalize())
}

fn sha256_bytes(value: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(value);
    hex(&hasher.finalize())
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[derive(Default)]
struct Inventory {
    providers: usize,
    models: usize,
    by_provider: Vec<(String, usize)>,
    apis: Vec<(String, usize)>,
}

fn inventory(catalog: &[(String, Vec<Value>)]) -> Inventory {
    let mut by_provider: Vec<(String, usize)> = Vec::new();
    let mut apis: Vec<(String, usize)> = Vec::new();
    let mut models = 0;
    for (provider, rows) in catalog {
        by_provider.push((provider.clone(), rows.len()));
        models += rows.len();
        for row in rows {
            if let Some(api) = row["api"].as_str() {
                if let Some(entry) = apis.iter_mut().find(|(key, _)| key == api) {
                    entry.1 += 1;
                } else {
                    apis.push((api.to_string(), 1));
                }
            }
        }
    }
    Inventory {
        providers: catalog.len(),
        models,
        by_provider,
        apis,
    }
}

fn old_inventory(path: &Path) -> Option<Inventory> {
    let bytes = fs::read(path).ok()?;
    let value: Value = serde_json::from_slice(&bytes).ok()?;
    let rows = value.as_array()?;
    let catalog: Vec<(String, Vec<Value>)> = rows
        .iter()
        .filter_map(|row| {
            let provider = row.get("provider")?.as_str()?.to_string();
            let models = row.get("models")?.as_array()?.clone();
            Some((provider, models))
        })
        .collect();
    Some(inventory(&catalog))
}

// ---------------------------------------------------------------------
// Acquisition
// ---------------------------------------------------------------------

struct Acquired {
    file: Option<PathBuf>,
    data_dir: Option<PathBuf>,
    revision: String,
    tarball_sha256: Option<String>,
    _cleanup: Option<tempfile::TempDir>,
}

fn acquire(options: &Options) -> Result<Acquired, String> {
    if let Some(source) = &options.source {
        let source = PathBuf::from(source);
        if !source.exists() {
            return fail(format!("source does not exist: {}", source.display()));
        }
        let extension = source
            .extension()
            .and_then(|ext| ext.to_str())
            .unwrap_or_default();
        if extension == "ts" || extension == "js" {
            return Ok(Acquired {
                file: Some(source),
                revision: options.revision.clone().unwrap_or_else(|| "local-fixture".to_string()),
                data_dir: None,
                tarball_sha256: None,
                _cleanup: None,
            });
        }
        let as_package = source.join("dist").join("providers").join("data");
        let as_dist = source.join("providers").join("data");
        let data_dir = if as_package.exists() {
            as_package
        } else if as_dist.exists() {
            as_dist
        } else {
            source.clone()
        };
        return Ok(Acquired {
            file: None,
            data_dir: Some(data_dir),
            revision: options.revision.clone().unwrap_or_else(|| "local-fixture".to_string()),
            tarball_sha256: None,
            _cleanup: None,
        });
    }

    let temp = tempfile::tempdir().map_err(|error| format!("model catalog: tempdir failed: {error}"))?;
    match acquire_npm(options, temp.path()) {
        Ok(acquired) => Ok(Acquired {
            _cleanup: Some(temp),
            ..acquired
        }),
        Err(error) => Err(error),
    }
}

fn acquire_npm(options: &Options, temp: &Path) -> Result<Acquired, String> {
    let registry_url = format!("{NPM_REGISTRY}/{NPM_PACKAGE}/{}", urlencode(&options.version));
    let response = reqwest::blocking::get(&registry_url).map_err(|error| {
        format!("model catalog: npm registry request failed: {registry_url} ({error})")
    })?;
    if !response.status().is_success() {
        return fail(format!(
            "npm registry request failed: {registry_url} ({})",
            response.status()
        ));
    }
    let meta: Value = response
        .json()
        .map_err(|error| format!("model catalog: npm metadata parse failed: {error}"))?;
    let meta = object(&meta, "npm registry metadata")?;
    let resolved_version = string(&meta["version"], "npm version")?.to_string();
    let dist = object(&meta["dist"], "npm dist")?;
    let tarball_url = string(&dist["tarball"], "npm dist.tarball")?.to_string();
    let tarball_response = reqwest::blocking::get(&tarball_url).map_err(|error| {
        format!("model catalog: tarball download failed: {tarball_url} ({error})")
    })?;
    if !tarball_response.status().is_success() {
        return fail(format!(
            "tarball download failed: {tarball_url} ({})",
            tarball_response.status()
        ));
    }
    let bytes = tarball_response
        .bytes()
        .map_err(|error| format!("model catalog: tarball read failed: {error}"))?;
    let tarball_path = temp.join("package.tgz");
    fs::write(&tarball_path, &bytes)
        .map_err(|error| format!("model catalog: tarball write failed: {error}"))?;
    let tarball_sha256 = sha256_bytes(&bytes);
    let status = Command::new("tar")
        .args(["xzf"])
        .arg(&tarball_path)
        .arg("-C")
        .arg(temp)
        .status()
        .map_err(|error| format!("model catalog: tar failed: {error}"))?;
    if !status.success() {
        return fail("tar extraction failed");
    }
    let data_dir = temp.join("package").join("dist").join("providers").join("data");
    if !data_dir.exists() {
        return fail(format!("tarball has no {DEFAULT_SOURCE_PATH}: {tarball_url}"));
    }
    Ok(Acquired {
        file: None,
        data_dir: Some(data_dir),
        revision: resolved_version,
        tarball_sha256: Some(tarball_sha256),
        _cleanup: None,
    })
}

fn urlencode(value: &str) -> String {
    // npm package versions are conservative (semver / "latest"); the TS
    // original used encodeURIComponent.
    value
        .chars()
        .flat_map(|ch| match ch {
            'A'..='Z' | 'a'..='z' | '0'..='9' | '-' | '_' | '.' | '~' => vec![ch],
            other => format!("%{:02X}", other as u32).chars().collect(),
        })
        .collect()
}

// ---------------------------------------------------------------------
// Source import
// ---------------------------------------------------------------------

/// Parse a generated `.ts`/`.js` fixture module (`export const MODELS = …`)
/// into the catalog object. The fixtures are inert data literals (no
/// execution); this is the Rust equivalent of the deleted TS `import()`.
fn import_models(file: &Path) -> Result<Value, String> {
    let text = fs::read_to_string(file)
        .map_err(|error| format!("model catalog: cannot read {}: {error}", file.display()))?;
    let start = text
        .find('{')
        .ok_or_else(|| format!("model catalog: {} does not export MODELS", file.display()))?;
    let mut depth = 0usize;
    let mut end = None;
    for (offset, ch) in text[start..].char_indices() {
        match ch {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    end = Some(start + offset);
                    break;
                }
            }
            _ => {}
        }
    }
    let end = end.ok_or_else(|| {
        format!("model catalog: {} has an unterminated MODELS literal", file.display())
    })?;
    let body = &text[start..=end];
    let json = ts_object_literal_to_json(body)?;
    let value: Value = serde_json::from_str(&json).map_err(|error| {
        format!(
            "model catalog: {} is not a JSON-convertible MODELS literal: {error}",
            file.display()
        )
    })?;
    let map = object(&value, "MODELS")?;
    Ok(Value::Object(map.clone()))
}

/// Convert a JS object literal (unquoted keys, trailing commas, `as const`
/// already outside the extracted body) to strict JSON. String-aware: nothing
/// inside quoted strings is touched.
fn ts_object_literal_to_json(body: &str) -> Result<String, String> {
    let mut out = String::with_capacity(body.len());
    let mut chars = body.char_indices().peekable();
    let mut in_string = false;
    let mut string_char = '\0';
    while let Some((_, ch)) = chars.next() {
        if in_string {
            out.push(ch);
            if ch == '\\' {
                if let Some((_, next)) = chars.next() {
                    out.push(next);
                }
            } else if ch == string_char {
                in_string = false;
            }
            continue;
        }
        match ch {
            '"' | '\'' => {
                in_string = true;
                string_char = ch;
                out.push('"');
            }
            // Quote a bare identifier key: preceded by `{`/`,` + optional
            // whitespace, followed by `:`.
            ',' | '{' => {
                out.push(ch);
                let mut ahead = chars.clone();
                while let Some((_, c)) = ahead.peek().copied() {
                    if c.is_whitespace() {
                        ahead.next();
                    } else {
                        break;
                    }
                }
                let mut ident = String::new();
                while let Some((_, c)) = ahead.peek().copied() {
                    if c.is_ascii_alphanumeric() || c == '_' || c == '$' {
                        ident.push(c);
                        ahead.next();
                    } else {
                        break;
                    }
                }
                if !ident.is_empty()
                    && let Some((_, c)) = ahead.peek().copied()
                    && c == ':'
                {
                    out.push('"');
                    out.push_str(&ident);
                    out.push('"');
                    out.push(':');
                    // Consume the colon here; otherwise the fallthrough
                    // branch would push it a second time (id:: bug).
                    chars = ahead;
                    chars.next();
                    continue;
                }
            }
            '}' | ']' => {
                // Drop a trailing comma immediately before a closer.
                let trimmed = out.trim_end();
                if trimmed.ends_with(',') {
                    out.truncate(trimmed.len() - 1);
                }
                out.push(ch);
            }
            _ => out.push(ch),
        }
    }
    Ok(out)
}

fn import_catalog_data(data_dir: &Path) -> Result<Value, String> {
    let mut files: Vec<PathBuf> = fs::read_dir(data_dir)
        .map_err(|error| format!("model catalog: cannot read {}: {error}", data_dir.display()))?
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .filter(|path| {
            path.extension().and_then(|ext| ext.to_str()) == Some("json")
                && !path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.starts_with('.'))
        })
        .collect();
    files.sort();
    if files.is_empty() {
        return fail(format!("no provider data files in {}", data_dir.display()));
    }
    let mut raw = Map::new();
    for file in files {
        let provider = file
            .file_stem()
            .and_then(|stem| stem.to_str())
            .ok_or_else(|| format!("model catalog: invalid provider file name: {}", file.display()))?
            .to_string();
        let bytes = fs::read(&file)
            .map_err(|error| format!("model catalog: cannot read {}: {error}", file.display()))?;
        let groups: Value = serde_json::from_slice(&bytes)
            .map_err(|error| format!("model catalog: {} parse error: {error}", file.display()))?;
        let groups = object(&groups, &provider)?;
        let mut merged = Map::new();
        for (api, models) in groups {
            if !accepted_apis().contains(api.as_str()) {
                return fail(format!("{provider} uses unsupported wire protocol {api}"));
            }
            for (id, model) in object(models, &format!("{provider}.{api}"))? {
                if merged.contains_key(id) {
                    return fail(format!("duplicate model {provider}/{id} across API groups"));
                }
                merged.insert(id.clone(), model.clone());
            }
        }
        raw.insert(provider, Value::Object(merged));
    }
    Ok(Value::Object(raw))
}

// ---------------------------------------------------------------------
// Output formatting
// ---------------------------------------------------------------------

/// `JSON.stringify(value, null, "\t")` equivalent — tab-indented JSON
/// preserving insertion order, so regeneration is byte-stable against the
/// checked files produced by the deleted TS tool.
fn tab_pretty(value: &Value) -> Result<String, String> {
    let pretty = serde_json::to_string_pretty(value)
        .map_err(|error| format!("model catalog: serialize failed: {error}"))?;
    let lines = pretty
        .lines()
        .map(|line| {
            let leading = line.len() - line.trim_start().len();
            let indent = "\t".repeat(leading / 2);
            format!("{indent}{}", line.trim_start())
        })
        .collect::<Vec<_>>()
        .join("\n");
    Ok(lines)
}

// ---------------------------------------------------------------------
// Summary + main
// ---------------------------------------------------------------------

fn summary(
    revision: &str,
    before: Option<&Inventory>,
    after: &Inventory,
    provenance_hash: &str,
) -> String {
    let provider_delta = before
        .map_or(after.providers as i64, |before| after.providers as i64 - before.providers as i64);
    let model_delta = before
        .map_or(after.models as i64, |before| after.models as i64 - before.models as i64);
    let signed = |value: i64| if value >= 0 { format!("+{value}") } else { format!("{value}") };
    let apis = after
        .apis
        .iter()
        .map(|(api, count)| format!("{api}={count}"))
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        "## Model catalog update\n\n- source revision: `{revision}`\n- source catalog SHA-256: `{provenance_hash}`\n- providers: {} ({})\n- models: {} ({})\n- APIs: {apis}\n\nGenerated by `nix run .#update-model-catalog`; schema, duplicate IDs, protocol vocabulary, typed Rust round-trip, protocol replay, and flake checks gate merge.\n",
        after.providers,
        signed(provider_delta),
        after.models,
        signed(model_delta),
    )
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let options = parse_args(&args);
    match run(&options) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error}");
            ExitCode::FAILURE
        }
    }
}

fn run(options: &Options) -> Result<(), String> {
    let acquired = acquire(options)?;
    let raw = match (&acquired.data_dir, &acquired.file) {
        (Some(data_dir), _) => import_catalog_data(data_dir)?,
        (_, Some(file)) => import_models(file)?,
        _ => unreachable!("acquire always produces a source"),
    };
    let source_canonical = serde_json::to_string(&raw)
        .map_err(|error| format!("model catalog: serialize failed: {error}"))?;
    let mut catalog = normalize(&raw)?;
    let (overrides, overrides_bytes) = load_overrides(Path::new(&options.overrides))?;
    apply_overrides(&mut catalog, &overrides)?;
    let output_path = Path::new(&options.output);
    let before = old_inventory(output_path);
    let entries: Vec<Value> = catalog
        .iter()
        .map(|(provider, models)| {
            serde_json::json!({ "provider": provider, "models": models })
        })
        .collect();
    let output_value = Value::Array(entries);
    let output = format!("{}\n", tab_pretty(&output_value)?);
    let after = inventory(&catalog);
    let source_hash = sha256(&source_canonical);

    let mut source = Map::new();
    source.insert(
        "repository".to_string(),
        Value::String(if let Some(source) = &options.source {
            let path = Path::new(source);
            match fs::canonicalize(path) {
                Ok(canonical) => canonical.display().to_string(),
                Err(_) => source.clone(),
            }
        } else {
            format!("{NPM_REGISTRY}/{NPM_PACKAGE}")
        }),
    );
    source.insert("revision".to_string(), Value::String(acquired.revision.clone()));
    source.insert(
        "path".to_string(),
        Value::String(
            if acquired.data_dir.is_some() {
                DEFAULT_SOURCE_PATH
            } else {
                &options.source_path
            }
            .to_string(),
        ),
    );
    source.insert("catalogSha256".to_string(), Value::String(source_hash.clone()));
    if let Some(tarball) = &acquired.tarball_sha256 {
        source.insert("tarballSha256".to_string(), Value::String(tarball.clone()));
    }

    let provenance = serde_json::json!({
        "schemaVersion": 1,
        "source": source,
        "overrides": {
            "path": options.overrides,
            "sha256": sha256(&overrides_bytes),
            "count": overrides.len(),
        },
        "outputSha256": sha256(&output),
        "inventory": {
            "providers": after.providers,
            "models": after.models,
            "byProvider": after.by_provider.iter().map(|(provider, count)| (provider.clone(), Value::from(*count))).collect::<Map<_, _>>(),
            "apis": after.apis.iter().map(|(api, count)| (api.clone(), Value::from(*count))).collect::<Map<_, _>>(),
        },
    });

    fs::write(output_path, &output)
        .map_err(|error| format!("model catalog: cannot write {}: {error}", output_path.display()))?;
    let provenance_path = Path::new(&options.provenance);
    let provenance_text = format!("{}\n", tab_pretty(&provenance)?);
    fs::write(provenance_path, &provenance_text)
        .map_err(|error| format!("model catalog: cannot write {}: {error}", provenance_path.display()))?;
    let report = summary(&acquired.revision, before.as_ref(), &after, &source_hash);
    if let Some(summary_path) = &options.summary_output {
        fs::write(summary_path, &report)
            .map_err(|error| format!("model catalog: cannot write {}: {error}", Path::new(summary_path).display()))?;
    }
    println!("{}", report.trim_end());
    Ok(())
}
