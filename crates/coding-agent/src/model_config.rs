//! `core/model-config.ts` and the models.json half of `core/provider-composer.ts`: custom
//! providers and models, provider-wide `baseUrl`/`compat`/`headers`, and `modelOverrides`, laid
//! over pi-ai's built-in catalog.

use pi_durable::ai::types::{Model, ModelCost};
use serde_json::{Map, Value};
use std::collections::BTreeMap;

/// One provider entry of models.json, kept as JSON.
pub type ProviderConfig = Map<String, Value>;

#[derive(Debug, Clone, Default)]
pub struct ModelConfig {
    pub providers: BTreeMap<String, ProviderConfig>,
    /// Why models.json was ignored, as upstream words it.
    pub error: Option<String>,
}

fn schema_error(parsed: &Value) -> Option<String> {
    let Some(providers) = parsed.get("providers") else {
        return Some("  - providers: must have required property 'providers'".to_owned());
    };
    let Some(providers) = providers.as_object() else {
        return Some("  - providers: must be object".to_owned());
    };
    for (id, provider) in providers {
        let Some(p) = provider.as_object() else {
            return Some(format!("  - providers.{id}: must be object"));
        };
        for key in ["name", "baseUrl", "apiKey", "api"] {
            if p.get(key)
                .is_some_and(|v| !v.as_str().is_some_and(|s| !s.is_empty()))
            {
                return Some(format!(
                    "  - providers.{id}.{key}: must be non-empty string"
                ));
            }
        }
        if let Some(models) = p.get("models") {
            let Some(models) = models.as_array() else {
                return Some(format!("  - providers.{id}.models: must be array"));
            };
            for (i, m) in models.iter().enumerate() {
                if !m
                    .get("id")
                    .and_then(Value::as_str)
                    .is_some_and(|s| !s.is_empty())
                {
                    return Some(format!(
                        "  - providers.{id}.models.{i}.id: must be non-empty string"
                    ));
                }
            }
        }
    }
    None
}

impl ModelConfig {
    /// `ModelConfig.load(path)`: a missing file is no config; a broken one is no config plus an error.
    pub fn load(path: &str) -> Self {
        let text = match std::fs::read(path) {
            Ok(bytes) => String::from_utf8_lossy(&bytes).into_owned(),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Self::default(),
            Err(e) => {
                return Self {
                    error: Some(format!(
                        "Failed to load models.json: {}\n\nFile: {path}",
                        crate::node_fs::message(&e, "open", path)
                    )),
                    ..Self::default()
                };
            }
        };
        let text = text.strip_prefix('\u{feff}').unwrap_or(&text);
        let parsed = match crate::json::parse(&crate::json::strip_json_comments(text)) {
            Ok(v) => v,
            Err(e) => {
                return Self {
                    error: Some(format!("Failed to parse models.json: {e}\n\nFile: {path}")),
                    ..Self::default()
                };
            }
        };
        if let Some(errors) = schema_error(&parsed) {
            return Self {
                error: Some(format!(
                    "Invalid models.json schema:\n{errors}\n\nFile: {path}"
                )),
                ..Self::default()
            };
        }
        let providers = parsed["providers"]
            .as_object()
            .map(|m| {
                m.iter()
                    .filter_map(|(k, v)| Some((k.clone(), v.as_object()?.clone())))
                    .collect()
            })
            .unwrap_or_default();
        Self {
            providers,
            error: None,
        }
    }
}

fn str_of<'a>(map: &'a Map<String, Value>, key: &str) -> Option<&'a str> {
    map.get(key).and_then(Value::as_str)
}

/// `mergeCompat`: shallow, with the routing and template maps merged one level deeper.
fn merge_compat(base: Option<&Value>, over: Option<&Value>) -> Option<Value> {
    let Some(Value::Object(over)) = over else {
        return base.cloned();
    };
    let base_map = base.and_then(Value::as_object).cloned().unwrap_or_default();
    let mut merged = base_map.clone();
    for (k, v) in over {
        merged.insert(k.clone(), v.clone());
    }
    for key in [
        "openRouterRouting",
        "vercelGatewayRouting",
        "chatTemplateKwargs",
        "chatTemplateArgs",
    ] {
        let (b, o) = (base_map.get(key), over.get(key));
        if b.is_some_and(Value::is_object) || o.is_some_and(Value::is_object) {
            let mut nested = b.and_then(Value::as_object).cloned().unwrap_or_default();
            if let Some(Value::Object(o)) = o {
                for (k, v) in o {
                    nested.insert(k.clone(), v.clone());
                }
            }
            merged.insert(key.to_owned(), Value::Object(nested));
        }
    }
    Some(Value::Object(merged))
}

fn shallow(base: Option<&Map<String, Value>>, over: &Value) -> Option<Map<String, Value>> {
    let mut out = base.cloned().unwrap_or_default();
    for (k, v) in over.as_object()? {
        out.insert(k.clone(), v.clone());
    }
    Some(out)
}

