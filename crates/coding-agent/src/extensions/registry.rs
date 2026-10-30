use pi_durable::ai::classify::{self, ClassifierHeaders, ClassifierModel, ClassifierOptions};
use pi_durable::ai::types::{Model, SimpleStreamOptions};
use pi_durable::cancel::CancelToken;
use serde_json::Value;

use super::{ExtensionModels, options_of};

const REASONING_EFFORT_APIS: [&str; 5] = [
    "openai-completions",
    "openai-responses",
    "azure-openai-responses",
    "openai-codex-responses",
    "mistral-conversations",
];

pub(super) fn complete_options(
    model: &Model,
    given: &Value,
    cancel: &CancelToken,
) -> SimpleStreamOptions {
    let mut options = options_of(given, Some(cancel.clone()));
    options.reasoning = given
        .get("reasoningEffort")
        .and_then(Value::as_str)
        .filter(|_| REASONING_EFFORT_APIS.contains(&model.api.as_str()))
        .and_then(crate::run_config::parse_level)
        .and_then(pi_durable::harness::generation::reasoning);
    options
}

impl ExtensionModels {
    pub(super) fn classify(&self, args: &Value, cancel: &CancelToken) -> Result<Value, String> {
        let raw = args.get("model").cloned().unwrap_or(Value::Null);
        let model: ClassifierModel = serde_json::from_value(raw).unwrap_or_default();
        let context = args.get("context").cloned().unwrap_or(Value::Null);
        let given = args.get("options").unwrap_or(&Value::Null);
        let aborted = cancel.is_cancelled() || args.get("aborted") == Some(&Value::Bool(true));
        let (model, options) = self.prepare_classify(&model, &context, given, aborted, cancel)?;
        let result = classify::classify(&model, &context, &options);
        let mut value = serde_json::to_value(result).map_err(|e| e.to_string())?;
        crate::sessions::export::upstream_usage(&mut value);
        Ok(value)
    }

    fn prepare_classify(
        &self,
        model: &ClassifierModel,
        context: &Value,
        given: &Value,
        aborted: bool,
        cancel: &CancelToken,
    ) -> Result<(ClassifierModel, ClassifierOptions), String> {
        let provider = model.provider.as_deref().unwrap_or_default();
        let id = model.id.as_deref().unwrap_or_default();
        let api = model.api.as_deref().unwrap_or_default();
        if model.kind.as_deref() != Some("classifier") {
            return Err(format!("Model {provider}/{id} is not a classifier model"));
        }
        let images = context
            .get("images")
            .and_then(Value::as_array)
            .is_some_and(|images| !images.is_empty());
        if images && !model.input.iter().any(|kind| kind == "image") {
            return Err(format!("Model {provider}/{id} does not accept image input"));
        }
        if !self.registry.provider_exists(provider) {
            return Err(format!("Unknown provider: {provider}"));
        }
        if aborted {
            return Err("This operation was aborted".to_owned());
        }
        let mut options = ClassifierOptions::from_json(given);
        if options.api_key.is_none() && !self.registry.has_configured_auth(provider) {
            return Err(format!("Provider is not configured: {provider}"));
        }
        let auth = self
            .registry
            .provider_auth(provider, crate::auth::OAUTH_MINIMUM_VALIDITY_MS)?;
        let apis = classify::classifier_apis(provider);
        if apis.is_empty() {
            return Err(format!(
                "Provider {provider} does not support classification"
            ));
        }
        if !apis.contains(&api) {
            return Err(format!(
                "Provider {provider} has no classifier implementation for \"{api}\""
            ));
        }
        let mut model = model.clone();
        if auth.base_url.is_some() {
            model.base_url = auth.base_url;
        }
        let mut headers: Vec<_> = auth
            .headers
            .into_iter()
            .map(|(name, value)| (name, Some(value)))
            .collect();
        for (name, value) in options.headers.0 {
            headers.retain(|(known, _)| !known.eq_ignore_ascii_case(&name));
            headers.push((name, value));
        }
        options.headers = ClassifierHeaders(headers);
        options.api_key = options.api_key.or(auth.api_key);
        options.signal = Some(cancel.clone());
        Ok((model, options))
    }
}
