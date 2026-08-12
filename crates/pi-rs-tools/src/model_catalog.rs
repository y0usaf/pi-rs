//! Model-catalog normalization tool (A.3 Rust/Lua owner for the `model-catalog`
//! workflow).
//!
//! Port of the former `scripts/update-model-catalog.ts` (bun) workflow: read
//! Pi's generated model catalog (a TS `export const MODELS = {...}` literal or a
//! JSON file), validate every model against the reviewed schema, apply reviewed
//! metadata overrides, and emit the normalized data snapshot plus provenance and
//! an optional PR summary. This is the model-catalog workflow owner, so normal
//! `nix flake check` no longer needs a Node/Bun runtime.
//!
//! Normal (offline) checks drive this through `pi-rs-tools model-catalog` over
//! committed fixtures; opt-in regeneration fetches the pinned published catalog
//! or git revision and runs the same core.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use serde_json::{Map, Value};

use crate::jsvalue::parse_exported_const;

#[derive(Debug, thiserror::Error)]
pub enum ModelCatalogError {
    #[error("model catalog: {0}")]
    Message(String),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error(transparent)]
    Parse(#[from] crate::jsvalue::ParseError),
}

fn fail<T>(message: impl Into<String>) -> Result<T, ModelCatalogError> {
    Err(ModelCatalogError::Message(message.into()))
}

const ACCEPTED_APIS: &[&str] = &[
    "openai-completions",
    "mistral-conversations",
    "openai-responses",
    "azure-openai-responses",
    "openai-codex-responses",
    "anthropic-messages",
    "bedrock-converse-stream",
    "google-generative-ai",
    "google-vertex",
];

const MODEL_KEYS: &[&str] = &[
    "id",
    "name",
    "api",
    "provider",
    "baseUrl",
    "reasoning",
    "thinkingLevelMap",
    "input",
    "cost",
    "contextWindow",
    "maxTokens",
    "headers",
    "compat",
];

const COST_RATE_KEYS: &[&str] = &["input", "output", "cacheRead", "cacheWrite"];
const THINKING_KEYS: &[&str] = &["off", "minimal", "low", "medium", "high", "xhigh", "max"];

fn obj<'a>(value: &'a Value, where_: &str) -> Result<&'a Map<String, Value>, ModelCatalogError> {
    match value {
        Value::Object(m) => Ok(m),
        other => fail(format!("{where_} must be an object (got {other:?})")),
    }
}

fn as_str<'a>(value: &'a Value, where_: &str) -> Result<&'a str, ModelCatalogError> {
    match value {
        Value::String(s) if !s.is_empty() => Ok(s),
        other => fail(format!("{where_} must be a non-empty string (got {other:?})")),
    }
}

fn finite_number(value: &Value, where_: &str) -> Result<f64, ModelCatalogError> {
    match value {
        Value::Number(n) => n.as_f64().filter(|f| f.is_finite()).ok_or_else(|| {
            ModelCatalogError::Message(format!("{where_} must be a finite number"))
        }),
        other => fail(format!("{where_} must be a finite number (got {other:?})")),
    }
}

fn reject_unknown_keys(
    map: &Map<String, Value>,
    accepted: &[&str],
    where_: &str,
) -> Result<(), ModelCatalogError> {
    let accepted: BTreeSet<&str> = accepted.iter().copied().collect();
    let unknown: Vec<&str> = map
        .keys()
        .filter(|k| !accepted.contains(k.as_str()))
        .map(|k| k.as_str())
        .collect();
    if !unknown.is_empty() {
        return fail(format!("{where_} has unknown field(s): {}", unknown.join(", ")));
    }
    Ok(())
}

