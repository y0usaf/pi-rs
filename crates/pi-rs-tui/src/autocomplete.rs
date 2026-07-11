//! 1:1 port of `packages/tui/src/autocomplete.ts` — the
//! `CombinedAutocompleteProvider` (slash commands with argument completions,
//! file paths, @-fuzzy search via fd) plus `applyCompletion`.
//!
//! Cursor columns are UTF-8 byte offsets (the editor's convention); the spec
//! indexes UTF-16 units — identical for the ASCII command names and paths in
//! scope. `localeCompare` is approximated case-insensitively (lowercase
//! first on ties), which matches ICU for the ASCII file names in scope.

use std::cmp::Ordering;
use std::fs;
use std::process::Command;

use crate::fuzzy::fuzzy_filter;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AutocompleteItem {
    pub value: String,
    pub label: String,
    pub description: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Suggestions {
    pub items: Vec<AutocompleteItem>,
    pub prefix: String,
}

/// Spec: `SlashCommand`. Argument completions run through a host callback so
/// command policy (e.g. `/model`) stays with the integration.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SlashCommand {
    pub name: String,
    pub description: Option<String>,
    pub argument_hint: Option<String>,
    pub has_argument_completions: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Applied {
    pub lines: Vec<String>,
    pub cursor_line: usize,
    pub cursor_col: usize,
}

fn is_path_delimiter(c: char) -> bool {
    matches!(c, ' ' | '\t' | '"' | '\'' | '=')
}

fn to_display_path(value: &str) -> String {
    value.replace('\\', "/")
}

/// Spec: `escapeRegex` — `[.*+?^${}()|[\]\\]` prefixed with a backslash.
fn escape_regex(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for ch in value.chars() {
        if matches!(
            ch,
            '.' | '*' | '+' | '?' | '^' | '$' | '{' | '}' | '(' | ')' | '|' | '[' | ']' | '\\'
        ) {
            out.push('\\');
        }
        out.push(ch);
    }
    out
}

/// Spec: `buildFdPathQuery`.
fn build_fd_path_query(query: &str) -> String {
    let normalized = to_display_path(query);
    if !normalized.contains('/') {
        return normalized;
    }
    let has_trailing_separator = normalized.ends_with('/');
    let trimmed = normalized.trim_matches('/');
    if trimmed.is_empty() {
        return normalized;
    }
    let separator_pattern = "[\\\\/]";
    let segments: Vec<String> = trimmed
        .split('/')
        .filter(|s| !s.is_empty())
        .map(escape_regex)
        .collect();
    if segments.is_empty() {
        return normalized;
    }
    let mut pattern = segments.join(separator_pattern);
    if has_trailing_separator {
        pattern.push_str(separator_pattern);
    }
    pattern
}

/// Byte index of the last path delimiter, or `None`.
fn find_last_delimiter(text: &str) -> Option<usize> {
    text.char_indices()
        .rev()
        .find(|(_, c)| is_path_delimiter(*c))
        .map(|(i, _)| i)
}

/// Spec: `findUnclosedQuoteStart` — byte index of the opening `"` when the
/// text has an odd number of double quotes.
fn find_unclosed_quote_start(text: &str) -> Option<usize> {
    let mut in_quotes = false;
    let mut quote_start = None;
    for (i, c) in text.char_indices() {
        if c == '"' {
            in_quotes = !in_quotes;
            if in_quotes {
                quote_start = Some(i);
            }
        }
    }
    if in_quotes { quote_start } else { None }
}

fn is_token_start(text: &str, index: usize) -> bool {
    index == 0
        || text[..index]
            .chars()
            .next_back()
            .is_some_and(is_path_delimiter)
}

/// Spec: `extractQuotedPrefix`.
fn extract_quoted_prefix(text: &str) -> Option<&str> {
    let quote_start = find_unclosed_quote_start(text)?;
    if quote_start > 0 && text[..quote_start].ends_with('@') {
        let at_start = quote_start - 1;
        if !is_token_start(text, at_start) {
            return None;
        }
        return Some(&text[at_start..]);
    }
    if !is_token_start(text, quote_start) {
        return None;
    }
    Some(&text[quote_start..])
}

/// Spec: `parsePathPrefix` — `(rawPrefix, isAtPrefix, isQuotedPrefix)`.
fn parse_path_prefix(prefix: &str) -> (&str, bool, bool) {
    if let Some(rest) = prefix.strip_prefix("@\"") {
        (rest, true, true)
    } else if let Some(rest) = prefix.strip_prefix('"') {
        (rest, false, true)
    } else if let Some(rest) = prefix.strip_prefix('@') {
        (rest, true, false)
    } else {
        (prefix, false, false)
    }
}

/// Spec: `buildCompletionValue`.
fn build_completion_value(path: &str, is_at_prefix: bool, is_quoted_prefix: bool) -> String {
    let needs_quotes = is_quoted_prefix || path.contains(' ');
    let prefix = if is_at_prefix { "@" } else { "" };
    if !needs_quotes {
        return format!("{prefix}{path}");
    }
    format!("{prefix}\"{path}\"")
}

/// Node `path.posix` normalization used by `join`.
fn js_normalize(path: &str) -> String {
    if path.is_empty() {
        return ".".to_owned();
    }
    let absolute = path.starts_with('/');
    let trailing = path.ends_with('/');
    let mut stack: Vec<&str> = Vec::new();
    for segment in path.split('/') {
        match segment {
            "" | "." => {}
            ".." => {
                if stack.last().is_some_and(|last| *last != "..") {
                    stack.pop();
                } else if !absolute {
                    stack.push("..");
                }
            }
            other => stack.push(other),
        }
    }
    let mut out = stack.join("/");
    if absolute {
        out.insert(0, '/');
    }
    if out.is_empty() {
        out = if absolute { "/" } else { "." }.to_owned();
    }
    if trailing && !out.ends_with('/') {
        out.push('/');
    }
    out
}

/// Node `path.posix.join(a, b)`.
fn js_join(a: &str, b: &str) -> String {
    if a.is_empty() {
        return js_normalize(b);
    }
    if b.is_empty() {
        return js_normalize(a);
    }
    js_normalize(&format!("{a}/{b}"))
}

/// Node `path.posix.dirname`.
fn js_dirname(path: &str) -> String {
    let trimmed = path.trim_end_matches('/');
    if trimmed.is_empty() {
        return if path.starts_with('/') { "/" } else { "." }.to_owned();
    }
    match trimmed.rfind('/') {
        None => ".".to_owned(),
        Some(0) => "/".to_owned(),
        Some(at) => trimmed[..at].trim_end_matches('/').to_owned(),
    }
}

/// Node `path.posix.basename`.
fn js_basename(path: &str) -> &str {
    let trimmed = path.trim_end_matches('/');
    match trimmed.rfind('/') {
        None => trimmed,
        Some(at) => &trimmed[at + 1..],
    }
}

fn home_dir() -> String {
    std::env::var("HOME").unwrap_or_default()
}

/// Approximate `String.prototype.localeCompare` for ASCII names: primary
/// case-insensitive comparison, lowercase before uppercase on ties.
fn locale_compare(a: &str, b: &str) -> Ordering {
    let primary = a
        .chars()
        .flat_map(char::to_lowercase)
        .cmp(b.chars().flat_map(char::to_lowercase));
    if primary != Ordering::Equal {
        return primary;
    }
    for (ca, cb) in a.chars().zip(b.chars()) {
        if ca != cb {
            let a_lower = ca.is_lowercase();
            let b_lower = cb.is_lowercase();
            if a_lower != b_lower {
                return if a_lower {
                    Ordering::Less
                } else {
                    Ordering::Greater
                };
            }
            return ca.cmp(&cb);
        }
    }
    a.len().cmp(&b.len())
}

/// Spec: `CombinedAutocompleteProvider` — commands, base path, optional fd.
#[derive(Clone, Debug, Default)]
pub struct CombinedProvider {
    pub commands: Vec<SlashCommand>,
    pub base_path: String,
    pub fd_path: Option<String>,
}

impl CombinedProvider {
    /// Spec: `getSuggestions`. `argument_completions` bridges
    /// `SlashCommand.getArgumentCompletions(argumentPrefix)`; it is only
    /// consulted for commands flagged `has_argument_completions`.
    pub fn get_suggestions(
        &self,
        lines: &[String],
        cursor_line: usize,
        cursor_col: usize,
        force: bool,
        argument_completions: &mut dyn FnMut(&str, &str) -> Option<Vec<AutocompleteItem>>,
    ) -> Option<Suggestions> {
        let current_line = lines.get(cursor_line).map_or("", String::as_str);
        let cursor_col = clamp_boundary(current_line, cursor_col);
        let text_before_cursor = &current_line[..cursor_col];

        if let Some(at_prefix) = extract_at_prefix(text_before_cursor) {
            let (raw_prefix, _, is_quoted_prefix) = parse_path_prefix(at_prefix);
            let suggestions = self.get_fuzzy_file_suggestions(raw_prefix, is_quoted_prefix);
            if suggestions.is_empty() {
                return None;
            }
            return Some(Suggestions {
                items: suggestions,
                prefix: at_prefix.to_owned(),
            });
        }

        if !force && text_before_cursor.starts_with('/') {
            return match text_before_cursor.find(' ') {
                None => {
                    let prefix = &text_before_cursor[1..];
                    let command_items: Vec<(String, Option<String>)> = self
                        .commands
                        .iter()
                        .map(|cmd| {
                            let desc = cmd.description.clone().unwrap_or_default();
                            let full_desc = match cmd.argument_hint.as_deref() {
                                Some(hint) if !hint.is_empty() => {
                                    if desc.is_empty() {
                                        hint.to_owned()
                                    } else {
                                        format!("{hint} — {desc}")
                                    }
                                }
                                _ => desc,
                            };
                            (
                                cmd.name.clone(),
                                (!full_desc.is_empty()).then_some(full_desc),
                            )
                        })
                        .collect();
                    let filtered: Vec<AutocompleteItem> =
                        fuzzy_filter(command_items, prefix, |(name, _)| name.clone())
                            .into_iter()
                            .map(|(name, description)| AutocompleteItem {
                                value: name.clone(),
                                label: name,
                                description,
                            })
                            .collect();
                    if filtered.is_empty() {
                        return None;
                    }
                    Some(Suggestions {
                        items: filtered,
                        prefix: text_before_cursor.to_owned(),
                    })
                }
                Some(space_index) => {
                    let command_name = &text_before_cursor[1..space_index];
                    let argument_text = &text_before_cursor[space_index + 1..];
                    let command = self.commands.iter().find(|cmd| cmd.name == command_name)?;
                    if !command.has_argument_completions {
                        return None;
                    }
                    let items = argument_completions(command_name, argument_text)?;
                    if items.is_empty() {
                        return None;
                    }
                    Some(Suggestions {
                        items,
                        prefix: argument_text.to_owned(),
                    })
                }
            };
        }

        let path_match = extract_path_prefix(text_before_cursor, force)?;
        let suggestions = self.get_file_suggestions(path_match);
        if suggestions.is_empty() {
            return None;
        }
        Some(Suggestions {
            items: suggestions,
            prefix: path_match.to_owned(),
        })
    }

    /// Spec: `shouldTriggerFileCompletion`.
    pub fn should_trigger_file_completion(
        lines: &[String],
        cursor_line: usize,
        cursor_col: usize,
    ) -> bool {
        let current_line = lines.get(cursor_line).map_or("", String::as_str);
        let cursor_col = clamp_boundary(current_line, cursor_col);
        let trimmed = current_line[..cursor_col].trim();
        // Spec: don't trigger while typing a slash command at line start.
        !trimmed.starts_with('/') || trimmed.contains(' ')
    }

    /// Spec: `expandHomePath`.
    fn expand_home_path(path: &str) -> String {
        if let Some(rest) = path.strip_prefix("~/") {
            let expanded = js_join(&home_dir(), rest);
            if path.ends_with('/') && !expanded.ends_with('/') {
                return format!("{expanded}/");
            }
            return expanded;
        }
        if path == "~" {
            return home_dir();
        }
        path.to_owned()
    }

    /// Spec: `resolveScopedFuzzyQuery`.
    fn resolve_scoped_fuzzy_query(&self, raw_query: &str) -> Option<(String, String, String)> {
        let normalized_query = to_display_path(raw_query);
        let slash_index = normalized_query.rfind('/')?;
        let display_base = normalized_query[..=slash_index].to_owned();
        let query = normalized_query[slash_index + 1..].to_owned();
        let base_dir = if display_base.starts_with("~/") {
            Self::expand_home_path(&display_base)
        } else if display_base.starts_with('/') {
            display_base.clone()
        } else {
            js_join(&self.base_path, &display_base)
        };
        if !fs::metadata(&base_dir).is_ok_and(|meta| meta.is_dir()) {
            return None;
        }
        Some((base_dir, query, display_base))
    }

    /// Spec: `scopedPathForDisplay`.
    fn scoped_path_for_display(display_base: &str, relative_path: &str) -> String {
        let normalized_relative = to_display_path(relative_path);
        if display_base == "/" {
            return format!("/{normalized_relative}");
        }
        format!("{}{normalized_relative}", to_display_path(display_base))
    }

    /// Spec: `getFileSuggestions` — direct directory-listing completion.
    fn get_file_suggestions(&self, prefix: &str) -> Vec<AutocompleteItem> {
        let (raw_prefix, is_at_prefix, is_quoted_prefix) = parse_path_prefix(prefix);
        let mut expanded_prefix = raw_prefix.to_owned();
        if expanded_prefix.starts_with('~') {
            expanded_prefix = Self::expand_home_path(&expanded_prefix);
        }
        let is_root_prefix = matches!(raw_prefix, "" | "./" | "../" | "~" | "~/" | "/")
            || (is_at_prefix && raw_prefix.is_empty());

        let (search_dir, search_prefix) = if is_root_prefix || raw_prefix.ends_with('/') {
            let dir = if raw_prefix.starts_with('~') || expanded_prefix.starts_with('/') {
                expanded_prefix.clone()
            } else {
                js_join(&self.base_path, &expanded_prefix)
            };
            (dir, String::new())
        } else {
            let dir = js_dirname(&expanded_prefix);
            let file = js_basename(&expanded_prefix).to_owned();
            let search = if raw_prefix.starts_with('~') || expanded_prefix.starts_with('/') {
                dir
            } else {
                js_join(&self.base_path, &dir)
            };
            (search, file)
        };

        let Ok(entries) = fs::read_dir(&search_dir) else {
            return Vec::new();
        };
        let search_prefix_lower = search_prefix.to_lowercase();
        let mut suggestions = Vec::new();
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if !name.to_lowercase().starts_with(&search_prefix_lower) {
                continue;
            }
            let mut is_directory = entry.file_type().is_ok_and(|kind| kind.is_dir());
            if !is_directory && entry.file_type().is_ok_and(|kind| kind.is_symlink()) {
                is_directory = fs::metadata(entry.path()).is_ok_and(|meta| meta.is_dir());
            }

            let display_prefix = raw_prefix;
            let relative_path = if display_prefix.ends_with('/') {
                format!("{display_prefix}{name}")
            } else if display_prefix.contains('/') || display_prefix.contains('\\') {
                if let Some(home_relative) = display_prefix.strip_prefix("~/") {
                    let dir = js_dirname(home_relative);
                    if dir == "." {
                        format!("~/{name}")
                    } else {
                        format!("~/{}", js_join(&dir, &name))
                    }
                } else if display_prefix.starts_with('/') {
                    let dir = js_dirname(display_prefix);
                    if dir == "/" {
                        format!("/{name}")
                    } else {
                        format!("{dir}/{name}")
                    }
                } else {
                    let mut joined = js_join(&js_dirname(display_prefix), &name);
                    if display_prefix.starts_with("./") && !joined.starts_with("./") {
                        joined = format!("./{joined}");
                    }
                    joined
                }
            } else if display_prefix.starts_with('~') {
                format!("~/{name}")
            } else {
                name.clone()
            };

            let relative_path = to_display_path(&relative_path);
            let path_value = if is_directory {
                format!("{relative_path}/")
            } else {
                relative_path
            };
            let value = build_completion_value(&path_value, is_at_prefix, is_quoted_prefix);
            suggestions.push(AutocompleteItem {
                value,
                label: format!("{name}{}", if is_directory { "/" } else { "" }),
                description: None,
            });
        }

        suggestions.sort_by(|a, b| {
            let a_is_dir = a.value.ends_with('/');
            let b_is_dir = b.value.ends_with('/');
            match (a_is_dir, b_is_dir) {
                (true, false) => Ordering::Less,
                (false, true) => Ordering::Greater,
                _ => locale_compare(&a.label, &b.label),
            }
        });
        suggestions
    }

    /// Spec: `scoreEntry`.
    fn score_entry(file_path: &str, query: &str, is_directory: bool) -> i32 {
        let file_name = js_basename(file_path);
        let lower_file_name = file_name.to_lowercase();
        let lower_query = query.to_lowercase();
        let mut score = 0;
        if lower_file_name == lower_query {
            score = 100;
        } else if lower_file_name.starts_with(&lower_query) {
            score = 80;
        } else if lower_file_name.contains(&lower_query) {
            score = 50;
        } else if file_path.to_lowercase().contains(&lower_query) {
            score = 30;
        }
        if is_directory && score > 0 {
            score += 10;
        }
        score
    }

    /// Spec: `getFuzzyFileSuggestions` — fd-backed @-fuzzy search.
    fn get_fuzzy_file_suggestions(
        &self,
        query: &str,
        is_quoted_prefix: bool,
    ) -> Vec<AutocompleteItem> {
        let Some(fd_path) = self.fd_path.as_deref() else {
            return Vec::new();
        };
        let scoped = self.resolve_scoped_fuzzy_query(query);
        let (fd_base_dir, fd_query, display_base) = match &scoped {
            Some((base, scoped_query, display)) => {
                (base.as_str(), scoped_query.as_str(), Some(display.as_str()))
            }
            None => (self.base_path.as_str(), query, None),
        };
        let entries = walk_directory_with_fd(fd_base_dir, fd_path, fd_query, 100);
        let mut scored: Vec<(String, bool, i32)> = entries
            .into_iter()
            .map(|(path, is_directory)| {
                let score = if fd_query.is_empty() {
                    1
                } else {
                    Self::score_entry(&path, fd_query, is_directory)
                };
                (path, is_directory, score)
            })
            .filter(|(_, _, score)| *score > 0)
            .collect();
        scored.sort_by_key(|entry| std::cmp::Reverse(entry.2));
        scored.truncate(20);

        scored
            .into_iter()
            .map(|(entry_path, is_directory, _)| {
                let path_without_slash = if is_directory {
                    &entry_path[..entry_path.len() - 1]
                } else {
                    entry_path.as_str()
                };
                let display_path = match display_base {
                    Some(base) => Self::scoped_path_for_display(base, path_without_slash),
                    None => path_without_slash.to_owned(),
                };
                let entry_name = js_basename(path_without_slash).to_owned();
                let completion_path = if is_directory {
                    format!("{display_path}/")
                } else {
                    display_path.clone()
                };
                let value = build_completion_value(&completion_path, true, is_quoted_prefix);
                AutocompleteItem {
                    value,
                    label: format!("{entry_name}{}", if is_directory { "/" } else { "" }),
                    description: Some(display_path),
                }
            })
            .collect()
    }
}

