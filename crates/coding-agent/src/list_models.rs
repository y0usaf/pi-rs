//! `cli/list-models.ts`: `--list-models`, the models whose provider has auth, as a table.

use crate::models::ModelRegistry;
use crate::paths::join;
use crate::stdout;
use crate::system_prompt;
use pi_durable::ai::types::Model;
use pi_tui::fuzzy::fuzzy_filter;

/// `formatNoModelsAvailableMessage` (`core/auth-guidance.ts`).
pub fn no_models_available() -> String {
    let docs = join(&system_prompt::package_dir(), "docs");
    format!(
        "No models available. Use /login to log into a provider via OAuth or API key. See:\n  {}\n  {}",
        join(&docs, "providers.md"),
        join(&docs, "models.md")
    )
}

/// `formatTokenCount`: 200000 is `200K`, 1000000 is `1M`.
fn token_count(count: u64) -> String {
    let scaled = |value: u64, unit: u64, suffix: &str| {
        let n = value as f64 / unit as f64;
        if n.fract() == 0.0 {
            format!("{n:.0}{suffix}")
        } else {
            format!("{n:.1}{suffix}")
        }
    };
    if count >= 1_000_000 {
        return scaled(count, 1_000_000, "M");
    }
    if count >= 1_000 {
        return scaled(count, 1_000, "K");
    }
    count.to_string()
}

const HEADERS: [&str; 6] = [
    "provider", "model", "context", "max-out", "thinking", "images",
];

/// `listModels`: the table on stdout, sorted by provider then model id.
pub fn list(registry: &ModelRegistry, search: Option<&str>) -> Result<(), String> {
    if let Some(error) = registry.error() {
        eprintln!("Warning: errors loading models.json:\n{error}");
    }
    let models = registry.available();
    if models.is_empty() {
        return stdout::write(&format!("{}\n", no_models_available()));
    }
    let mut models: Vec<&Model> = match search {
        Some(pattern) => fuzzy_filter(&models, pattern, |m| format!("{} {}", m.provider, m.id)),
        None => models,
    };
    if models.is_empty() {
        let message = format!("No models matching \"{}\"\n", search.unwrap_or_default());
        return stdout::write(&message);
    }
    models.sort_by(|a, b| a.provider.cmp(&b.provider).then_with(|| a.id.cmp(&b.id)));
    let yes_no = |value: bool| if value { "yes" } else { "no" };
    let rows: Vec<[String; 6]> = models
        .iter()
        .map(|m| {
            [
                m.provider.clone(),
                m.id.clone(),
                token_count(m.context_window),
                token_count(m.max_tokens),
                yes_no(m.reasoning).to_owned(),
                yes_no(m.input.iter().any(|i| i == "image")).to_owned(),
            ]
        })
        .collect();
    let widths: Vec<usize> = (0..6)
        .map(|column| {
            rows.iter()
                .map(|row| row[column].chars().count())
                .chain([HEADERS[column].len()])
                .max()
                .unwrap_or_default()
        })
        .collect();
    let line = |cells: [&str; 6]| {
        cells
            .iter()
            .enumerate()
            .map(|(column, cell)| format!("{cell:<width$}", width = widths[column]))
            .collect::<Vec<String>>()
            .join("  ")
    };
    let mut out = line(HEADERS);
    out.push('\n');
    for row in &rows {
        out.push_str(&line([
            &row[0], &row[1], &row[2], &row[3], &row[4], &row[5],
        ]));
        out.push('\n');
    }
    stdout::write(&out)
}