fn validate_model(
    value: &Value,
    provider_key: &str,
    model_key: &str,
) -> Result<Map<String, Value>, ModelCatalogError> {
    let where_ = format!("{provider_key}/{model_key}");
    let model = obj(value, &where_)?;
    reject_unknown_keys(model, MODEL_KEYS, &where_)?;

    if as_str(&model["id"], &format!("{where_}.id"))? != model_key {
        return fail(format!("{where_}.id does not match its catalog key"));
    }
    as_str(&model["name"], &format!("{where_}.name"))?;
    let api = as_str(&model["api"], &format!("{where_}.api"))?;
    if !ACCEPTED_APIS.contains(&api) {
        return fail(format!("{where_}.api uses unsupported wire protocol {api:?}"));
    }
    if as_str(&model["provider"], &format!("{where_}.provider"))? != provider_key {
        return fail(format!("{where_}.provider does not match its catalog key"));
    }
    if !model["baseUrl"].is_string() {
        return fail(format!("{where_}.baseUrl must be a string"));
    }
    if !model["reasoning"].is_boolean() {
        return fail(format!("{where_}.reasoning must be boolean"));
    }
    let input = match model.get("input") {
        Some(Value::Array(a)) if !a.is_empty() => a,
        other => return fail(format!("{where_}.input must be a non-empty array ({other:?})")),
    };
    for modality in input {
        if *modality != Value::String("text".into()) && *modality != Value::String("image".into())
        {
            return fail(format!("{where_}.input has unknown modality {modality:?}"));
        }
    }

    let cost_where = format!("{where_}.cost");
    let cost = obj(&model["cost"], &cost_where)?;
    let cost_keys: Vec<&str> = COST_RATE_KEYS
        .iter()
        .copied()
        .chain(std::iter::once("tiers"))
        .collect();
    reject_unknown_keys(cost, &cost_keys, &cost_where)?;
    for key in COST_RATE_KEYS {
        finite_number(&cost[*key], &format!("{cost_where}.{key}"))?;
    }
    if let Some(tiers) = cost.get("tiers") {
        let tiers = match tiers {
            Value::Array(a) => a,
            other => return fail(format!("{cost_where}.tiers must be an array ({other:?})")),
        };
        for (index, raw_tier) in tiers.iter().enumerate() {
            let tier_where = format!("{cost_where}.tiers[{index}]");
            let tier = obj(raw_tier, &tier_where)?;
            let tier_keys: Vec<&str> = COST_RATE_KEYS
                .iter()
                .copied()
                .chain(std::iter::once("inputTokensAbove"))
                .collect();
            reject_unknown_keys(tier, &tier_keys, &tier_where)?;
            for key in COST_RATE_KEYS {
                finite_number(&tier[*key], &format!("{tier_where}.{key}"))?;
            }
            let threshold =
                finite_number(&tier["inputTokensAbove"], &format!("{tier_where}.inputTokensAbove"))?;
            if threshold.fract() != 0.0 || threshold < 0.0 {
                return fail(format!(
                    "{tier_where}.inputTokensAbove must be a non-negative safe integer"
                ));
            }
        }
    }

    for key in ["contextWindow", "maxTokens"] {
        let value = finite_number(&model[key], &format!("{where_}.{key}"))?;
        if value.fract() != 0.0 || !value.is_sign_positive() {
            return fail(format!("{where_}.{key} must be a positive safe integer"));
        }
    }

    if let Some(map) = model.get("thinkingLevelMap") {
        let map = obj(map, &format!("{where_}.thinkingLevelMap"))?;
        reject_unknown_keys(map, THINKING_KEYS, &format!("{where_}.thinkingLevelMap"))?;
        for (key, value) in map {
            if !value.is_null() && !value.is_string() {
                return fail(format!("{where_}.thinkingLevelMap.{key} must be string or null"));
            }
        }
    }

    if let Some(headers) = model.get("headers") {
        for (key, value) in obj(headers, &format!("{where_}.headers"))? {
            if key.is_empty() || !value.is_string() {
                return fail(format!("{where_}.headers must map non-empty names to strings"));
            }
        }
    }

    if let Some(compat) = model.get("compat") {
        let _ = obj(compat, &format!("{where_}.compat"))?;
    }

    Ok(model.clone())
}

