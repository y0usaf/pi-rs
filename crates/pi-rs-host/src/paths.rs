//! Host-side path helpers — port of the slice of the spec's
//! `utils/paths.ts` the substrate needs (`normalizePath`, `resolvePath`,
//! `canonicalizePath`). The Lua-visible `pi.path.*` bindings live in
//! `os.rs`; these run on the host side of the bridge (extension
//! discovery, trust store).
//!
//! Omitted from the port until a caller needs them: `trim`,
//! `stripAtPrefix` (CLI `@file` handling — arrives with the cli args
//! port) and `file://` URL resolution.

/// Spec `UNICODE_SPACES`: `/[\u00A0\u2000-\u200A\u202F\u205F\u3000]/`.
fn is_unicode_space(c: char) -> bool {
    matches!(
        c,
        '\u{00A0}' | '\u{2000}'..='\u{200A}' | '\u{202F}' | '\u{205F}' | '\u{3000}'
    )
}

pub(crate) fn home_dir() -> String {
    std::env::var("HOME").unwrap_or_else(|_| "/".to_owned())
}

pub(crate) fn process_cwd() -> String {
    std::env::current_dir()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|_| ".".to_owned())
}

/// Spec `normalizePath`: optional unicode-space normalization, then `~`
/// expansion (always on — the spec's default).
pub(crate) fn normalize_path(input: &str, normalize_unicode_spaces: bool) -> String {
    let normalized: String = if normalize_unicode_spaces {
        input
            .chars()
            .map(|c| if is_unicode_space(c) { ' ' } else { c })
            .collect()
    } else {
        input.to_owned()
    };
    if normalized == "~" {
        return home_dir();
    }
    if let Some(rest) = normalized.strip_prefix("~/") {
        return crate::os::join(&[home_dir(), rest.to_owned()]);
    }
    normalized
}

/// Spec `resolvePath`: normalize input and base, then Node
/// `path.resolve(base, input)` (absolute input wins).
pub(crate) fn resolve_path(input: &str, base_dir: &str, normalize_unicode_spaces: bool) -> String {
    let normalized = normalize_path(input, normalize_unicode_spaces);
    let base = normalize_path(base_dir, false);
    crate::os::resolve(&[base, normalized], &process_cwd())
}

/// Spec `canonicalizePath`: realpath, falling back to the raw path when
/// the target does not exist — callers never crash on missing entries.
pub(crate) fn canonicalize_path(path: &str) -> String {
    std::fs::canonicalize(path)
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|_| path.to_owned())
}
