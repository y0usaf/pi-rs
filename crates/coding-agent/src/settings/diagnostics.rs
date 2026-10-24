//! `core/settings-diagnostics.ts`: the load and write failures a session reports.

use super::manager::SettingsManager;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiagnosticKind {
    Warning,
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SettingsDiagnostic {
    pub kind: DiagnosticKind,
    pub message: String,
}

/// `collectSettingsDiagnostics`: each drained error as a warning naming its file.
pub fn collect_diagnostics(manager: &mut SettingsManager) -> Vec<SettingsDiagnostic> {
    manager
        .drain_errors()
        .into_iter()
        .map(|error| SettingsDiagnostic {
            kind: DiagnosticKind::Warning,
            message: match &error.path {
                Some(path) => format!("Invalid settings file {path}: {}", error.error),
                None => format!("Invalid {} settings: {}", error.scope.name(), error.error),
            },
        })
        .collect()
}

/// `deduplicateDiagnostics`: the same kind and message once, in first-seen order.
pub fn deduplicate_diagnostics(diagnostics: &[SettingsDiagnostic]) -> Vec<SettingsDiagnostic> {
    let mut seen: Vec<&SettingsDiagnostic> = Vec::new();
    let mut unique = Vec::new();
    for diagnostic in diagnostics {
        if seen
            .iter()
            .any(|known| known.kind == diagnostic.kind && known.message == diagnostic.message)
        {
            continue;
        }
        seen.push(diagnostic);
        unique.push(diagnostic.clone());
    }
    unique
}