/// Normalize the raw `MODELS` object into the `[{ provider, models: [...] }]`
/// catalog shape.
pub fn normalize(raw: &Value) -> Result<Vec<Value>, ModelCatalogError> {
    let providers = obj(raw, "MODELS")?;
    let mut catalog: Vec<Value> = Vec::new();
    let mut provider_ids: BTreeSet<String> = BTreeSet::new();
    for (provider, models_value) in providers {
        if !provider_ids.insert(provider.clone()) {
            return fail(format!("duplicate provider {provider}"));
        }
        let models = obj(models_value, provider)?;
        let mut ids: BTreeSet<String> = BTreeSet::new();
        let mut normalized: Vec<Value> = Vec::new();
        for (id, model) in models {
            if !ids.insert(id.clone()) {
                return fail(format!("duplicate model {provider}/{id}"));
            }
            normalized.push(Value::Object(validate_model(model, provider, id)?));
        }
        let mut entry = Map::new();
        entry.insert("provider".into(), Value::String(provider.clone()));
        entry.insert("models".into(), Value::Array(normalized));
        catalog.push(Value::Object(entry));
    }
    Ok(catalog)
}

#[derive(Debug)]
struct Override {
    provider: String,
    model: String,
    set: Option<Map<String, Value>>,
    remove: Vec<String>,
}

struct LoadedOverrides {
    overrides: Vec<Override>,
    bytes: Vec<u8>,
}

fn load_overrides(path: &Path) -> Result<LoadedOverrides, ModelCatalogError> {
    let bytes = std::fs::read(path)?;
    let root_value: Value = serde_json::from_slice(&bytes)?;
    let root = obj(&root_value, "overrides")?;
    reject_unknown_keys(root, &["schemaVersion", "overrides"], "overrides")?;
    if root["schemaVersion"] != Value::Number(1.into()) {
        return fail("overrides must use schemaVersion 1");
    }
    let overrides_raw = match root.get("overrides") {
        Some(Value::Array(a)) => a,
        other => return fail(format!("overrides must be an array (got {other:?})")),
    };
    let mut overrides = Vec::new();
    for (index, raw) in overrides_raw.iter().enumerate() {
        let item_where = format!("overrides[{index}]");
        let item = obj(raw, &item_where)?;
        reject_unknown_keys(item, &["provider", "model", "reason", "set", "remove"], &item_where)?;
        let set = match item.get("set") {
            Some(s) => Some(obj(s, &format!("{item_where}.set"))?.clone()),
            None => None,
        };
        let remove = match item.get("remove") {
            Some(Value::Array(a)) => {
                for e in a {
                    if !e.is_string() {
                        return fail(format!("{item_where}.remove must be an array of field names"));
                    }
                }
                a.iter()
                    .map(|e| e.as_str().unwrap_or_default().to_owned())
                    .collect()
            }
            Some(other) => {
                return fail(format!(
                    "{item_where}.remove must be an array of field names ({other:?})"
                ))
            }
            None => Vec::new(),
        };
        overrides.push(Override {
            provider: as_str(&item["provider"], &format!("{item_where}.provider"))?.to_owned(),
            model: as_str(&item["model"], &format!("{item_where}.model"))?.to_owned(),
            set,
            remove,
        });
    }
    Ok(LoadedOverrides { overrides, bytes })
}

