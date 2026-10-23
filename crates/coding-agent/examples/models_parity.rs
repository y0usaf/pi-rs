//! Runs `parity/models.json` through this crate's model resolver and prints one JSON line
//! per case, for `parity/run.sh` to compare with upstream's `model-resolver.ts`.
//!
//! Usage: `models_parity CASES.json`

use pi_coding_agent::model_resolver::{resolve_cli_model, resolve_model_scope};
use pi_durable::ai::types::Model;
use serde_json::{Value, json};

fn main() {
    let path = std::env::args()
        .nth(1)
        .expect("usage: models_parity CASES.json");
    let spec: Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    let models: Vec<Model> = spec["models"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| {
            let mut model = Model::new(
                m["provider"].as_str().unwrap(),
                m["id"].as_str().unwrap(),
                "openai-completions",
                1000,
                100,
                false,
            );
            model.name = m["name"].as_str().unwrap().to_string();
            model
        })
        .collect();
    let authed: Vec<String> = serde_json::from_value(spec["authed"].clone()).unwrap();
    let has_auth = |p: &str| authed.iter().any(|a| a == p);

    for c in spec["cli"].as_array().unwrap() {
        let r = resolve_cli_model(
            Value::as_str(&c["provider"]),
            Value::as_str(&c["model"]),
            Value::as_str(&c["thinking"]),
            c["inlineKey"].as_bool().unwrap_or(false),
            &models,
            &has_auth,
        );
        let model = r.model.map(|m| {
            json!({"provider": m.provider, "id": m.id, "name": m.name, "reasoning": m.reasoning})
        });
        println!(
            "{}",
            json!({"case": c, "model": model, "thinking": r.thinking_level, "warning": r.warning, "error": r.error})
        );
    }
    for patterns in spec["scope"].as_array().unwrap() {
        let patterns: Vec<String> = serde_json::from_value(patterns.clone()).unwrap();
        let (scoped, diags) = resolve_model_scope(&patterns, &models);
        let scoped: Vec<Value> = scoped
            .iter()
            .map(|s| json!([s.model.provider, s.model.id, s.thinking_level]))
            .collect();
        let diags: Vec<Value> = diags
            .iter()
            .map(|d| json!([d.code, d.message, d.pattern]))
            .collect();
        println!(
            "{}",
            json!({"case": patterns, "scoped": scoped, "diagnostics": diags})
        );
    }
}
