use crate::types::Model;

pub const GENERATED_AT_MS: i64 = include!("generated_at");

pub fn all() -> Vec<Model> {
    let values: Vec<serde_json::Value> = serde_json::from_str(include_str!("models.json"))
        .unwrap_or_else(|error| panic!("catalog models.json: not a JSON array: {error}"));
    values
        .into_iter()
        .map(|value| {
            let id = value
                .get("id")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("<no id>")
                .to_owned();
            serde_json::from_value(value)
                .unwrap_or_else(|error| panic!("catalog model {id}: {error}"))
        })
        .collect()
}
