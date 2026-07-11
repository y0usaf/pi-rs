//! Port of `packages/coding-agent/src/utils/paths.ts` — the slice the
//! session manager consumes (`normalizePath`, `resolvePath`,
//! `canonicalizePath`, `isLocalPath`). The cwd-relative display helpers
//! and `markPathIgnoredByCloudSync` land with their consumers (WS3.3
//! tools / WS7 periphery).
//!
//! Node's `path.resolve` is lexical (collapses `.`/`..` without touching
//! the filesystem); [`resolve_path`] reproduces that. Windows path
//! semantics are out of scope until pi-rs targets Windows.

/// Spec: `UNICODE_SPACES` — unicode space variants normalized to ASCII.
fn is_unicode_space(c: char) -> bool {
    matches!(
        c,
        '\u{00A0}' | '\u{2000}'..='\u{200A}' | '\u{202F}' | '\u{205F}' | '\u{3000}'
    )
}

/// Spec: `PathInputOptions`.
#[derive(Clone, Debug)]
pub struct PathInputOptions {
    /// Trim leading/trailing whitespace before normalization.
    pub trim: bool,
    /// Expand a leading `~` to a home directory. Defaults to true.
    pub expand_tilde: bool,
    /// Home directory used for `~` expansion. Defaults to `$HOME`.
    pub home_dir: Option<String>,
    /// Strip a leading `@`, used for CLI @file paths.
    pub strip_at_prefix: bool,
    /// Normalize unicode space variants to regular spaces.
    pub normalize_unicode_spaces: bool,
}

impl Default for PathInputOptions {
    fn default() -> Self {
        Self {
            trim: false,
            expand_tilde: true,
            home_dir: None,
            strip_at_prefix: false,
            normalize_unicode_spaces: false,
        }
    }
}

fn home_dir() -> String {
    std::env::var("HOME").unwrap_or_else(|_| "/".to_owned())
}

/// Node `fileURLToPath` for the `file://` case: strip the scheme (and an
/// empty host) and percent-decode.
fn file_url_to_path(url: &str) -> String {
    let rest = url.trim_start_matches("file://");
    let mut out = Vec::with_capacity(rest.len());
    let bytes = rest.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && i + 2 < bytes.len()
            && let Ok(byte) = u8::from_str_radix(&rest[i + 1..i + 3], 16)
        {
            out.push(byte);
            i += 3;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Spec: `normalizePath(input, options)`.
pub fn normalize_path_with(input: &str, options: &PathInputOptions) -> String {
    let mut normalized: String = if options.trim {
        input.trim().to_owned()
    } else {
        input.to_owned()
    };
    if options.normalize_unicode_spaces {
        normalized = normalized
            .chars()
            .map(|c| if is_unicode_space(c) { ' ' } else { c })
            .collect();
    }
    if options.strip_at_prefix && normalized.starts_with('@') {
        normalized.remove(0);
    }

    if options.expand_tilde {
        let home = options.home_dir.clone().unwrap_or_else(home_dir);
        if normalized == "~" {
            return home;
        }
        if let Some(rest) = normalized.strip_prefix("~/") {
            return join(&home, rest);
        }
    }

    if normalized.starts_with("file://") {
        return file_url_to_path(&normalized);
    }

    normalized
}

/// `normalizePath` with default options.
pub fn normalize_path(input: &str) -> String {
    normalize_path_with(input, &PathInputOptions::default())
}

/// Node `path.join(a, b)` for two segments.
fn join(a: &str, b: &str) -> String {
    if a.ends_with('/') {
        format!("{a}{b}")
    } else {
        format!("{a}/{b}")
    }
}

/// Node `path.resolve(...)` — lexical: make absolute against the base
/// (or the process cwd), then collapse `.`, `..`, and duplicate
/// separators; no trailing slash except for the root itself.
fn node_resolve(base: &str, path: &str) -> String {
    let joined = if path.starts_with('/') {
        path.to_owned()
    } else if base.starts_with('/') {
        join(base, path)
    } else {
        let cwd = std::env::current_dir()
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_else(|_| "/".to_owned());
        join(&join(&cwd, base), path)
    };
    let mut parts: Vec<&str> = Vec::new();
    for part in joined.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            other => parts.push(other),
        }
    }
    format!("/{}", parts.join("/"))
}

/// Spec: `resolvePath(input, baseDir = process.cwd(), options = {})`.
pub fn resolve_path_with(input: &str, base_dir: &str, options: &PathInputOptions) -> String {
    let normalized = normalize_path_with(input, options);
    let normalized_base = normalize_path(base_dir);
    node_resolve(&normalized_base, &normalized)
}

/// `resolvePath` with the process cwd as base and default options.
pub fn resolve_path(input: &str) -> String {
    resolve_path_with(input, ".", &PathInputOptions::default())
}

/// `resolvePath` with an explicit base and default options.
pub fn resolve_path_in(input: &str, base_dir: &str) -> String {
    resolve_path_with(input, base_dir, &PathInputOptions::default())
}

/// Spec: `canonicalizePath` — realpath, falling back to the raw path so
/// callers never crash on missing filesystem entries.
pub fn canonicalize_path(path: &str) -> String {
    std::fs::canonicalize(path)
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|_| path.to_owned())
}

/// Spec: `isLocalPath` — true unless the value is a package source
/// (npm:, git:, …) or remote URL protocol. `file:` URLs are local.
pub fn is_local_path(value: &str) -> bool {
    let trimmed = value.trim();
    !(trimmed.starts_with("npm:")
        || trimmed.starts_with("git:")
        || trimmed.starts_with("github:")
        || trimmed.starts_with("http:")
        || trimmed.starts_with("https:")
        || trimmed.starts_with("ssh:"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tilde_expansion() {
        let opts = PathInputOptions {
            home_dir: Some("/home/u".to_owned()),
            ..Default::default()
        };
        assert_eq!(normalize_path_with("~", &opts), "/home/u");
        assert_eq!(normalize_path_with("~/x", &opts), "/home/u/x");
        assert_eq!(normalize_path_with("/abs", &opts), "/abs");
    }

    #[test]
    fn file_urls() {
        assert_eq!(normalize_path("file:///a%20b/c"), "/a b/c");
    }

    #[test]
    fn lexical_resolve() {
        assert_eq!(resolve_path_in("c", "/a/b"), "/a/b/c");
        assert_eq!(resolve_path_in("../c", "/a/b"), "/a/c");
        assert_eq!(resolve_path_in("/x/./y//z", "/a"), "/x/y/z");
        assert_eq!(resolve_path_in("/", "/a"), "/");
    }

    #[test]
    fn local_path_detection() {
        assert!(is_local_path("./x"));
        assert!(is_local_path("file:///x"));
        assert!(!is_local_path("npm:foo"));
        assert!(!is_local_path("https://example.com"));
    }
}
