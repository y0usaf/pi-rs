//! `packages/codemode/src/declarations.ts`: a JSON Schema as a TypeScript type, and a tool's
//! sample (its description and declaration) as a script sees it.

use serde_json::{Map, Value};

const IDENTIFIER: [char; 2] = ['$', '_'];
const INDENT: &str = "  ";
/// Largest rendered input type, in characters, before it becomes `unknown`.
const DEFAULT_INPUT_SCHEMA_MAX_CHARS: usize = 16_000;
/// Local `$ref` expansions per rendered schema.
const MAX_REF_EXPANSIONS: usize = 32;
const TAG_KEYWORDS: [&str; 11] = [
    "minimum",
    "exclusiveMinimum",
    "maximum",
    "exclusiveMaximum",
    "multipleOf",
    "minLength",
    "maxLength",
    "pattern",
    "format",
    "minItems",
    "maxItems",
];
const LOWER_BOUNDS: [&str; 4] = ["minimum", "exclusiveMinimum", "minLength", "minItems"];
const UPPER_BOUNDS: [&str; 4] = ["maximum", "exclusiveMaximum", "maxLength", "maxItems"];
const MAX_TAG_CHARS: usize = 80;

fn is_identifier(name: &str) -> bool {
    let mut chars = name.chars();
    match chars.next() {
        Some(first) if first.is_ascii_alphabetic() || IDENTIFIER.contains(&first) => {}
        _ => return false,
    }
    chars.all(|c| c.is_ascii_alphanumeric() || IDENTIFIER.contains(&c))
}

/// Characters that are not valid in an identifier become `_`.
pub fn to_identifier(name: &str) -> String {
    pi_codemode::protocol::to_identifier(name)
}

fn property_key(name: &str) -> String {
    if is_identifier(name) {
        name.to_owned()
    } else {
        json_string(name)
    }
}

fn json_string(text: &str) -> String {
    Value::String(text.to_owned()).to_string()
}

fn json_or_unknown(value: &Value) -> String {
    value.to_string()
}

fn union(types: Vec<String>) -> String {
    let mut unique: Vec<String> = Vec::new();
    for kind in types {
        if !unique.contains(&kind) {
            unique.push(kind);
        }
    }
    if unique.iter().any(|kind| kind == "unknown") {
        return "unknown".to_owned();
    }
    if unique.is_empty() {
        return "never".to_owned();
    }
    unique.join(" | ")
}

struct Context<'a> {
    root: &'a Value,
    resolving: Vec<String>,
    expansions: usize,
    tags: bool,
}

fn resolve_ref(reference: &str, root: &Value) -> Option<Value> {
    if reference != "#" && !reference.starts_with("#/") {
        return None;
    }
    let mut current = root.clone();
    for segment in reference[2..].split('/').filter(|s| !s.is_empty()) {
        let key = segment.replace("~1", "/").replace("~0", "~");
        match current {
            Value::Object(map) => current = map.get(&key)?.clone(),
            _ => return None,
        }
    }
    match current {
        Value::Bool(_) | Value::Object(_) => Some(current),
        _ => None,
    }
}

pub fn schema_to_type(schema: &Value) -> String {
    schema_to_type_limited(schema, None)
}

fn schema_to_type_limited(schema: &Value, max_chars: Option<usize>) -> String {
    let render = |tags: bool| {
        let mut context = Context {
            root: schema,
            resolving: Vec::new(),
            expansions: 0,
            tags,
        };
        to_type(schema, &mut context)
    };
    let text = render(true);
    match max_chars {
        Some(max) if text.chars().count() > max => {
            let untagged = render(false);
            if untagged.chars().count() <= max {
                untagged
            } else {
                "unknown".to_owned()
            }
        }
        _ => text,
    }
}

fn to_type(schema: &Value, context: &mut Context<'_>) -> String {
    match schema {
        Value::Bool(true) => "unknown".to_owned(),
        Value::Bool(false) => "never".to_owned(),
        Value::Object(map) => to_type_object(map, context),
        _ => "unknown".to_owned(),
    }
}

