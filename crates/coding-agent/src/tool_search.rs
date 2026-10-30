use serde::Deserialize;
use serde_json::{Value, json};

pub const TOOL_NAME: &str = "tool_search";
pub const SNIPPET: &str = "Search for tools that are not loaded yet and load the matches";
const DEFAULT_LIMIT: f64 = 8.0;

const STOP_WORDS: [&str; 21] = [
    "a", "an", "and", "are", "as", "at", "be", "by", "for", "from", "in", "is", "it", "of", "on",
    "or", "that", "the", "this", "to", "with",
];

pub fn description() -> String {
    format!(
        "# Tool discovery\n\nSearches over deferred tool metadata with BM25 and exposes matching tools for the next model call.\n\nSome of the tools, such as tools of MCP servers, may not have been provided to you upfront, and you should use this tool (`{TOOL_NAME}`) to search for the required tools. For MCP tool discovery, always use `{TOOL_NAME}`."
    )
}

pub fn parameters() -> Value {
    json!({
        "type": "object",
        "required": ["query"],
        "properties": {
            "query": {"type": "string", "description": "Search query for deferred tools."},
            "limit": {
                "type": "number",
                "description": format!("Maximum number of tools to return. Defaults to {DEFAULT_LIMIT}.")
            }
        }
    })
}

#[derive(Deserialize)]
pub struct Args {
    query: String,
    #[serde(default)]
    limit: Option<f64>,
}

impl Args {
    pub fn limit(&self) -> Result<usize, String> {
        let max = self.limit.unwrap_or(DEFAULT_LIMIT);
        if max.fract() != 0.0 || !max.is_finite() || max <= 0.0 {
            return Err("limit must be a positive integer".to_owned());
        }
        Ok(max.min(usize::MAX as f64) as usize)
    }

    pub fn query(&self) -> Result<&str, String> {
        if self.query.trim().is_empty() {
            return Err("query must not be empty".to_owned());
        }
        Ok(&self.query)
    }
}

pub struct Candidate<'a> {
    pub name: &'a str,
    pub description: &'a str,
    pub parameters: &'a Value,
}

fn stem(term: &str) -> String {
    let n = term.len();
    if n > 4 && term.ends_with("ies") {
        return format!("{}y", &term[..n - 3]);
    }
    if n > 4
        && ["ches", "shes", "sses", "xes", "zes"]
            .iter()
            .any(|end| term.ends_with(end))
    {
        return term[..n - 2].to_owned();
    }
    if n > 3 && term.ends_with('s') && !term.ends_with("ss") {
        return term[..n - 1].to_owned();
    }
    term.to_owned()
}

fn split_camel(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    for (index, &c) in chars.iter().enumerate() {
        if index > 0 && c.is_ascii_uppercase() {
            let before = chars[index - 1];
            let lower_then_upper = before.is_ascii_lowercase() || before.is_ascii_digit();
            let acronym_end = before.is_ascii_uppercase()
                && chars.get(index + 1).is_some_and(char::is_ascii_lowercase);
            if lower_then_upper || acronym_end {
                out.push(' ');
            }
        }
        out.push(c);
    }
    out
}

pub fn tokenize(text: &str) -> Vec<String> {
    split_camel(text)
        .to_lowercase()
        .split(|c: char| !(c.is_ascii_lowercase() || c.is_ascii_digit()))
        .filter(|term| !term.is_empty() && !STOP_WORDS.contains(term))
        .map(stem)
        .collect()
}

fn schema_text(schema: &Value, parts: &mut Vec<String>) {
    let Value::Object(fields) = schema else {
        return;
    };
    if let Some(Value::String(description)) = fields.get("description") {
        parts.push(description.clone());
    }
    if let Some(Value::Object(properties)) = fields.get("properties") {
        for (name, property) in properties {
            parts.push(name.clone());
            schema_text(property, parts);
        }
    }
    if let Some(items) = fields.get("items") {
        schema_text(items, parts);
    }
    for key in ["anyOf", "oneOf", "allOf"] {
        if let Some(Value::Array(variants)) = fields.get(key) {
            for variant in variants {
                schema_text(variant, parts);
            }
        }
    }
}

pub fn document(tool: &Candidate<'_>) -> String {
    let mut parts = vec![
        tool.name.to_owned(),
        tool.name.replace('_', " "),
        tool.description.to_owned(),
    ];
    schema_text(tool.parameters, &mut parts);
    parts
        .into_iter()
        .filter(|part| !part.trim().is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

pub fn rank(query: &str, documents: &[String], limit: usize) -> Vec<usize> {
    let mut terms: Vec<String> = Vec::new();
    for term in tokenize(query) {
        if !terms.contains(&term) {
            terms.push(term);
        }
    }
    if terms.is_empty() || documents.is_empty() || limit == 0 {
        return Vec::new();
    }
    let (k1, b) = (1.2, 0.75);
    let counts: Vec<std::collections::HashMap<String, f64>> = documents
        .iter()
        .map(|document| {
            let mut counts = std::collections::HashMap::new();
            for term in tokenize(document) {
                *counts.entry(term).or_insert(0.0) += 1.0;
            }
            counts
        })
        .collect();
    let lengths: Vec<f64> = counts.iter().map(|c| c.values().sum()).collect();
    let total = documents.len() as f64;
    let average = match lengths.iter().sum::<f64>() / total {
        average if average != 0.0 && average.is_finite() => average,
        _ => 1.0,
    };
    let idf: Vec<f64> = terms
        .iter()
        .map(|term| {
            let frequency = counts.iter().filter(|c| c.contains_key(term)).count() as f64;
            (1.0 + (total - frequency + 0.5) / (frequency + 0.5)).ln()
        })
        .collect();
    let mut matches: Vec<(usize, f64)> = Vec::new();
    for (index, document) in counts.iter().enumerate() {
        let mut score = 0.0;
        for (term, idf) in terms.iter().zip(&idf) {
            let Some(&count) = document.get(term) else {
                continue;
            };
            let norm = k1 * (1.0 - b + b * lengths[index] / average);
            score += idf * (count * (k1 + 1.0)) / (count + norm);
        }
        if score > 0.0 {
            matches.push((index, score));
        }
    }
    matches.sort_by(|x, y| y.1.total_cmp(&x.1));
    matches
        .into_iter()
        .take(limit)
        .map(|(index, _)| index)
        .collect()
}

pub fn result(loaded: &[(String, String)]) -> (String, Value) {
    let text = if loaded.is_empty() {
        "No matching tools found.".to_owned()
    } else {
        let lines: Vec<String> = loaded
            .iter()
            .map(|(name, description)| {
                let first = description.trim().split('\n').next().unwrap_or_default();
                format!("- {name}: {}", first.strip_suffix('\r').unwrap_or(first))
            })
            .collect();
        format!(
            "Loaded {} tool{}. They are available from your next call:\n{}",
            loaded.len(),
            if loaded.len() == 1 { "" } else { "s" },
            lines.join("\n")
        )
    };
    let names: Vec<&str> = loaded.iter().map(|(name, _)| name.as_str()).collect();
    (text, json!({ "loaded": names }))
}