/// Spec: `walkDirectoryWithFd` — spawn fd, parse `path` + trailing-slash
/// directory markers, drop `.git` entries.
fn walk_directory_with_fd(
    base_dir: &str,
    fd_path: &str,
    query: &str,
    max_results: usize,
) -> Vec<(String, bool)> {
    let mut args: Vec<String> = vec![
        "--base-directory".into(),
        base_dir.into(),
        "--max-results".into(),
        max_results.to_string(),
        "--type".into(),
        "f".into(),
        "--type".into(),
        "d".into(),
        "--follow".into(),
        "--hidden".into(),
        "--exclude".into(),
        ".git".into(),
        "--exclude".into(),
        ".git/*".into(),
        "--exclude".into(),
        ".git/**".into(),
    ];
    if to_display_path(query).contains('/') {
        args.push("--full-path".into());
    }
    if !query.is_empty() {
        args.push(build_fd_path_query(query));
    }
    let Ok(output) = Command::new(fd_path).args(&args).output() else {
        return Vec::new();
    };
    if !output.status.success() {
        return Vec::new();
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut results = Vec::new();
    for line in stdout.trim().split('\n').filter(|line| !line.is_empty()) {
        let display_line = to_display_path(line);
        let has_trailing_separator = display_line.ends_with('/');
        let normalized = if has_trailing_separator {
            &display_line[..display_line.len() - 1]
        } else {
            display_line.as_str()
        };
        if normalized == ".git" || normalized.starts_with(".git/") || normalized.contains("/.git/")
        {
            continue;
        }
        results.push((display_line, has_trailing_separator));
    }
    results
}

/// Spec: `extractAtPrefix`.
fn extract_at_prefix(text: &str) -> Option<&str> {
    if let Some(quoted) = extract_quoted_prefix(text)
        && quoted.starts_with("@\"")
    {
        return Some(quoted);
    }
    let token_start = find_last_delimiter(text).map_or(0, |i| i + 1);
    if text[token_start..].starts_with('@') {
        return Some(&text[token_start..]);
    }
    None
}

/// Spec: `extractPathPrefix`.
fn extract_path_prefix(text: &str, force_extract: bool) -> Option<&str> {
    if let Some(quoted) = extract_quoted_prefix(text) {
        return Some(quoted);
    }
    let token_start = find_last_delimiter(text).map_or(0, |i| i + 1);
    let path_prefix = &text[token_start..];
    if force_extract {
        return Some(path_prefix);
    }
    if path_prefix.contains('/') || path_prefix.starts_with('.') || path_prefix.starts_with("~/") {
        return Some(path_prefix);
    }
    if path_prefix.is_empty() && text.ends_with(' ') {
        return Some(path_prefix);
    }
    None
}

fn clamp_boundary(text: &str, mut at: usize) -> usize {
    at = at.min(text.len());
    while !text.is_char_boundary(at) {
        at -= 1;
    }
    at
}

/// Spec: `applyCompletion` — shared by every completion kind.
pub fn apply_completion(
    lines: &[String],
    cursor_line: usize,
    cursor_col: usize,
    item: &AutocompleteItem,
    prefix: &str,
) -> Applied {
    let current_line = lines.get(cursor_line).map_or("", String::as_str);
    let cursor_col = clamp_boundary(current_line, cursor_col);
    let before_prefix = &current_line[..cursor_col.saturating_sub(prefix.len())];
    let after_cursor = &current_line[cursor_col..];
    let is_quoted_prefix = prefix.starts_with('"') || prefix.starts_with("@\"");
    let has_leading_quote_after_cursor = after_cursor.starts_with('"');
    let has_trailing_quote_in_item = item.value.ends_with('"');
    let adjusted_after_cursor =
        if is_quoted_prefix && has_trailing_quote_in_item && has_leading_quote_after_cursor {
            &after_cursor[1..]
        } else {
            after_cursor
        };

    let mut new_lines: Vec<String> = lines.to_vec();
    if new_lines.is_empty() {
        new_lines.push(String::new());
    }

    // Slash-command name completion: line-leading "/" with no path separator.
    let is_slash_command =
        prefix.starts_with('/') && before_prefix.trim().is_empty() && !prefix[1..].contains('/');
    if is_slash_command {
        new_lines[cursor_line] = format!("{before_prefix}/{} {adjusted_after_cursor}", item.value);
        return Applied {
            lines: new_lines,
            cursor_line,
            cursor_col: before_prefix.len() + item.value.len() + 2,
        };
    }

    let is_directory = item.label.ends_with('/');
    let has_trailing_quote = item.value.ends_with('"');
    let cursor_offset = if is_directory && has_trailing_quote {
        item.value.len() - 1
    } else {
        item.value.len()
    };

    // File-attachment completion (@-prefix): no space after directories.
    if prefix.starts_with('@') {
        let suffix = if is_directory { "" } else { " " };
        new_lines[cursor_line] = format!(
            "{before_prefix}{}{suffix}{adjusted_after_cursor}",
            item.value
        );
        return Applied {
            lines: new_lines,
            cursor_line,
            cursor_col: before_prefix.len() + cursor_offset + suffix.len(),
        };
    }

    // Command-argument and plain file-path completion share the same shape.
    new_lines[cursor_line] = format!("{before_prefix}{}{adjusted_after_cursor}", item.value);
    Applied {
        lines: new_lines,
        cursor_line,
        cursor_col: before_prefix.len() + cursor_offset,
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use std::path::{Path, PathBuf};

    fn temp_tree(structure: &[&str]) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "pi-rs-autocomplete-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = fs::remove_dir_all(&dir);
        for entry in structure {
            let path = dir.join(entry.trim_end_matches('/'));
            if entry.ends_with('/') {
                fs::create_dir_all(&path).unwrap();
            } else {
                if let Some(parent) = path.parent() {
                    fs::create_dir_all(parent).unwrap();
                }
                fs::write(&path, "content").unwrap();
            }
        }
        dir
    }

    fn provider(base: &Path) -> CombinedProvider {
        CombinedProvider {
            commands: Vec::new(),
            base_path: base.to_string_lossy().into_owned(),
            fd_path: None,
        }
    }

    fn suggestions(
        provider: &CombinedProvider,
        line: &str,
        cursor_col: usize,
        force: bool,
    ) -> Option<Suggestions> {
        provider.get_suggestions(&[line.to_owned()], 0, cursor_col, force, &mut |_, _| None)
    }

    #[test]
    fn extracts_root_slash_prefix_when_forced() {
        let base = temp_tree(&["src/", "README.md"]);
        let result = suggestions(&provider(&base), "hey /", 5, true).unwrap();
        assert_eq!(result.prefix, "/");
        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn does_not_trigger_for_slash_commands() {
        let base = temp_tree(&["src/"]);
        assert_eq!(suggestions(&provider(&base), "/model", 6, true), None);
        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn command_menu_uses_fuzzy_filter_and_hint_description() {
        let base = temp_tree(&[]);
        let combined = CombinedProvider {
            commands: vec![
                SlashCommand {
                    name: "model".into(),
                    description: Some("Select model".into()),
                    ..SlashCommand::default()
                },
                SlashCommand {
                    name: "name".into(),
                    description: Some("Set name".into()),
                    argument_hint: Some("<value>".into()),
                    ..SlashCommand::default()
                },
            ],
            base_path: base.to_string_lossy().into_owned(),
            fd_path: None,
        };
        let result = suggestions(&combined, "/", 1, false).unwrap();
        assert_eq!(result.prefix, "/");
        assert_eq!(result.items.len(), 2);
        let named = result.items.iter().find(|i| i.value == "name").unwrap();
        assert_eq!(named.description.as_deref(), Some("<value> — Set name"));
        let filtered = suggestions(&combined, "/mo", 3, false).unwrap();
        assert_eq!(filtered.items.len(), 1);
        assert_eq!(filtered.items[0].value, "model");
        assert_eq!(filtered.prefix, "/mo");
        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn argument_completions_route_through_callback() {
        let base = temp_tree(&[]);
        let combined = CombinedProvider {
            commands: vec![SlashCommand {
                name: "model".into(),
                has_argument_completions: true,
                ..SlashCommand::default()
            }],
            base_path: base.to_string_lossy().into_owned(),
            fd_path: None,
        };
        let mut called = Vec::new();
        let result = combined.get_suggestions(
            &["/model gp".to_owned()],
            0,
            9,
            false,
            &mut |name, prefix| {
                called.push((name.to_owned(), prefix.to_owned()));
                Some(vec![AutocompleteItem {
                    value: "openai/gpt-5.4".into(),
                    label: "gpt-5.4".into(),
                    description: Some("openai".into()),
                }])
            },
        );
        assert_eq!(called, vec![("model".to_owned(), "gp".to_owned())]);
        let result = result.unwrap();
        assert_eq!(result.prefix, "gp");
        assert_eq!(result.items[0].value, "openai/gpt-5.4");
        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn preserves_dot_slash_prefix_for_files_and_directories() {
        let base = temp_tree(&["update.sh", "utils.ts", "src/index.ts"]);
        let combined = provider(&base);
        let result = suggestions(&combined, "./up", 4, true).unwrap();
        assert!(result.items.iter().any(|i| i.value == "./update.sh"));
        let result = suggestions(&combined, "./sr", 4, true).unwrap();
        assert!(result.items.iter().any(|i| i.value == "./src/"));
        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn quotes_paths_with_spaces_and_continues_inside_quotes() {
        let base = temp_tree(&["my folder/test.txt", "my folder/other.txt"]);
        let combined = provider(&base);
        let result = suggestions(&combined, "my", 2, true).unwrap();
        assert!(result.items.iter().any(|i| i.value == "\"my folder/\""));
        let line = "\"my folder/\"";
        let result = suggestions(&combined, line, line.len() - 1, true).unwrap();
        assert!(
            result
                .items
                .iter()
                .any(|i| i.value == "\"my folder/test.txt\"")
        );
        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn applies_quoted_completion_without_duplicating_closing_quote() {
        let line = "\"my folder/te\"".to_owned();
        let cursor_col = line.len() - 1;
        let item = AutocompleteItem {
            value: "\"my folder/test.txt\"".into(),
            label: "test.txt".into(),
            description: None,
        };
        let applied = apply_completion(&[line], 0, cursor_col, &item, "\"my folder/te");
        assert_eq!(applied.lines[0], "\"my folder/test.txt\"");
        assert_eq!(applied.cursor_col, "\"my folder/test.txt\"".len());
    }

    #[test]
    fn slash_command_apply_inserts_name_and_space() {
        let item = AutocompleteItem {
            value: "model".into(),
            label: "model".into(),
            description: None,
        };
        let applied = apply_completion(&["/mo".to_owned()], 0, 3, &item, "/mo");
        assert_eq!(applied.lines[0], "/model ");
        assert_eq!(applied.cursor_col, 7);
    }

    #[test]
    fn at_directory_apply_omits_space_and_parks_cursor_inside_quotes() {
        let item = AutocompleteItem {
            value: "@\"my folder/\"".into(),
            label: "my folder/".into(),
            description: Some("my folder".into()),
        };
        let applied = apply_completion(&["@my".to_owned()], 0, 3, &item, "@my");
        assert_eq!(applied.lines[0], "@\"my folder/\"");
        // Directory + trailing quote: cursor sits before the closing quote.
        assert_eq!(applied.cursor_col, "@\"my folder/\"".len() - 1);
        let file = AutocompleteItem {
            value: "@src/main.rs".into(),
            label: "main.rs".into(),
            description: Some("src/main.rs".into()),
        };
        let applied = apply_completion(&["@ma".to_owned()], 0, 3, &file, "@ma");
        assert_eq!(applied.lines[0], "@src/main.rs ");
        assert_eq!(applied.cursor_col, "@src/main.rs ".len());
    }

    #[test]
    fn sorts_directories_first_then_locale() {
        let base = temp_tree(&["beta.txt", "Alpha.txt", "zeta/"]);
        let combined = provider(&base);
        let result = suggestions(&combined, "", 0, true).unwrap();
        let labels: Vec<&str> = result.items.iter().map(|i| i.label.as_str()).collect();
        assert_eq!(labels, vec!["zeta/", "Alpha.txt", "beta.txt"]);
        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn fd_query_builds_full_path_pattern() {
        assert_eq!(build_fd_path_query("plain"), "plain");
        assert_eq!(
            build_fd_path_query("tui/src/auto"),
            "tui[\\\\/]src[\\\\/]auto"
        );
        assert_eq!(build_fd_path_query("components/"), "components[\\\\/]");
        assert_eq!(build_fd_path_query("a.b/c"), "a\\.b[\\\\/]c");
    }

    #[test]
    fn should_trigger_file_completion_skips_leading_slash_command() {
        let lines = vec!["/mod".to_owned()];
        assert!(!CombinedProvider::should_trigger_file_completion(
            &lines, 0, 4
        ));
        let lines = vec!["/model x".to_owned()];
        assert!(CombinedProvider::should_trigger_file_completion(
            &lines, 0, 8
        ));
        let lines = vec!["hello".to_owned()];
        assert!(CombinedProvider::should_trigger_file_completion(
            &lines, 0, 5
        ));
    }
}
