use std::fmt;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use pi_chord::json::{JsonObject, JsonValue};
use pi_rt::cancel::CancelToken;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::error_body::format_provider_error;
use crate::http::{self, HttpError, Request};
use crate::types::{Cost, ModelCost};

const SYSTEM_ONE: &str = "typesafe-system-one";
const LABEL: &str = "System One API";
const DEFAULT_MAX_RETRIES: u64 = 2;
const DEFAULT_MAX_RETRY_DELAY_MS: f64 = 60_000.0;
const MAX_DATA_URL_IMAGES: usize = 4;

#[derive(Debug, Clone, Default)]
pub struct ClassifierHeaders(pub Vec<(String, Option<String>)>);

impl<'de> Deserialize<'de> for ClassifierHeaders {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct Headers;
        impl<'de> serde::de::Visitor<'de> for Headers {
            type Value = ClassifierHeaders;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("an object of string or null headers")
            }

            fn visit_map<M: serde::de::MapAccess<'de>>(
                self,
                mut map: M,
            ) -> Result<Self::Value, M::Error> {
                let mut headers = Vec::new();
                while let Some(header) = map.next_entry::<String, Option<String>>()? {
                    headers.push(header);
                }
                Ok(ClassifierHeaders(headers))
            }
        }
        deserializer.deserialize_map(Headers)
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClassifierModel {
    #[serde(rename = "type")]
    pub kind: Option<String>,
    pub id: Option<String>,
    pub api: Option<String>,
    pub provider: Option<String>,
    pub base_url: Option<String>,
    #[serde(default)]
    pub input: Vec<String>,
    pub headers: Option<ClassifierHeaders>,
    pub compat: Option<SystemOneCompat>,
    pub cost: Option<ModelCost>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SystemOneCompat {
    pub image_input: Option<String>,
    pub send_image_opt_in_header: Option<bool>,
}

#[derive(Clone, Default)]
pub struct ClassifierOptions {
    pub api_key: Option<String>,
    pub headers: ClassifierHeaders,
    pub timeout_ms: Option<u64>,
    pub max_retries: Option<u64>,
    pub max_retry_delay_ms: Option<f64>,
    pub identity: Option<JsonObject>,
    pub signal: Option<CancelToken>,
}

impl ClassifierOptions {
    pub fn from_json(value: &JsonValue) -> Self {
        let field = |key: &str| value.get(key).filter(|field| !field.is_null());
        ClassifierOptions {
            api_key: field("apiKey")
                .and_then(JsonValue::as_str)
                .map(str::to_owned),
            headers: field("headers")
                .and_then(|headers| serde_json::from_value(headers.clone()).ok())
                .unwrap_or_default(),
            timeout_ms: field("timeoutMs").and_then(JsonValue::as_u64),
            max_retries: field("maxRetries").and_then(JsonValue::as_u64),
            max_retry_delay_ms: field("maxRetryDelayMs").and_then(JsonValue::as_f64),
            identity: field("identity").and_then(JsonValue::as_object).cloned(),
            signal: None,
        }
    }

    fn aborted(&self) -> bool {
        self.signal.as_ref().is_some_and(CancelToken::is_cancelled)
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClassifierResult {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    pub answers: JsonObject,
    pub stop_reason: &'static str,
    pub timestamp: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage: Option<ClassifierUsage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_message: Option<String>,
}

pub fn classifier_apis(provider: &str) -> &'static [&'static str] {
    match provider {
        "typesafe" | "openrouter" | "opencode" | "vercel-ai-gateway" => &[SYSTEM_ONE],
        "cloudflare-workers-ai" => &["cloudflare-workers-ai-system-one"],
        "openai" => &["openai-decisions"],
        _ => &[],
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_millis() as u64)
}

fn result(model: &ClassifierModel) -> ClassifierResult {
    ClassifierResult {
        api: model.api.clone(),
        provider: model.provider.clone(),
        model: model.id.clone(),
        answers: JsonObject::new(),
        usage: None,
        stop_reason: "stop",
        error_message: None,
        timestamp: now_ms(),
    }
}

fn error_result(model: &ClassifierModel, message: &str, aborted: bool) -> ClassifierResult {
    ClassifierResult {
        stop_reason: if aborted { "aborted" } else { "error" },
        error_message: Some(message.to_owned()),
        ..result(model)
    }
}

pub fn classify(
    model: &ClassifierModel,
    context: &JsonValue,
    options: &ClassifierOptions,
) -> ClassifierResult {
    match model.api.as_deref() {
        Some(SYSTEM_ONE) => system_one(model, context, options),
        api => error_result(
            model,
            &format!(
                "pi-rs does not implement the {} classifier API yet",
                api.unwrap_or_default()
            ),
            options.aborted(),
        ),
    }
}

struct Failure {
    message: String,
    status: Option<u16>,
    body: Option<String>,
}

impl Failure {
    fn plain(message: impl Into<String>) -> Self {
        Failure {
            message: message.into(),
            status: None,
            body: None,
        }
    }
}

fn system_one(
    model: &ClassifierModel,
    context: &JsonValue,
    options: &ClassifierOptions,
) -> ClassifierResult {
    let mut output = result(model);
    if let Err(failure) = system_one_into(model, context, options, &mut output) {
        output.stop_reason = if options.aborted() {
            "aborted"
        } else {
            "error"
        };
        output.error_message = Some(format_provider_error(
            Some(&format!("{LABEL} error")),
            failure.status,
            &failure.message,
            failure.body.as_deref(),
        ));
    }
    output
}

fn system_one_into(
    model: &ClassifierModel,
    context: &JsonValue,
    options: &ClassifierOptions,
    output: &mut ClassifierResult,
) -> Result<(), Failure> {
    let id = model.id.clone().unwrap_or_default();
    let provider = model.provider.clone().unwrap_or_default();
    let base_url = model.base_url.clone().unwrap_or_default();
    let questions = context
        .get("questions")
        .and_then(JsonValue::as_object)
        .cloned()
        .unwrap_or_default();
    let mut request = JsonObject::new();
    request.insert("model".to_owned(), json!(id));
    request.insert(
        "state".to_owned(),
        context.get("state").cloned().unwrap_or(JsonValue::Null),
    );
    request.insert("questions".to_owned(), JsonValue::Object(wire(&questions)));
    let mut opt_in = false;
    let images = context
        .get("images")
        .and_then(JsonValue::as_array)
        .filter(|images| !images.is_empty());
    if let Some(images) = images {
        if !model.input.iter().any(|kind| kind == "image") {
            return Err(Failure::plain(format!(
                "{LABEL} does not support image input for {id}"
            )));
        }
        let openrouter = provider == "openrouter" || base_url.contains("openrouter.ai");
        let compat = model.compat.clone().unwrap_or_default();
        let input = compat.image_input.unwrap_or_else(|| {
            if openrouter {
                "state-image-urls".to_owned()
            } else {
                "assets".to_owned()
            }
        });
        add_images(&id, &input, images, &mut request)?;
        opt_in = compat.send_image_opt_in_header.unwrap_or(!openrouter);
    }
    let url = format!("{}/systemone", base_url.trim_end_matches('/'));
    let body = post(&url, model, &JsonValue::Object(request), options, opt_in)?;
    let JsonValue::Object(body) = body else {
        return Err(Failure::plain(format!(
            "{LABEL} returned an unexpected response"
        )));
    };
    output.usage = body
        .get("usage")
        .and_then(|usage| parse_usage(usage, model));
    output.answers = parse_answers(body.get("answers"), &questions)?;
    Ok(())
}

fn wire(questions: &JsonObject) -> JsonObject {
    questions
        .iter()
        .map(|(id, question)| {
            let mut question = question.clone();
            if let JsonValue::Object(fields) = &mut question
                && fields.get("type").and_then(JsonValue::as_str) == Some("bool")
            {
                fields.insert("type".to_owned(), json!("noul"));
            }
            (id.clone(), question)
        })
        .collect()
}

fn add_images(
    id: &str,
    input: &str,
    images: &[JsonValue],
    request: &mut JsonObject,
) -> Result<(), Failure> {
    let image = |item: &JsonValue| {
        let field = |key: &str| {
            item.get(key)
                .and_then(JsonValue::as_str)
                .unwrap_or_default()
                .to_owned()
        };
        (field("mimeType"), field("data"))
    };
    let state = request.get("state").cloned().unwrap_or(JsonValue::Null);
    match input {
        "images" => {
            if images.len() > MAX_DATA_URL_IMAGES {
                return Err(Failure::plain(format!(
                    "{LABEL} accepts at most {MAX_DATA_URL_IMAGES} images for {id}, got {}",
                    images.len()
                )));
            }
            let mut urls = Vec::new();
            for item in images {
                let (mime, data) = image(item);
                if !matches!(mime.as_str(), "image/png" | "image/jpeg" | "image/webp") {
                    return Err(Failure::plain(format!(
                        "{LABEL} accepts PNG, JPEG, and WebP images for {id}, got {mime}"
                    )));
                }
                urls.push(json!(format!("data:{mime};base64,{data}")));
            }
            request.insert("images".to_owned(), JsonValue::Array(urls));
        }
        "state-image-urls" => {
            let mut parts: Vec<JsonValue> = images
                .iter()
                .map(|item| {
                    let (mime, data) = image(item);
                    json!({"type": "image_url", "image_url": {"url": format!("data:{mime};base64,{data}")}})
                })
                .collect();
            parts.push(state);
            request.insert("state".to_owned(), JsonValue::Array(parts));
        }
        _ => {
            let mut assets = JsonObject::new();
            for (index, item) in images.iter().enumerate() {
                let (mime, data) = image(item);
                let format = match mime.as_str() {
                    "image/png" => "png",
                    "image/jpeg" => "jpeg",
                    _ => {
                        return Err(Failure::plain(format!(
                            "{LABEL} accepts PNG and JPEG images for {id}, got {mime}"
                        )));
                    }
                };
                assets.insert(
                    format!("<image-{}>", index + 1),
                    json!({"type": "image", "format": format, "data_base64": data}),
                );
            }
            let mut names: Vec<JsonValue> = assets.keys().map(|name| json!(name)).collect();
            names.push(state);
            request.insert("assets".to_owned(), JsonValue::Object(assets));
            request.insert("state".to_owned(), JsonValue::Array(names));
        }
    }
    Ok(())
}

fn headers(
    model: &ClassifierModel,
    api_key: &str,
    options: &ClassifierOptions,
    opt_in: bool,
) -> Vec<(String, String)> {
    let mut merged: Vec<(String, (String, String))> = Vec::new();
    let mut set = |name: &str, value: Option<&str>| {
        let key = name.to_ascii_lowercase();
        merged.retain(|(known, _)| *known != key);
        if let Some(value) = value {
            merged.push((key, (name.to_owned(), value.to_owned())));
        }
    };
    for (name, value) in identity_headers(model, options.identity.as_ref()) {
        set(&name, Some(&value));
    }
    set("authorization", Some(&format!("Bearer {api_key}")));
    set("content-type", Some("application/json"));
    for (name, value) in model.headers.iter().flat_map(|headers| &headers.0) {
        set(name, value.as_deref());
    }
    if opt_in {
        set("TypeSafe-Danger-Zone-Images", Some("true"));
    }
    for (name, value) in &options.headers.0 {
        set(name, value.as_deref());
    }
    merged.into_iter().map(|(_, header)| header).collect()
}

fn identity_headers(
    model: &ClassifierModel,
    identity: Option<&JsonObject>,
) -> Vec<(String, String)> {
    let mut out = Vec::new();
    match identity.and_then(|identity| identity.get("userAgent")) {
        Some(JsonValue::String(mode)) if mode == "short" => {
            out.push(("User-Agent".to_owned(), "pi".to_owned()));
        }
        Some(JsonValue::Object(mode)) => {
            if let Some(custom) = mode.get("custom").and_then(JsonValue::as_str) {
                out.push(("User-Agent".to_owned(), custom.to_owned()));
            }
        }
        _ => {}
    }
    let Some(attribution) = identity
        .and_then(|identity| identity.get("attribution"))
        .and_then(JsonValue::as_object)
    else {
        return out;
    };
    let field = |key: &str| {
        attribution
            .get(key)
            .and_then(JsonValue::as_str)
            .unwrap_or_default()
            .to_owned()
    };
    let (name, url) = (field("name"), field("url"));
    let provider = model.provider.as_deref().unwrap_or_default();
    let base_url = model.base_url.as_deref().unwrap_or_default();
    let host = base_url
        .split_once("://")
        .map(|(_, rest)| rest.split(['/', ':', '?', '#']).next().unwrap_or_default())
        .unwrap_or_default();
    if provider == "openrouter" || base_url.contains("openrouter.ai") {
        out.push(("HTTP-Referer".to_owned(), url));
        out.push(("X-OpenRouter-Title".to_owned(), name));
        let categories: Vec<&str> = attribution
            .get("categories")
            .and_then(JsonValue::as_array)
            .into_iter()
            .flatten()
            .filter_map(JsonValue::as_str)
            .collect();
        if !categories.is_empty() {
            out.push(("X-OpenRouter-Categories".to_owned(), categories.join(",")));
        }
    } else if provider == "nvidia" || host == "integrate.api.nvidia.com" {
        let mut chars = name.chars();
        let origin = chars
            .next()
            .map(|first| first.to_uppercase().chain(chars).collect())
            .unwrap_or_default();
        out.push(("X-BILLING-INVOKE-ORIGIN".to_owned(), origin));
    } else if provider == "opencode" || provider == "opencode-go" || host == "opencode.ai" {
        out.push(("x-opencode-client".to_owned(), name));
    }
    out
}

enum Attempt {
    Status {
        status: u16,
        should_retry: Option<String>,
        retry_after_ms: Option<String>,
        retry_after: Option<String>,
        body: String,
    },
    Timeout(u64),
    Other(String),
}

impl Attempt {
    fn message(&self) -> String {
        match self {
            Attempt::Status { status, .. } => format!("{LABEL} returned {status}"),
            Attempt::Timeout(ms) => format!("Request timed out after {ms}ms"),
            Attempt::Other(message) => message.clone(),
        }
    }

    fn retryable(&self) -> bool {
        match self {
            Attempt::Status {
                status,
                should_retry,
                ..
            } => match should_retry.as_deref() {
                Some("true") => true,
                Some("false") => false,
                _ => matches!(status, 408 | 409 | 429) || *status >= 500,
            },
            Attempt::Timeout(_) => true,
            Attempt::Other(_) => false,
        }
    }

    fn failure(self) -> Failure {
        let message = self.message();
        match self {
            Attempt::Status { status, body, .. } => Failure {
                message,
                status: Some(status),
                body: Some(body),
            },
            _ => Failure::plain(message),
        }
    }
}

fn post(
    url: &str,
    model: &ClassifierModel,
    body: &JsonValue,
    options: &ClassifierOptions,
    opt_in: bool,
) -> Result<JsonValue, Failure> {
    let Some(api_key) = options.api_key.as_deref().filter(|key| !key.is_empty()) else {
        return Err(Failure::plain(format!(
            "No API key for provider: {}",
            model.provider.as_deref().unwrap_or_default()
        )));
    };
    let mut request =
        Request::new("POST", url).timeout(options.timeout_ms.map(Duration::from_millis));
    request.body = serde_json::to_vec(body).map_err(|error| Failure::plain(error.to_string()))?;
    for (name, value) in headers(model, api_key, options, opt_in) {
        request = request.header(&name, value);
    }
    let max_retries = options.max_retries.unwrap_or(DEFAULT_MAX_RETRIES);
    let mut remaining = max_retries;
    loop {
        let attempt = match attempt(&request, options) {
            Ok(json) => return Ok(json),
            Err(attempt) => attempt,
        };
        if options.aborted() {
            return Err(Failure::plain("Request aborted"));
        }
        if remaining == 0 || !attempt.retryable() {
            return Err(attempt.failure());
        }
        let index = max_retries - remaining;
        remaining -= 1;
        let delay = retry_delay(&attempt, index, options.max_retry_delay_ms)?;
        let interrupted = match &options.signal {
            Some(signal) => signal.wait_timeout(delay),
            None => {
                std::thread::sleep(delay);
                false
            }
        };
        if interrupted {
            return Err(Failure::plain("Request aborted"));
        }
    }
}

fn attempt(request: &Request, options: &ClassifierOptions) -> Result<JsonValue, Attempt> {
    let signal = options.signal.as_ref();
    let fail = |error: HttpError| match (error, options.timeout_ms) {
        (HttpError::Timeout(_), Some(ms)) => Attempt::Timeout(ms),
        (HttpError::Cancelled, _) => Attempt::Other("This operation was aborted".to_owned()),
        _ => Attempt::Other("fetch failed".to_owned()),
    };
    let response = http::send(request, signal).map_err(fail)?;
    let status = response.status;
    if !response.ok() {
        let header = |name: &str| response.header(name).map(str::to_owned);
        let (should_retry, retry_after_ms, retry_after) = (
            header("x-should-retry"),
            header("retry-after-ms"),
            header("retry-after"),
        );
        let body = response.text().map_err(fail)?;
        return Err(Attempt::Status {
            status,
            should_retry,
            retry_after_ms,
            retry_after,
            body,
        });
    }
    let text = response.text().map_err(fail)?;
    serde_json::from_str(&text).map_err(|error| Attempt::Other(error.to_string()))
}

fn js_float(text: &str) -> Option<f64> {
    let text = text.trim_start();
    let bytes = text.as_bytes();
    let end = (0..bytes.len())
        .take_while(|&index| {
            let ch = bytes[index];
            let after_exponent = index > 0 && matches!(bytes[index - 1], b'e' | b'E');
            ch.is_ascii_digit()
                || matches!(ch, b'.' | b'e' | b'E')
                || ((index == 0 || after_exponent) && matches!(ch, b'-' | b'+'))
        })
        .count();
    (1..=end)
        .rev()
        .find_map(|cut| text[..cut].parse::<f64>().ok())
        .filter(|value| value.is_finite())
}

fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = year.div_euclid(400);
    let year_of_era = year - era * 400;
    let day_of_year = (153 * ((month + 9) % 12) + 2) / 5 + day - 1;
    era * 146_097 + year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year - 719_468
}

fn http_date_ms(text: &str) -> Option<f64> {
    const MONTHS: [&str; 12] = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    let parts: Vec<&str> = text
        .split([' ', ',', '-'])
        .filter(|part| !part.is_empty())
        .collect();
    let (day, month, year, time) = match parts.as_slice() {
        [_, day, month, year, time, "GMT"] => (*day, *month, *year, *time),
        [_, month, day, time, year] => (*day, *month, *year, *time),
        _ => return None,
    };
    let month = MONTHS.iter().position(|name| *name == month)? as i64 + 1;
    let day: i64 = day.parse().ok().filter(|day| (1..=31).contains(day))?;
    let mut year: i64 = year.parse().ok()?;
    if year < 100 {
        year += if year < 50 { 2000 } else { 1900 };
    }
    let clock: Vec<i64> = time
        .split(':')
        .map(str::parse)
        .collect::<Result<_, _>>()
        .ok()?;
    let [hours, minutes, seconds] = clock[..] else {
        return None;
    };
    if hours > 23 || minutes > 59 || seconds > 60 {
        return None;
    }
    let total = days_from_civil(year, month, day) * 86_400 + hours * 3_600 + minutes * 60 + seconds;
    Some(total as f64 * 1000.0)
}

fn retry_delay(
    attempt: &Attempt,
    index: u64,
    max_delay_ms: Option<f64>,
) -> Result<Duration, Failure> {
    let server = match attempt {
        Attempt::Status {
            retry_after_ms,
            retry_after,
            ..
        } => retry_after_ms.as_deref().and_then(js_float).or_else(|| {
            let retry_after = retry_after.as_deref().filter(|value| !value.is_empty())?;
            match js_float(retry_after) {
                Some(seconds) => Some(seconds * 1000.0),
                None => http_date_ms(retry_after).map(|at| at - now_ms() as f64),
            }
        }),
        _ => None,
    };
    let delay = match server {
        Some(delay) => {
            let max = max_delay_ms.unwrap_or(DEFAULT_MAX_RETRY_DELAY_MS);
            if max > 0.0 && delay > max {
                return Err(Failure::plain(format!(
                    "Server requested {}s retry delay (max: {}s). {}",
                    (delay / 1000.0).ceil(),
                    (max / 1000.0).ceil(),
                    attempt.message()
                )));
            }
            delay
        }
        None => {
            let exponential = (0.5 * 2f64.powi(index.min(30) as i32)).min(8.0) * 1000.0;
            exponential * (1.0 - jitter() * 0.25)
        }
    };
    Ok(Duration::from_secs_f64(delay.max(0.0) / 1000.0))
}

fn jitter() -> f64 {
    use std::hash::{BuildHasher, Hasher};
    let bits = std::collections::hash_map::RandomState::new()
        .build_hasher()
        .finish();
    (bits >> 11) as f64 / (1u64 << 53) as f64
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClassifierUsage {
    pub input: f64,
    pub output: f64,
    pub cache_read: u64,
    pub cache_write: u64,
    pub total_tokens: f64,
    pub cost: Cost,
}

fn tokens(value: Option<&JsonValue>) -> f64 {
    value
        .and_then(JsonValue::as_f64)
        .filter(|count| count.is_finite() && *count > 0.0)
        .unwrap_or(0.0)
}

fn parse_usage(value: &JsonValue, model: &ClassifierModel) -> Option<ClassifierUsage> {
    let usage = value.as_object()?;
    if usage.get("input_tokens").is_none() && usage.get("output_tokens").is_none() {
        return None;
    }
    let (input, output) = (
        tokens(usage.get("input_tokens")),
        tokens(usage.get("output_tokens")),
    );
    let cost = model.cost.clone().unwrap_or_default();
    let (mut input_rate, mut output_rate) = (cost.input, cost.output);
    let mut matched = -1.0;
    for tier in &cost.tiers {
        let above = tier.input_tokens_above as f64;
        if input > above && above > matched {
            (input_rate, output_rate) = (tier.input, tier.output);
            matched = above;
        }
    }
    let (input_cost, output_cost) = (
        input_rate / 1_000_000.0 * input,
        output_rate / 1_000_000.0 * output,
    );
    Some(ClassifierUsage {
        input,
        output,
        cache_read: 0,
        cache_write: 0,
        total_tokens: input + output,
        cost: Cost {
            input: input_cost,
            output: output_cost,
            cache_read: 0.0,
            cache_write: 0.0,
            total: input_cost + output_cost,
            total_from_provider: Some(false),
        },
    })
}

fn number(value: Option<&JsonValue>, field: &str) -> Result<f64, Failure> {
    value
        .and_then(JsonValue::as_f64)
        .filter(|number| number.is_finite())
        .ok_or_else(|| Failure::plain(format!("{LABEL} returned an invalid {field}")))
}

fn parse_answers(value: Option<&JsonValue>, questions: &JsonObject) -> Result<JsonObject, Failure> {
    let Some(JsonValue::Object(answers)) = value else {
        return Err(Failure::plain(format!(
            "{LABEL} returned an unexpected response"
        )));
    };
    let mut out = JsonObject::new();
    for (id, question) in questions {
        let Some(JsonValue::Object(answer)) = answers.get(id) else {
            return Err(Failure::plain(format!(
                "{LABEL} did not return an answer for {id}"
            )));
        };
        let kind = answer.get("type").and_then(JsonValue::as_str);
        let parsed = match question.get("type").and_then(JsonValue::as_str) {
            Some("choice") => {
                let choice = answer.get("choice").and_then(JsonValue::as_str);
                let (Some("choice"), Some(choice)) = (kind, choice) else {
                    return Err(Failure::plain(format!(
                        "{LABEL} did not return a choice answer for {id}"
                    )));
                };
                let Some(JsonValue::Object(given)) = answer.get("probabilities") else {
                    return Err(Failure::plain(format!(
                        "{LABEL} returned invalid probabilities for {id}"
                    )));
                };
                let mut probabilities = JsonObject::new();
                for (key, probability) in given {
                    let field = format!("probability for {id}.{key}");
                    probabilities.insert(key.clone(), json!(number(Some(probability), &field)?));
                }
                json!({
                    "type": "choice",
                    "choice": choice,
                    "probabilities": probabilities,
                    "confidence": number(answer.get("confidence"), &format!("confidence for {id}"))?,
                })
            }
            Some("score") => {
                if kind != Some("score") {
                    return Err(Failure::plain(format!(
                        "{LABEL} did not return a score answer for {id}"
                    )));
                }
                json!({
                    "type": "score",
                    "score": number(answer.get("score"), &format!("score for {id}"))?,
                    "confidence": number(answer.get("confidence"), &format!("confidence for {id}"))?,
                })
            }
            _ => {
                if kind != Some("noul") {
                    return Err(Failure::plain(format!(
                        "{LABEL} did not return a bool answer for {id}"
                    )));
                }
                json!({
                    "type": "bool",
                    "probability": number(answer.get("noul"), &format!("probability for {id}"))?,
                })
            }
        };
        out.insert(id.clone(), parsed);
    }
    Ok(out)
}
