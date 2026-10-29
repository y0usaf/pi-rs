use std::time::Duration;

use pi_durable::ai::http::{self, Request};
use pi_durable::ai::types::Model;
use pi_durable::cancel::CancelToken;
use serde_json::{Map, Value, json};

use crate::auth::FileLock;

const CATALOG_URL: &str = "https://pi.dev";
const REFRESH_INTERVAL_MS: i64 = 4 * 60 * 60 * 1000;
const ATTEMPT_TIMEOUT: Duration = Duration::from_secs(4);
const MAX_RETRIES: u32 = 2;
const RETRYABLE: [u16; 7] = [408, 425, 429, 500, 502, 503, 504];
const MODEL_TYPES: [&str; 3] = ["chat", "image", "classifier"];
const LOCK_WAIT: Duration = Duration::from_secs(10);

pub type Entry = Map<String, Value>;

pub fn extends(provider: &str) -> bool {
    provider != "radius"
}

fn base_url() -> String {
    std::env::var("PI_CATALOG_URL")
        .ok()
        .filter(|url| !url.is_empty())
        .unwrap_or_else(|| CATALOG_URL.to_owned())
}

pub struct Store {
    path: String,
}

impl Store {
    pub fn at(path: &str) -> Self {
        Self {
            path: path.to_owned(),
        }
    }

    fn parse(&self) -> Map<String, Value> {
        let text = std::fs::read_to_string(&self.path).unwrap_or_default();
        match crate::json::parse(text.strip_prefix('\u{feff}').unwrap_or(&text)) {
            Ok(Value::Object(entries)) => entries,
            _ => Map::new(),
        }
    }

    pub fn entries(&self) -> Map<String, Value> {
        let _lock = FileLock::acquire(&self.path, LOCK_WAIT).ok();
        self.parse()
    }

    pub fn write(&self, provider: &str, entry: &Entry) -> Result<(), String> {
        let _lock = FileLock::acquire(&self.path, LOCK_WAIT)?;
        let mut entries = self.parse();
        entries.insert(provider.to_owned(), Value::Object(entry.clone()));
        let text =
            serde_json::to_string_pretty(&Value::Object(entries)).map_err(|e| e.to_string())?;
        crate::auth::write_private(&self.path, &text)
    }
}

pub fn overlay(provider: &str, entry: &Entry) -> Vec<Model> {
    if entry
        .get("lastModified")
        .and_then(Value::as_f64)
        .is_none_or(|modified| modified <= pi_durable::ai::catalog::GENERATED_AT_MS as f64)
    {
        return Vec::new();
    }
    entry
        .get("models")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|model| model.get("provider").and_then(Value::as_str) == Some(provider))
        .filter(|model| model.get("type").is_none_or(|kind| kind == "chat"))
        .filter_map(|model| serde_json::from_value(model.clone()).ok())
        .collect()
}

pub fn merge(base: &[Model], overlay: &[Model]) -> Vec<Model> {
    let mut merged = base.to_vec();
    for model in overlay {
        match merged.iter_mut().find(|known| known.id == model.id) {
            Some(slot) => *slot = model.clone(),
            None => merged.push(model.clone()),
        }
    }
    merged
}

fn number(entry: &Entry, key: &str) -> Option<f64> {
    entry.get(key).and_then(Value::as_f64)
}

pub fn fetch(
    provider: &str,
    stored: Option<&Entry>,
    force: bool,
    store: &Store,
    cancel: &CancelToken,
) -> Result<Option<Entry>, String> {
    if !force
        && let Some(stored) = stored
        && let Some(checked) = number(stored, "checkedAt")
        && number(stored, "lastModified").is_some()
        && (now_ms() as f64 - checked) < REFRESH_INTERVAL_MS as f64
    {
        return Ok(None);
    }
    let validator = stored
        .filter(|stored| {
            stored
                .get("models")
                .and_then(Value::as_array)
                .is_some_and(|models| !models.is_empty())
        })
        .and_then(|stored| stored.get("etag"))
        .and_then(Value::as_str);
    let url = format!(
        "{}/api/models/providers/{provider}?types={}",
        base_url().trim_end_matches('/'),
        MODEL_TYPES.join("%2C")
    );
    let mut request = Request::new("GET", url)
        .header("accept", "application/json")
        .header("User-Agent", format!("pi-rs/{}", env!("CARGO_PKG_VERSION")));
    if let Some(etag) = validator {
        request = request.header("if-none-match", etag);
    }
    let Some((status, last_modified, etag, body)) = get(&request, cancel)? else {
        return Ok(None);
    };
    let checked_at = json!(now_ms());
    let mut kept = stored.cloned().unwrap_or_else(|| {
        let mut empty = Map::new();
        empty.insert("models".to_owned(), json!([]));
        empty
    });
    if status == 304 && stored.is_some() {
        kept.insert("checkedAt".to_owned(), checked_at);
        store.write(provider, &kept)?;
        return Ok(None);
    }
    if status == 404 || status == 501 {
        kept.insert("checkedAt".to_owned(), checked_at);
        kept.insert("lastModified".to_owned(), json!(0));
        kept.remove("etag");
        store.write(provider, &kept)?;
        return Ok(Some(kept));
    }
    if !(200..300).contains(&status) {
        kept.insert("checkedAt".to_owned(), checked_at);
        store.write(provider, &kept)?;
        return Err(format!(
            "Model catalog request failed for {provider}: {status}"
        ));
    }
    let catalog: Value = serde_json::from_str(&body).map_err(|e| e.to_string())?;
    let mut entry = Map::new();
    entry.insert("models".to_owned(), Value::Array(parse(provider, catalog)?));
    entry.insert("checkedAt".to_owned(), checked_at);
    entry.insert(
        "lastModified".to_owned(),
        json!(last_modified.as_deref().and_then(http_date).unwrap_or(0)),
    );
    if let Some(etag) = etag {
        entry.insert("etag".to_owned(), json!(etag));
    }
    store.write(provider, &entry)?;
    Ok(Some(entry))
}