fn to_type_object(schema: &Map<String, Value>, context: &mut Context<'_>) -> String {
    if let Some(Value::String(reference)) = schema.get("$ref") {
        if context.resolving.contains(reference) || context.expansions >= MAX_REF_EXPANSIONS {
            return "unknown".to_owned();
        }
        let Some(target) = resolve_ref(reference, context.root) else {
            return "unknown".to_owned();
        };
        context.expansions += 1;
        context.resolving.push(reference.clone());
        let rendered = to_type(&target, context);
        context.resolving.pop();
        return rendered;
    }
    if let Some(value) = schema.get("const") {
        return json_or_unknown(value);
    }
    if let Some(Value::Array(values)) = schema.get("enum") {
        return union(values.iter().map(json_or_unknown).collect());
    }
    if let Some(Value::Array(variants)) = schema.get("anyOf").or_else(|| schema.get("oneOf")) {
        return union(variants.iter().map(|v| to_type(v, context)).collect());
    }
    if let Some(Value::Array(parts)) = schema.get("allOf") {
        let parts: Vec<String> = parts
            .iter()
            .map(|part| to_type(part, context))
            .filter(|part| part != "unknown")
            .collect();
        if parts.is_empty() {
            return "unknown".to_owned();
        }
        return parts
            .into_iter()
            .map(|part| {
                if part.contains(" | ") {
                    format!("({part})")
                } else {
                    part
                }
            })
            .collect::<Vec<_>>()
            .join(" & ");
    }
    if let Some(Value::Array(types)) = schema.get("type") {
        return union(
            types
                .iter()
                .map(|kind| {
                    let mut variant = schema.clone();
                    variant.insert("type".to_owned(), kind.clone());
                    to_type_object(&variant, context)
                })
                .collect(),
        );
    }
    match schema.get("type").and_then(Value::as_str) {
        Some("string") => "string".to_owned(),
        Some("number") | Some("integer") => "number".to_owned(),
        Some("boolean") => "boolean".to_owned(),
        Some("null") => "null".to_owned(),
        Some("array") => array_type(schema, context),
        Some("object") => object_type(schema, context),
        Some(_) => "unknown".to_owned(),
        None => {
            if schema.contains_key("properties")
                || schema.contains_key("additionalProperties")
                || schema.contains_key("required")
            {
                return object_type(schema, context);
            }
            if schema.contains_key("items") || schema.contains_key("prefixItems") {
                return array_type(schema, context);
            }
            "unknown".to_owned()
        }
    }
}

fn array_type(schema: &Map<String, Value>, context: &mut Context<'_>) -> String {
    if let Some(items) = schema.get("items")
        && !items.is_array()
    {
        return format!("Array<{}>", to_type(items, context));
    }
    let tuple: Vec<Value> = match (schema.get("prefixItems"), schema.get("items")) {
        (Some(Value::Array(items)), _) => items.clone(),
        (_, Some(Value::Array(items))) => items.clone(),
        _ => Vec::new(),
    };
    if tuple.is_empty() {
        return "unknown[]".to_owned();
    }
    format!(
        "[{}]",
        tuple
            .iter()
            .map(|item| to_type(item, context))
            .collect::<Vec<_>>()
            .join(", ")
    )
}

fn description_of(property: Option<&Value>) -> String {
    property
        .and_then(|property| property.get("description"))
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or_default()
        .to_owned()
}

fn number_text(value: f64) -> String {
    if value.fract() == 0.0 && value.abs() < 1e21 {
        format!("{}", value as i64)
    } else {
        format!("{value}")
    }
}