fn apply_overrides(catalog: &mut [Value], overrides: &[Override]) -> Result<(), ModelCatalogError> {
    let mut seen: BTreeSet<String> = BTreeSet::new();
    for override_ in overrides {
        let target = format!("{}/{}", override_.provider, override_.model);
        if !seen.insert(target.clone()) {
            return fail(format!("multiple overrides target {target}"));
        }
        let entry = catalog
            .iter_mut()
            .find(|e| e["provider"] == Value::String(override_.provider.clone()))
            .ok_or_else(|| ModelCatalogError::Message(format!("override target does not exist: {target}")))?;
        let model = entry["models"]
            .as_array_mut()
            .and_then(|arr| {
                arr.iter_mut().find(|m| m["id"] == Value::String(override_.model.clone()))
            })
            .ok_or_else(|| ModelCatalogError::Message(format!("override target does not exist: {target}")))?;
        let m = model
            .as_object_mut()
            .ok_or_else(|| ModelCatalogError::Message(format!("override target does not exist: {target}")))?;
        if let Some(set) = &override_.set {
            for (key, value) in set {
                m.insert(key.clone(), value.clone());
            }
        }
        for key in &override_.remove {
            m.remove(key);
        }
    }
    validate_catalog(catalog)
}

fn validate_catalog(catalog: &[Value]) -> Result<(), ModelCatalogError> {
    let mut providers: BTreeSet<String> = BTreeSet::new();
    for entry in catalog {
        let provider = as_str(&entry["provider"], "provider id")?;
        if !providers.insert(provider.to_owned()) {
            return fail(format!("duplicate provider {provider}"));
        }
        let models = entry["models"]
            .as_array()
            .ok_or_else(|| ModelCatalogError::Message(format!("{provider} models must be an array")))?;
        let mut ids: BTreeSet<String> = BTreeSet::new();
        for model in models {
            let id = as_str(&model["id"], &format!("{provider} model id"))?;
            if !ids.insert(id.to_owned()) {
                return fail(format!("duplicate model {provider}/{id}"));
            }
            validate_model(model, provider, id)?;
        }
    }
    Ok(())
}

fn sha256_hex(data: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(data);
    hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

#[derive(Debug, Default)]
struct Inventory {
    providers: usize,
    models: usize,
    by_provider: BTreeMap<String, usize>,
    apis: BTreeMap<String, usize>,
}

fn inventory(catalog: &[Value]) -> Inventory {
    let mut inv = Inventory {
        providers: catalog.len(),
        ..Default::default()
    };
    for entry in catalog {
        let provider = entry["provider"].as_str().unwrap_or_default().to_owned();
        let models = entry["models"].as_array().map(|a| a.len()).unwrap_or(0);
        inv.by_provider.insert(provider, models);
        inv.models += models;
        if let Some(arr) = entry["models"].as_array() {
            for m in arr {
                if let Some(api) = m["api"].as_str() {
                    *inv.apis.entry(api.to_owned()).or_insert(0) += 1;
                }
            }
        }
    }
    inv
}

fn pretty_json_tab(value: &Value) -> String {
    let mut buf = Vec::new();
    let fmt = serde_json::ser::PrettyFormatter::with_indent(b"\t");
    let mut ser = serde_json::Serializer::with_formatter(&mut buf, fmt);
    use serde::ser::Serialize;
    let _ = value.serialize(&mut ser);
    String::from_utf8(buf).unwrap_or_default()
}

/// A materialized raw catalog source (TS literal or JSON object).
#[derive(Debug)]
pub enum RawSource {
    /// The parsed `MODELS` value plus the raw bytes of the source file.
    Value(Value),
}

/// Read and parse a generated catalog source file.
///
/// - `.json` files are parsed directly as the `MODELS` object.
/// - `.ts`/`.js` files must contain `export const MODELS = {...} (as const);`
///   and are parsed with [`parse_exported_const`].
pub fn read_source(path: &Path) -> Result<Value, ModelCatalogError> {
    let bytes = std::fs::read(path)?;
    let text = String::from_utf8_lossy(&bytes).into_owned();
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
        .unwrap_or_default();
    if ext == "json" {
        let v: Value = serde_json::from_str(&text)?;
        obj(&v, "MODELS")?;
        return Ok(v);
    }
    Ok(parse_exported_const(&text, "MODELS")?)
}

pub struct Options<'a> {
    pub source: &'a Path,
    pub overrides: &'a Path,
    pub output: &'a Path,
    pub provenance: &'a Path,
    pub summary_output: Option<&'a Path>,
    /// Revision recorded in provenance for non-remote sources.
    pub revision: String,
    pub source_desc: Value,
    pub remote: bool,
}