fn parse(provider: &str, catalog: Value) -> Result<Vec<Value>, String> {
    let entries: Vec<Value> = match catalog {
        Value::Array(entries) => entries,
        Value::Object(mut object) if object.get("models").is_some_and(Value::is_array) => {
            match object.remove("models") {
                Some(Value::Array(entries)) => entries,
                _ => Vec::new(),
            }
        }
        Value::Object(object) => object.into_iter().map(|(_, entry)| entry).collect(),
        _ => {
            return Err(format!("Invalid model catalog for provider \"{provider}\""));
        }
    };
    Ok(entries
        .into_iter()
        .filter_map(|entry| match entry {
            Value::Object(mut model)
                if model.contains_key("id")
                    && match model.get("type") {
                        None => true,
                        Some(Value::String(kind)) => MODEL_TYPES.contains(&kind.as_str()),
                        Some(_) => false,
                    } =>
            {
                model.insert("provider".to_owned(), json!(provider));
                Some(Value::Object(model))
            }
            _ => None,
        })
        .collect())
}

type Fetched = (u16, Option<String>, Option<String>, String);

fn get(request: &Request, cancel: &CancelToken) -> Result<Option<Fetched>, String> {
    let mut attempt = 0;
    loop {
        if cancel.is_cancelled() {
            return Ok(None);
        }
        let token = cancel.child();
        let finished = CancelToken::new();
        {
            let (token, finished) = (token.clone(), finished.clone());
            std::thread::spawn(move || {
                if !finished.wait_timeout(ATTEMPT_TIMEOUT) {
                    token.cancel();
                }
            });
        }
        let result = http::send(request, Some(&token)).and_then(|response| {
            let status = response.status;
            let last_modified = response.header("last-modified").map(str::to_owned);
            let etag = response.header("etag").map(str::to_owned);
            response
                .text()
                .map(|body| (status, last_modified, etag, body))
        });
        let timed_out = token.is_cancelled();
        finished.cancel();
        if cancel.is_cancelled() {
            return Ok(None);
        }
        match result {
            Ok((status, ..)) if RETRYABLE.contains(&status) && attempt < MAX_RETRIES => {}
            Ok(fetched) => return Ok(Some(fetched)),
            Err(_) if attempt < MAX_RETRIES => {}
            Err(_) if timed_out => {
                return Err("The operation was aborted due to timeout".to_owned());
            }
            Err(error) => return Err(error.to_string()),
        }
        attempt += 1;
    }
}

fn http_date(text: &str) -> Option<i64> {
    const MONTHS: [&str; 12] = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    let mut parts = text.split_whitespace();
    parts.next()?;
    let day: u32 = parts.next()?.parse().ok()?;
    let month = parts.next()?;
    let month = MONTHS.iter().position(|name| *name == month)? as u32 + 1;
    let year: i64 = parts.next()?.parse().ok()?;
    let mut clock = parts.next()?.split(':').map(str::parse::<i64>);
    let (hour, minute, second) = (
        clock.next()?.ok()?,
        clock.next()?.ok()?,
        clock.next()?.ok()?,
    );
    (parts.next()? == "GMT").then_some(())?;
    let days = crate::sessions::days_from_civil(year, month, day);
    Some(((days * 24 + hour) * 60 + minute) * 60_000 + second * 1000)
}

fn now_ms() -> i64 {
    pi_durable::ai::oauth::now_ms()
}