fn strings(value: &Value) -> Option<Vec<String>> {
    value.as_array().map(|a| {
        a.iter()
            .filter_map(|v| v.as_str().map(str::to_owned))
            .collect()
    })
}

fn cost(value: &Value, base: Option<&ModelCost>) -> ModelCost {
    let base = base.cloned().unwrap_or_default();
    let num = |k: &str, d: f64| value.get(k).and_then(Value::as_f64).unwrap_or(d);
    ModelCost {
        input: num("input", base.input),
        output: num("output", base.output),
        cache_read: num("cacheRead", base.cache_read),
        cache_write: num("cacheWrite", base.cache_write),
        tiers: value
            .get("tiers")
            .and_then(|t| serde_json::from_value(t.clone()).ok())
            .unwrap_or(base.tiers),
    }
}

/// `applyModelOverride`.
fn apply_override(mut model: Model, o: &Map<String, Value>) -> Model {
    if let Some(name) = str_of(o, "name") {
        model.name = name.to_owned();
    }
    if let Some(r) = o.get("reasoning").and_then(Value::as_bool) {
        model.reasoning = r;
    }
    if let Some(m) = o.get("thinkingLevelMap") {
        model.thinking_level_map =
            shallow(model.thinking_level_map.as_ref(), m).or(model.thinking_level_map);
    }
    if let Some(input) = o.get("input").and_then(strings) {
        model.input = input;
    }
    if let Some(c) = o.get("cost") {
        model.cost = Some(cost(c, model.cost.as_ref()));
    }
    if let Some(Value::Object(pc)) = o.get("promptCache") {
        let mut merged = model
            .prompt_cache
            .as_ref()
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default();
        merged.extend(pc.clone());
        model.prompt_cache = Some(Value::Object(merged));
    }
    if let Some(n) = o.get("contextWindow").and_then(Value::as_u64) {
        model.context_window = n;
    }
    if let Some(n) = o.get("maxTokens").and_then(Value::as_u64) {
        model.max_tokens = n;
    }
    if let Some(s) = o.get("samplingParams") {
        model.sampling_params =
            shallow(model.sampling_params.as_ref(), s).or(model.sampling_params);
    }
    if let Some(Value::Object(by)) = o.get("samplingParamsByThinkingLevel") {
        let mut merged = model
            .sampling_params_by_thinking_level
            .clone()
            .unwrap_or_default();
        for (level, params) in by {
            let base = merged.get(level).and_then(Value::as_object).cloned();
            if let Some(m) = shallow(base.as_ref(), params) {
                merged.insert(level.clone(), Value::Object(m));
            }
        }
        model.sampling_params_by_thinking_level = Some(merged);
    }
    model.compat = merge_compat(model.compat.as_ref(), o.get("compat"));
    model
}

/// `modelFromJson`.
fn model_from_json(
    provider: &str,
    def: &Map<String, Value>,
    config: &ProviderConfig,
    defaults: Option<&Model>,
) -> Result<Model, String> {
    let id = str_of(def, "id").unwrap_or_default();
    let api = str_of(def, "api").or(str_of(config, "api")).map(str::to_owned).or_else(|| defaults.map(|d| d.api.clone()))
        .ok_or_else(|| format!("Provider {provider}, model {id}: no \"api\" specified. Set at provider or model level."))?;
    let base_url = str_of(def, "baseUrl")
        .or(str_of(config, "baseUrl"))
        .map(str::to_owned)
        .or_else(|| defaults.map(|d| d.base_url.clone()))
        .filter(|u| !u.is_empty())
        .ok_or_else(|| {
            format!("Provider {provider}: \"baseUrl\" is required when defining custom models.")
        })?;
    for key in ["contextWindow", "maxTokens"] {
        if def
            .get(key)
            .and_then(Value::as_f64)
            .is_some_and(|n| n <= 0.0)
        {
            let what = if key == "contextWindow" {
                "contextWindow"
            } else {
                "maxTokens"
            };
            return Err(format!("Provider {provider}, model {id}: invalid {what}"));
        }
    }
    let mut model = Model::new(
        provider,
        id,
        &api,
        def.get("contextWindow")
            .and_then(Value::as_u64)
            .unwrap_or(128_000),
        def.get("maxTokens")
            .and_then(Value::as_u64)
            .unwrap_or(16_384),
        def.get("reasoning")
            .and_then(Value::as_bool)
            .unwrap_or(false),
    );
    model.name = str_of(def, "name").unwrap_or(id).to_owned();
    model.base_url = base_url;
    model.thinking_level_map = def
        .get("thinkingLevelMap")
        .and_then(Value::as_object)
        .cloned();
    model.input = def
        .get("input")
        .and_then(strings)
        .unwrap_or_else(|| vec!["text".to_owned()]);
    model.cost = Some(def.get("cost").map(|c| cost(c, None)).unwrap_or_default());
    model.prompt_cache = def.get("promptCache").cloned();
    model.sampling_params = def
        .get("samplingParams")
        .and_then(Value::as_object)
        .cloned();
    model.sampling_params_by_thinking_level = def
        .get("samplingParamsByThinkingLevel")
        .and_then(Value::as_object)
        .cloned();
    model.headers = None;
    model.compat = merge_compat(config.get("compat"), def.get("compat"));
    Ok(model)
}

