//! `utils/paths.ts` and `core/tools/path-utils.ts`: how tools and the CLI turn a path argument into
//! an absolute path, with Node's `path.resolve` semantics.

use std::path::Path;
use unicode_normalization::UnicodeNormalization;

fn is_unicode_space(c: char) -> bool {
    matches!(
        c,
        '\u{a0}' | '\u{2000}'..='\u{200a}' | '\u{202f}' | '\u{205f}' | '\u{3000}'
    )
}

/// The home directory, `os.homedir()`: `$HOME`, else `/`.
pub fn home_dir() -> String {
    std::env::var("HOME")
        .ok()
        .filter(|h| !h.is_empty())
        .unwrap_or_else(|| "/".to_owned())
}

#[derive(Debug, Clone, Copy, Default)]
pub struct PathInput {
    pub trim: bool,
    pub no_tilde: bool,
    pub strip_at_prefix: bool,
    pub normalize_unicode_spaces: bool,
}

/// The tools' options: unicode spaces become spaces and a leading `@` goes.
pub const TOOL_PATH: PathInput = PathInput {
    trim: false,
    no_tilde: false,
    strip_at_prefix: true,
    normalize_unicode_spaces: true,
};

/// `normalizePath`: `~` expansion, `file://` URLs, and the options above.
pub fn normalize_path(input: &str, options: PathInput) -> String {
    let mut s = if options.trim {
        input.trim().to_owned()
    } else {
        input.to_owned()
    };
    if options.normalize_unicode_spaces {
        s = s
            .chars()
            .map(|c| if is_unicode_space(c) { ' ' } else { c })
            .collect();
    }
    if options.strip_at_prefix
        && let Some(rest) = s.strip_prefix('@')
    {
        s = rest.to_owned();
    }
    if !options.no_tilde {
        let home = home_dir();
        if s == "~" {
            return home;
        }
        if let Some(rest) = s.strip_prefix("~/") {
            return join(&home, rest);
        }
    }
    if let Some(rest) = s.strip_prefix("file://") {
        return percent_decode(rest);
    }
    s
}

fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && i + 2 < bytes.len()
            && let Ok(v) = u8::from_str_radix(&text[i + 1..i + 3], 16)
        {
            out.push(v);
            i += 3;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// `path.normalize` of an absolute POSIX path: no `.`, `..` or repeated `/`, no trailing `/`.
pub fn normalize_absolute(path: &str) -> String {
    let mut parts: Vec<&str> = Vec::new();
    for part in path.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            p => parts.push(p),
        }
    }
    format!("/{}", parts.join("/"))
}

/// `path.join(a, b)` where the result is absolute.
pub fn join(base: &str, rest: &str) -> String {
    if rest.is_empty() {
        return normalize_absolute(base);
    }
    normalize_absolute(&format!("{base}/{rest}"))
}

pub fn within(path: &str, root: &str) -> bool {
    path == root || path.starts_with(&format!("{}/", root.trim_end_matches('/')))
}

/// `path.dirname` of an absolute normalized path.
pub fn dirname(path: &str) -> String {
    match path.rfind('/') {
        Some(0) | None => "/".to_owned(),
        Some(i) => path[..i].to_owned(),
    }
}

/// `path.resolve(base, input)`.
pub fn resolve(base: &str, input: &str) -> String {
    if input.starts_with('/') {
        normalize_absolute(input)
    } else {
        join(base, input)
    }
}

/// `resolvePath(input, baseDir, options)`.
pub fn resolve_path(input: &str, base_dir: &str, options: PathInput) -> String {
    let normalized = normalize_path(input, options);
    let base = normalize_path(base_dir, PathInput::default());
    resolve(&base, &normalized)
}

/// `resolveToCwd`: a tool's path argument made absolute against `cwd`.
pub fn resolve_to_cwd(path: &str, cwd: &str) -> String {
    resolve_path(path, cwd, TOOL_PATH)
}

/// `path.relative(from, to)` for two absolute normalized paths.
pub fn relative(from: &str, to: &str) -> String {
    let a: Vec<&str> = from.split('/').filter(|p| !p.is_empty()).collect();
    let b: Vec<&str> = to.split('/').filter(|p| !p.is_empty()).collect();
    let common = a.iter().zip(&b).take_while(|(x, y)| x == y).count();
    let mut out: Vec<&str> = vec![".."; a.len() - common];
    out.extend(&b[common..]);
    out.join("/")
}

fn exists(path: &str) -> bool {
    Path::new(path).symlink_metadata().is_ok() && Path::new(path).exists()
}

fn narrow_before_meridiem(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    while i < chars.len() {
        let hit = chars[i] == ' '
            && i + 3 < chars.len()
            && chars[i + 3] == '.'
            && matches!(
                (
                    chars[i + 1].to_ascii_lowercase(),
                    chars[i + 2].to_ascii_lowercase()
                ),
                ('a', 'm') | ('p', 'm')
            );
        if hit {
            out.push('\u{202f}');
            out.push(chars[i + 1]);
            out.push(chars[i + 2]);
            out.push('.');
            i += 4;
        } else {
            out.push(chars[i]);
            i += 1;
        }
    }
    out
}

/// `resolveReadPath`: like [`resolve_to_cwd`], then macOS screenshot-name variants when the path
/// does not exist.
pub fn resolve_read_path(path: &str, cwd: &str) -> String {
    let resolved = resolve_to_cwd(path, cwd);
    if exists(&resolved) {
        return resolved;
    }
    let nfd: String = resolved.nfd().collect();
    let variants = [
        narrow_before_meridiem(&resolved),
        nfd.clone(),
        resolved.replace('\'', "\u{2019}"),
        nfd.replace('\'', "\u{2019}"),
    ];
    for variant in variants {
        if variant != resolved && exists(&variant) {
            return variant;
        }
    }
    resolved
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_like_node() {
        assert_eq!(resolve_to_cwd("a/../b/./c/", "/x/y"), "/x/y/b/c");
        assert_eq!(resolve_to_cwd("@/abs//p", "/x"), "/abs/p");
        assert_eq!(resolve_to_cwd("/..", "/x"), "/");
        assert_eq!(relative("/a/b", "/a/c/d"), "../c/d");
        assert_eq!(relative("/a", "/a"), "");
    }
}