/// Constraints the type cannot express, as `@integer @minimum 1 @default 20`.
fn constraint_tags(property: Option<&Value>) -> String {
    let Some(Value::Object(property)) = property else {
        return String::new();
    };
    let non_null: Vec<Value> = property
        .get("anyOf")
        .or_else(|| property.get("oneOf"))
        .and_then(Value::as_array)
        .map(|list| {
            list.iter()
                .filter(|variant| variant.get("type").and_then(Value::as_str) != Some("null"))
                .cloned()
                .collect()
        })
        .unwrap_or_default();
    let branches: Vec<Map<String, Value>> = if non_null.len() == 1 {
        match &non_null[0] {
            Value::Object(branch) => vec![property.clone(), branch.clone()],
            _ => vec![property.clone()],
        }
    } else {
        vec![property.clone()]
    };
    let schemas: Vec<Map<String, Value>> = branches
        .into_iter()
        .map(|mut schema| {
            if schema.get("exclusiveMinimum") == Some(&Value::Bool(true)) {
                let minimum = schema.get("minimum").cloned();
                schema.remove("minimum");
                if let Some(minimum) = minimum {
                    schema.insert("exclusiveMinimum".to_owned(), minimum);
                }
            }
            if schema.get("exclusiveMaximum") == Some(&Value::Bool(true)) {
                let maximum = schema.get("maximum").cloned();
                schema.remove("maximum");
                if let Some(maximum) = maximum {
                    schema.insert("exclusiveMaximum".to_owned(), maximum);
                }
            }
            schema
        })
        .collect();
    let is_integer = |kind: Option<&Value>| match kind {
        Some(Value::String(kind)) => kind == "integer",
        Some(Value::Array(kinds)) => kinds.iter().any(|kind| kind == "integer"),
        _ => false,
    };
    let mut tags: Vec<String> = Vec::new();
    if schemas.iter().any(|schema| is_integer(schema.get("type"))) {
        tags.push("@integer".to_owned());
    }
    for keyword in TAG_KEYWORDS {
        let values: Vec<Option<&Value>> =
            schemas.iter().map(|schema| schema.get(keyword)).collect();
        if LOWER_BOUNDS.contains(&keyword) || UPPER_BOUNDS.contains(&keyword) {
            let numbers: Vec<f64> = values
                .iter()
                .filter_map(|value| value.and_then(Value::as_f64))
                .collect();
            if numbers.is_empty() {
                continue;
            }
            let picked = if LOWER_BOUNDS.contains(&keyword) {
                numbers.iter().cloned().fold(f64::NEG_INFINITY, f64::max)
            } else {
                numbers.iter().cloned().fold(f64::INFINITY, f64::min)
            };
            tags.push(format!("@{keyword} {}", number_text(picked)));
            continue;
        }
        let mut seen: Vec<String> = Vec::new();
        for value in values.into_iter().flatten() {
            let text = match value {
                Value::Number(number) => Some(number_text(number.as_f64().unwrap_or_default())),
                Value::String(text) => Some(text.clone()),
                _ => None,
            };
            if let Some(text) = text
                && !seen.contains(&text)
            {
                seen.push(text.clone());
                tags.push(format!("@{keyword} {text}"));
            }
        }
    }
    if schemas
        .iter()
        .any(|schema| schema.get("uniqueItems") == Some(&Value::Bool(true)))
    {
        tags.push("@uniqueItems".to_owned());
    }
    if let Some(default) = property.get("default")
        && !default.is_null()
    {
        tags.push(format!("@default {}", json_or_unknown(default)));
    }
    if property.get("deprecated") == Some(&Value::Bool(true)) {
        tags.push("@deprecated".to_owned());
    }
    tags.iter()
        .filter(|tag| tag.chars().count() <= MAX_TAG_CHARS)
        .map(|tag| {
            tag.chars()
                .map(|c| match c {
                    '\r' => "\\u000d".to_owned(),
                    '\n' => "\\u000a".to_owned(),
                    '\u{2028}' => "\\u2028".to_owned(),
                    '\u{2029}' => "\\u2029".to_owned(),
                    other => other.to_string(),
                })
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn object_type(schema: &Map<String, Value>, context: &mut Context<'_>) -> String {
    let properties: Map<String, Value> = schema
        .get("properties")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    let required: Vec<String> = schema
        .get("required")
        .and_then(Value::as_array)
        .map(|list| {
            list.iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default();
    let mut names: Vec<String> = properties.keys().cloned().collect();
    names.sort();
    let mut members: Vec<String> = names
        .iter()
        .map(|name| {
            let optional = if required.contains(name) { "" } else { "?" };
            format!(
                "{}{optional}: {};",
                property_key(name),
                to_type(&properties[name], context)
            )
        })
        .collect();
    match schema.get("additionalProperties") {
        Some(Value::Bool(false)) => {}
        Some(Value::Bool(true)) => members.push("[key: string]: unknown;".to_owned()),
        Some(other) => members.push(format!("[key: string]: {};", to_type(other, context))),
        None if names.is_empty() => members.push("[key: string]: unknown;".to_owned()),
        None => {}
    }
    if members.is_empty() {
        return "{}".to_owned();
    }
    let comments: Vec<Vec<String>> = names
        .iter()
        .map(|name| {
            let mut lines: Vec<String> = description_of(properties.get(name))
                .split(['\r', '\n', '\u{2028}', '\u{2029}'])
                .map(|line| line.trim().to_owned())
                .filter(|line| !line.is_empty())
                .collect();
            if context.tags {
                let tags = constraint_tags(properties.get(name));
                if !tags.is_empty() {
                    lines.push(tags);
                }
            }
            lines
        })
        .collect();
    if !comments.iter().any(|comment| !comment.is_empty()) {
        return format!("{{ {} }}", members.join(" "));
    }
    let mut lines = vec!["{".to_owned()];
    for (index, comment) in comments.iter().enumerate() {
        for line in comment {
            lines.push(format!("{INDENT}// {line}"));
        }
        lines.push(format!(
            "{INDENT}{}",
            members[index].replace('\n', &format!("\n{INDENT}"))
        ));
    }
    for member in &members[names.len()..] {
        lines.push(format!("{INDENT}{member}"));
    }
    lines.push("}".to_owned());
    lines.join("\n")
}

/// The type a tool call resolves to: the output schema's type, `unknown` without one.
pub fn render_output_type(schema: Option<&Value>) -> String {
    match schema {
        Some(schema) => schema_to_type(schema),
        None => "unknown".to_owned(),
    }
}

/// `name(args: T): Promise<R>;` with the identifier a script uses.
pub fn render_signature(name: &str, input: Option<&Value>, output: Option<&Value>) -> String {
    let input = match input {
        Some(schema) => schema_to_type_limited(schema, Some(DEFAULT_INPUT_SCHEMA_MAX_CHARS)),
        None => "unknown".to_owned(),
    };
    format!(
        "{}(args: {input}): Promise<{}>;",
        to_identifier(name),
        render_output_type(output)
    )
}

/// A tool's sample: its description followed by the declaration, as `ALL_TOOLS` and the codemode
/// description show it.
pub fn render_sample(
    name: &str,
    description: Option<&str>,
    input: Option<&Value>,
    output: Option<&Value>,
) -> String {
    let declaration = format!(
        "declare const tools: {{ {} }};",
        render_signature(name, input, output)
    );
    let fence = "```";
    format!(
        "{}\n\ncodemode tool declaration:\n{fence}ts\n{declaration}\n{fence}",
        description.unwrap_or_default().trim()
    )
}