/// `findModelDefaults`.
fn find_defaults<'a>(models: &'a [Model], id: &str, api: Option<&str>) -> Option<&'a Model> {
    models
        .iter()
        .find(|m| m.id == id)
        .or_else(|| api.and_then(|a| models.iter().find(|m| m.api == a)))
        .or_else(|| models.iter().find(|m| m.api == "openai-completions"))
        .or_else(|| models.first())
}

/// The models of one provider: its built-in ones with models.json applied (`applyModelsJson`,
/// then `modelOverrides`). Err is a composition error; upstream then keeps the built-in models.
pub fn compose(
    provider: &str,
    base: &[Model],
    config: Option<&ProviderConfig>,
) -> Result<Vec<Model>, String> {
    let Some(config) = config else {
        return Ok(base.to_vec());
    };
    if config.get("oauth").is_some() && str_of(config, "baseUrl").is_none() {
        return Err(format!(
            "Provider {provider}: \"baseUrl\" is required when \"oauth\" is set."
        ));
    }
    let has_overrides = config
        .get("modelOverrides")
        .and_then(Value::as_object)
        .is_some_and(|m| !m.is_empty());
    let has_models = config
        .get("models")
        .and_then(Value::as_array)
        .is_some_and(|m| !m.is_empty());
    if !has_models
        && ![
            "baseUrl",
            "headers",
            "compat",
            "apiKey",
            "oauth",
            "authHeader",
        ]
        .iter()
        .any(|k| config.contains_key(*k))
        && !has_overrides
    {
        return Err(format!(
            "Provider {provider}: must specify \"baseUrl\", \"headers\", \"compat\", \"modelOverrides\", or \"models\"."
        ));
    }
    let mut models: Vec<Model> = base
        .iter()
        .map(|m| {
            let mut m = m.clone();
            if config.get("oauth").is_none() {
                if let Some(url) = str_of(config, "baseUrl") {
                    m.base_url = url.to_owned();
                }
            }
            m.compat = merge_compat(m.compat.as_ref(), config.get("compat"));
            m
        })
        .collect();
    for def in config
        .get("models")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let Some(def) = def.as_object() else { continue };
        let id = str_of(def, "id").unwrap_or_default();
        let api = str_of(def, "api").or(str_of(config, "api"));
        let defaults = find_defaults(&models, id, api).cloned();
        let model = model_from_json(provider, def, config, defaults.as_ref())?;
        match models.iter().position(|m| m.id == id) {
            Some(i) => models[i] = model,
            None => models.push(model),
        }
    }
    if let Some(Value::Object(overrides)) = config.get("modelOverrides") {
        models = models
            .into_iter()
            .map(|m| match overrides.get(&m.id).and_then(Value::as_object) {
                Some(o) => apply_override(m, o),
                None => m,
            })
            .collect();
    }
    Ok(models)
}

/// The headers models.json sets for one model: `modelOverrides[id].headers`, then the model
/// definition's (`rawModelHeaders`). Values are still unresolved.
pub fn model_headers(
    model: &Model,
    config: Option<&ProviderConfig>,
) -> Option<BTreeMap<String, String>> {
    let config = config?;
    let mut out = BTreeMap::new();
    let mut add = |v: Option<&Value>| {
        for (k, v) in v.and_then(Value::as_object).into_iter().flatten() {
            if let Some(s) = v.as_str() {
                out.insert(k.clone(), s.to_owned());
            }
        }
    };
    add(config
        .get("modelOverrides")
        .and_then(|o| o.get(&model.id))
        .and_then(|o| o.get("headers")));
    add(config
        .get("models")
        .and_then(Value::as_array)
        .and_then(|ms| {
            ms.iter()
                .find(|m| m.get("id").and_then(Value::as_str) == Some(&model.id))
        })
        .and_then(|m| m.get("headers")));
    (!out.is_empty()).then_some(out)
}

/// Provider-level headers of models.json, unresolved.
pub fn provider_headers(config: Option<&ProviderConfig>) -> Option<BTreeMap<String, String>> {
    let map = config?.get("headers")?.as_object()?;
    Some(
        map.iter()
            .filter_map(|(k, v)| Some((k.clone(), v.as_str()?.to_owned())))
            .collect(),
    )
}