pub struct NormalizeResult {
    pub output_sha256: String,
    pub source_sha256: String,
    pub report: String,
}

/// Run the full normalization pipeline (offline, over a materialized source).
pub fn run_normalize(opts: &Options) -> Result<NormalizeResult, ModelCatalogError> {
    let raw = read_source(opts.source)?;
    let source_hash = sha256_hex(serde_json::to_string(&raw)?.as_bytes());

    let mut catalog = normalize(&raw)?;
    let loaded = load_overrides(opts.overrides)?;
    apply_overrides(&mut catalog, &loaded.overrides)?;
    let after = inventory(&catalog);

    let output = format!("{}\n", pretty_json_tab(&Value::Array(catalog)));
    let output_sha = sha256_hex(output.as_bytes());

    let mut provenance_map = Map::new();
    provenance_map.insert("schemaVersion".into(), Value::Number(1.into()));
    provenance_map.insert("source".into(), opts.source_desc.clone());
    let mut overrides_map = Map::new();
    overrides_map.insert(
        "path".into(),
        Value::String(opts.overrides.to_string_lossy().into_owned()),
    );
    overrides_map.insert("sha256".into(), Value::String(sha256_hex(&loaded.bytes)));
    overrides_map.insert("count".into(), Value::Number(loaded.overrides.len().into()));
    provenance_map.insert("overrides".into(), Value::Object(overrides_map));
    provenance_map.insert("outputSha256".into(), Value::String(output_sha.clone()));

    let mut inv_map = Map::new();
    inv_map.insert("providers".into(), Value::Number(after.providers.into()));
    inv_map.insert("models".into(), Value::Number(after.models.into()));
    inv_map.insert(
        "byProvider".into(),
        Value::Object(
            after
                .by_provider
                .iter()
                .map(|(k, v)| (k.clone(), Value::Number((*v).into())))
                .collect(),
        ),
    );
    inv_map.insert(
        "apis".into(),
        Value::Object(
            after
                .apis
                .iter()
                .map(|(k, v)| (k.clone(), Value::Number((*v).into())))
                .collect(),
        ),
    );
    provenance_map.insert("inventory".into(), Value::Object(inv_map));
    let provenance = format!(
        "{}\n",
        pretty_json_tab(&Value::Object(provenance_map))
    );

    std::fs::write(opts.output, output)?;
    std::fs::write(opts.provenance, provenance)?;

    let report = format_summary(&after, &opts.revision, &source_hash, opts.remote);
    if let Some(sout) = opts.summary_output {
        std::fs::write(sout, report.as_bytes())?;
    }

    Ok(NormalizeResult {
        output_sha256: output_sha,
        source_sha256: source_hash,
        report,
    })
}

fn format_summary(after: &Inventory, revision: &str, source_hash: &str, remote: bool) -> String {
    let apis: Vec<String> = after.apis.iter().map(|(k, v)| format!("{k}={v}")).collect();
    let generator = if remote {
        "nix run .#update-model-catalog"
    } else {
        "nix run .#update-model-catalog -- --source <generated>"
    };
    format!(
        "## Model catalog update\n\
         \n\
         - source revision: `{revision}`\n\
         - source catalog SHA-256: `{source_hash}`\n\
         - providers: {}\n\
         - models: {}\n\
         - APIs: {}\n\
         \n\
         Generated by `{generator}`; schema, duplicate IDs, protocol vocabulary, typed \
         Rust round-trip, protocol replay, and flake checks gate merge.\n\
         \n",
        after.providers,
        after.models,
        apis.join(", ")
    )
}

/// SHA-256 helper re-exported for other tool modules.
pub fn sha256(data: &[u8]) -> String {
    sha256_hex(data)
}
