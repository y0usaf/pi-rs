//! JSON-Schema argument coercion/validation used by registered tools.
//! Mirrors pi's `validateToolArguments` practiced plain-schema behavior;
//! error lines match typebox's en_US locale strings and error order
//! (aggregated `required` before per-property `type` errors), pinned by
//! tests/agent-parity.

use serde_json::Value;

#[derive(Debug, thiserror::Error)]
#[error("Validation failed for tool \"{tool}\":\n{details}\n\nReceived arguments:\n{received}")]
pub(crate) struct SchemaError {
    tool: String,
    details: String,
    received: String,
}

pub(crate) fn validate_tool_arguments(
    tool: &str,
    schema: &Value,
    arguments: &Value,
) -> Result<Value, SchemaError> {
    let mut value = arguments.clone();
    let mut errors = Vec::new();
    coerce_and_validate(schema, &mut value, "root", &mut errors);
    if errors.is_empty() {
        return Ok(value);
    }
    let received =
        serde_json::to_string_pretty(arguments).unwrap_or_else(|_| arguments.to_string());
    Err(SchemaError {
        tool: tool.to_owned(),
        details: errors
            .into_iter()
            .map(|error| format!("  - {error}"))
            .collect::<Vec<_>>()
            .join("\n"),
        received,
    })
}

fn coerce_and_validate(schema: &Value, value: &mut Value, path: &str, errors: &mut Vec<String>) {
    let Some(object) = schema.as_object() else {
        return;
    };
    if let Some(types) = object.get("type") {
        let accepted = match types {
            Value::String(kind) => vec![kind.as_str()],
            Value::Array(kinds) => kinds.iter().filter_map(Value::as_str).collect(),
            _ => Vec::new(),
        };
        if accepted.len() == 1 {
            coerce(value, accepted[0]);
        }
        if !accepted.is_empty() && !accepted.iter().any(|kind| matches_type(value, kind)) {
            let message = if accepted.len() == 1 {
                format!("must be {}", accepted[0])
            } else {
                format!("must be either {}", accepted.join(" or "))
            };
            errors.push(format!("{path}: {message}"));
            return;
        }
    }
    if let Some(map) = value.as_object_mut() {
        if let Some(required) = object.get("required").and_then(Value::as_array) {
            // typebox emits one aggregated `required` error per object,
            // addressed at the first missing property (formatValidationPath).
            let missing: Vec<&str> = required
                .iter()
                .filter_map(Value::as_str)
                .filter(|name| !map.contains_key(*name))
                .collect();
            if let Some(first) = missing.first() {
                errors.push(format!(
                    "{}: must have required properties {}",
                    child_path(path, first),
                    missing.join(", ")
                ));
            }
        }
        if let Some(properties) = object.get("properties").and_then(Value::as_object) {
            for (name, property_schema) in properties {
                if let Some(child) = map.get_mut(name) {
                    coerce_and_validate(property_schema, child, &child_path(path, name), errors);
                }
            }
        }
    }
    if let (Some(items), Some(values)) = (object.get("items"), value.as_array_mut()) {
        for (index, child) in values.iter_mut().enumerate() {
            coerce_and_validate(items, child, &child_path(path, &index.to_string()), errors);
        }
    }
}

fn child_path(path: &str, child: &str) -> String {
    if path == "root" {
        child.to_owned()
    } else {
        format!("{path}.{child}")
    }
}

fn matches_type(value: &Value, kind: &str) -> bool {
    match kind {
        "object" => value.is_object(),
        "array" => value.is_array(),
        "string" => value.is_string(),
        "number" => value.is_number(),
        "integer" => value.as_i64().is_some() || value.as_u64().is_some(),
        "boolean" => value.is_boolean(),
        "null" => value.is_null(),
        _ => true,
    }
}

fn coerce(value: &mut Value, kind: &str) {
    let replacement = match kind {
        // JSON→Lua→JSON cannot distinguish an empty array from an empty
        // object. The schema supplies that missing type information.
        "array" if value.as_object().is_some_and(serde_json::Map::is_empty) => {
            Some(Value::Array(Vec::new()))
        }
        "number" | "integer" => match value {
            Value::String(text) => text.parse::<serde_json::Number>().ok().map(Value::Number),
            Value::Bool(flag) => Some(Value::from(if *flag { 1 } else { 0 })),
            Value::Null => Some(Value::from(0)),
            _ => None,
        },
        "boolean" => match value {
            Value::String(text) if text == "true" => Some(Value::Bool(true)),
            Value::String(text) if text == "false" => Some(Value::Bool(false)),
            Value::Number(number) if number.as_i64() == Some(1) => Some(Value::Bool(true)),
            Value::Number(number) if number.as_i64() == Some(0) => Some(Value::Bool(false)),
            _ => None,
        },
        "string" => match value {
            Value::Null => Some(Value::String(String::new())),
            Value::Bool(flag) => Some(Value::String(flag.to_string())),
            _ => None,
        },
        "null" => match value {
            Value::String(text) if text.is_empty() => Some(Value::Null),
            Value::Number(number) if number.as_i64() == Some(0) => Some(Value::Null),
            Value::Bool(false) => Some(Value::Null),
            _ => None,
        },
        _ => None,
    };
    if let Some(replacement) = replacement {
        *value = replacement;
    }
}
