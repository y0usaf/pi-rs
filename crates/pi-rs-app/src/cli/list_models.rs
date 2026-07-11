//! Port of `cli/list-models.ts` — list available models with optional
//! fuzzy search.

use pi_rs_ai_types::{Modality, Model};
use pi_rs_tui::fuzzy::fuzzy_filter;

use crate::core::auth_guidance::format_no_models_available_message;
use crate::core::model_registry::ModelRegistry;

/// Spec: `formatTokenCount` — `200000` → `"200K"`, `1000000` → `"1M"`.
pub fn format_token_count(count: u64) -> String {
    if count >= 1_000_000 {
        let millions = count as f64 / 1_000_000.0;
        return if millions.fract() == 0.0 {
            format!("{}M", millions as u64)
        } else {
            format!("{millions:.1}M")
        };
    }
    if count >= 1_000 {
        let thousands = count as f64 / 1_000.0;
        return if thousands.fract() == 0.0 {
            format!("{}K", thousands as u64)
        } else {
            format!("{thousands:.1}K")
        };
    }
    count.to_string()
}

struct Row {
    provider: String,
    model: String,
    context: String,
    max_out: String,
    thinking: &'static str,
    images: &'static str,
}

const HEADERS: [&str; 6] = [
    "provider", "model", "context", "max-out", "thinking", "images",
];

/// Spec: `listModels(modelRegistry, searchPattern?)` — returns the
/// rendered output; the caller prints (stdout) and reports the
/// models.json load warning (stderr).
pub fn render_model_list(
    model_registry: &ModelRegistry,
    auth_storage: &crate::core::auth_storage::AuthStorage,
    search_pattern: Option<&str>,
) -> String {
    let models: Vec<&Model> = model_registry.get_available(auth_storage);

    if models.is_empty() {
        return format_no_models_available_message();
    }

    // Apply fuzzy filter if search pattern provided.
    let mut filtered_models = match search_pattern {
        Some(pattern) => fuzzy_filter(models, pattern, |m| format!("{} {}", m.provider, m.id)),
        None => models,
    };

    if filtered_models.is_empty() {
        // `search_pattern` is non-empty here: an empty/blank pattern
        // returns the input unfiltered.
        return format!(
            "No models matching \"{}\"",
            search_pattern.unwrap_or_default()
        );
    }

    // Sort by provider, then by model id.
    filtered_models.sort_by(|a, b| a.provider.cmp(&b.provider).then_with(|| a.id.cmp(&b.id)));

    let rows: Vec<Row> = filtered_models
        .iter()
        .map(|m| Row {
            provider: m.provider.clone(),
            model: m.id.clone(),
            context: format_token_count(m.context_window),
            max_out: format_token_count(m.max_tokens),
            thinking: if m.reasoning { "yes" } else { "no" },
            images: if m.input.contains(&Modality::Image) {
                "yes"
            } else {
                "no"
            },
        })
        .collect();

    let width = |header: &str, get: fn(&Row) -> usize| -> usize {
        rows.iter()
            .map(get)
            .chain(std::iter::once(header.len()))
            .max()
            .unwrap_or(header.len())
    };
    let widths = [
        width(HEADERS[0], |r| r.provider.len()),
        width(HEADERS[1], |r| r.model.len()),
        width(HEADERS[2], |r| r.context.len()),
        width(HEADERS[3], |r| r.max_out.len()),
        width(HEADERS[4], |r| r.thinking.len()),
        width(HEADERS[5], |r| r.images.len()),
    ];

    let render_line = |cells: [&str; 6]| -> String {
        cells
            .iter()
            .zip(widths.iter())
            .map(|(cell, width)| format!("{cell:<width$}"))
            .collect::<Vec<_>>()
            .join("  ")
    };

    let mut out = String::new();
    out.push_str(&render_line(HEADERS));
    for row in &rows {
        out.push('\n');
        out.push_str(&render_line([
            &row.provider,
            &row.model,
            &row.context,
            &row.max_out,
            row.thinking,
            row.images,
        ]));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::auth_storage::{AuthCredential, AuthStorage, AuthStorageData};

    #[test]
    fn token_count_formatting_matches_spec() {
        assert_eq!(format_token_count(200_000), "200K");
        assert_eq!(format_token_count(1_000_000), "1M");
        assert_eq!(format_token_count(1_500_000), "1.5M");
        assert_eq!(format_token_count(12_800), "12.8K");
        assert_eq!(format_token_count(999), "999");
    }

    fn registry() -> (AuthStorage, ModelRegistry) {
        let mut data = AuthStorageData::new();
        data.insert(
            "anthropic".to_owned(),
            AuthCredential::ApiKey { key: "sk".into() },
        );
        let storage = AuthStorage::in_memory(data);
        let registry = ModelRegistry::new(&storage);
        (storage, registry)
    }

    #[test]
    fn renders_header_and_anthropic_rows() {
        let (storage, registry) = registry();
        let output = render_model_list(&registry, &storage, None);
        let mut lines = output.lines();
        let header = lines.next().unwrap_or_default();
        assert!(header.starts_with("provider"));
        assert!(header.contains("model"));
        assert!(header.contains("max-out"));
        assert!(output.lines().any(|l| l.contains("claude-opus-4-8")));
    }

    #[test]
    fn fuzzy_pattern_filters() {
        // The table holds exactly the fuzzy-filtered rows (fuzzy
        // semantics themselves are pinned in pi-rs-tui).
        let (storage, registry) = registry();
        let output = render_model_list(&registry, &storage, Some("opus"));
        assert!(output.lines().any(|l| l.contains("claude-opus-4-8")));
        let expected = fuzzy_filter(registry.get_available(&storage), "opus", |m| {
            format!("{} {}", m.provider, m.id)
        });
        assert_eq!(output.lines().count(), expected.len() + 1);
        let none = render_model_list(&registry, &storage, Some("zzzzzz"));
        assert_eq!(none, "No models matching \"zzzzzz\"");
    }

    #[test]
    fn no_auth_means_no_models_message() {
        let storage = AuthStorage::in_memory(AuthStorageData::new());
        let registry = ModelRegistry::new(&storage);
        // Only meaningful without ambient env keys (CI sandbox).
        if registry.get_available(&storage).is_empty() {
            assert!(
                render_model_list(&registry, &storage, None).starts_with("No models available.")
            );
        }
    }
}
