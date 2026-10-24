//! Autocomplete for slash commands and file paths (upstream `autocomplete.ts`).
//!
//! Upstream's provider is async (argument completions and `fd` run in the background). Here it is
//! synchronous: `fd` runs as a blocking child process. Cursor columns are byte offsets.

use crate::fuzzy::fuzzy_filter;
use crate::utils::{is_autocomplete_separator, js_trim};
use std::rc::Rc;

/// Completions for a command's argument.
pub type ArgumentCompletions = Rc<dyn Fn(&str) -> Option<Vec<AutocompleteItem>>>;

const PATH_DELIMITERS: &[char] = &[' ', '\t', '\x22', '\'', '='];

fn wrapper_closer(c: char) -> Option<char> {
    match c {
        '(' => Some(')'),
        '[' => Some(']'),
        '{' => Some('}'),
        '<' => Some('>'),
        '`' => Some('`'),
        _ => None,
    }
}

/// Text that is empty or ends in a separator (upstream `tokenStartRegex`).
fn is_token_start_text(text: &str) -> bool {
    text.chars().last().is_none_or(is_autocomplete_separator)
}

fn to_display_path(v: &str) -> String {
    v.replace('\\', "/")
}

fn escape_regex(v: &str) -> String {
    let mut out = String::new();
    for c in v.chars() {
        if ".*+?^${}()|[]\\".contains(c) {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

fn build_fd_path_query(query: &str) -> String {
    let n = to_display_path(query);
    if !n.contains('/') {
        return escape_regex(&n);
    }
    let trailing = n.ends_with('/');
    let trimmed = n.trim_matches('/');
    if trimmed.is_empty() {
        return n;
    }
    let sep = "[\\\\/]";
    let mut p = trimmed
        .split('/')
        .filter(|s| !s.is_empty())
        .map(escape_regex)
        .collect::<Vec<_>>()
        .join(sep);
    if trailing {
        p.push_str(sep);
    }
    p
}

/// Byte offset just past the last path delimiter, if any.
fn after_last_delimiter(text: &str) -> Option<usize> {
    let mut last = None;
    for (i, c) in text.char_indices() {
        if PATH_DELIMITERS.contains(&c) || is_autocomplete_separator(c) {
            last = Some(i + c.len_utf8());
        }
    }
    last
}

fn strip_leading_wrappers(token: &str) -> &str {
    let mut r = token;
    while let Some(c) = r.chars().next() {
        match wrapper_closer(c) {
            Some(closer) if !r[c.len_utf8()..].contains(closer) => r = &r[c.len_utf8()..],
            _ => break,
        }
    }
    r
}

fn find_unclosed_quote_start(text: &str) -> Option<usize> {
    let mut in_quotes = false;
    let mut start = 0;
    for (i, b) in text.bytes().enumerate() {
        if b == b'\x22' {
            in_quotes = !in_quotes;
            if in_quotes {
                start = i;
            }
        }
    }
    in_quotes.then_some(start)
}

fn is_token_start(text: &str, index: usize) -> bool {
    let b = text.as_bytes();
    let mut start = index;
    while start > 0 && wrapper_closer(b[start - 1] as char).is_some() {
        start -= 1;
    }
    (start > 0 && PATH_DELIMITERS.contains(&(b[start - 1] as char)))
        || is_token_start_text(&text[..start])
}

fn extract_quoted_prefix(text: &str) -> Option<&str> {
    let q = find_unclosed_quote_start(text)?;
    if q > 0 && text.as_bytes()[q - 1] == b'@' {
        return is_token_start(text, q - 1).then(|| &text[q - 1..]);
    }
    is_token_start(text, q).then(|| &text[q..])
}

struct PathPrefix<'a> {
    raw: &'a str,
    at: bool,
    quoted: bool,
}

fn parse_path_prefix(p: &str) -> PathPrefix<'_> {
    if let Some(r) = p.strip_prefix("@\x22") {
        return PathPrefix {
            raw: r,
            at: true,
            quoted: true,
        };
    }
    if let Some(r) = p.strip_prefix('\x22') {
        return PathPrefix {
            raw: r,
            at: false,
            quoted: true,
        };
    }
    if let Some(r) = p.strip_prefix('@') {
        return PathPrefix {
            raw: r,
            at: true,
            quoted: false,
        };
    }
    PathPrefix {
        raw: p,
        at: false,
        quoted: false,
    }
}

fn build_completion_value(path: &str, at: bool, quoted: bool) -> String {
    let needs_quotes = quoted || path.chars().any(is_autocomplete_separator);
    let prefix = if at { "@" } else { "" };
    if needs_quotes {
        format!("{prefix}\x22{path}\x22")
    } else {
        format!("{prefix}{path}")
    }
}

// POSIX path helpers with Node's semantics.

fn normalize_string(path: &str, allow_above_root: bool) -> String {
    let mut out: Vec<&str> = Vec::new();
    for seg in path.split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                if out.last().is_some_and(|l| *l != "..") {
                    out.pop();
                } else if allow_above_root {
                    out.push("..");
                }
            }
            s => out.push(s),
        }
    }
    out.join("/")
}

pub(crate) fn path_normalize(path: &str) -> String {
    if path.is_empty() {
        return ".".into();
    }
    let abs = path.starts_with('/');
    let trailing = path.ends_with('/');
    let mut p = normalize_string(path, !abs);
    if p.is_empty() {
        if abs {
            return "/".into();
        }
        return if trailing { "./".into() } else { ".".into() };
    }
    if trailing {
        p.push('/');
    }
    if abs { format!("/{p}") } else { p }
}

pub(crate) fn path_join(parts: &[&str]) -> String {
    let joined: Vec<&str> = parts.iter().copied().filter(|p| !p.is_empty()).collect();
    if joined.is_empty() {
        return ".".into();
    }
    path_normalize(&joined.join("/"))
}

pub(crate) fn path_dirname(p: &str) -> String {
    if p.is_empty() {
        return ".".into();
    }
    let abs = p.starts_with('/');
    let b = p.as_bytes();
    let mut end: Option<usize> = None;
    let mut matched_slash = true;
    for i in (1..b.len()).rev() {
        if b[i] == b'/' {
            if !matched_slash {
                end = Some(i);
                break;
            }
        } else {
            matched_slash = false;
        }
    }
    match end {
        None => {
            if abs {
                "/".into()
            } else {
                ".".into()
            }
        }
        Some(e) => p[..e].to_string(),
    }
}

pub(crate) fn path_basename(p: &str) -> String {
    let t = p.trim_end_matches('/');
    if t.is_empty() {
        return String::new();
    }
    t.rsplit('/').next().unwrap_or("").to_string()
}

/// An approximation of ICU root collation (`localeCompare`) for file names: punctuation before
/// digits before letters, letters compared case-insensitively, lowercase first on ties.
pub(crate) fn locale_compare(a: &str, b: &str) -> std::cmp::Ordering {
    const PUNCT: &str = "_-,;:!?.'\x22()\x5b\x5d{}@*\x2f\x5c&#%`^+<=>|~$";
    let key = |c: char| -> (u8, u32) {
        if c.is_whitespace() {
            (0, c as u32)
        } else if let Some(i) = PUNCT.find(c) {
            (1, i as u32)
        } else if c.is_ascii_digit() {
            (2, c as u32)
        } else {
            (3, c.to_lowercase().next().unwrap_or(c) as u32)
        }
    };
    let primary = a.chars().map(key).cmp(b.chars().map(key));
    if primary != std::cmp::Ordering::Equal {
        return primary;
    }
    for (x, y) in a.chars().zip(b.chars()) {
        if x != y {
            return if x.is_lowercase() && y.is_uppercase() {
                std::cmp::Ordering::Less
            } else {
                std::cmp::Ordering::Greater
            };
        }
    }
    a.cmp(b)
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct AutocompleteItem {
    pub value: String,
    pub label: String,
    pub description: Option<String>,
}

/// A slash command with optional argument completion.
#[derive(Clone, Default)]
pub struct SlashCommand {
    pub name: String,
    pub description: Option<String>,
    pub argument_hint: Option<String>,
    pub argument_completions: Option<ArgumentCompletions>,
}

/// A command entry: a full slash command or a plain item (value = name).
#[derive(Clone)]
pub enum CommandEntry {
    Command(SlashCommand),
    Item(AutocompleteItem),
}

impl CommandEntry {
    fn name(&self) -> &str {
        match self {
            CommandEntry::Command(c) => &c.name,
            CommandEntry::Item(i) => &i.value,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AutocompleteSuggestions {
    pub items: Vec<AutocompleteItem>,
    /// What the items match against (for example "/" or "src/").
    pub prefix: String,
}

/// The text after a completion and the new cursor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Completion {
    pub lines: Vec<String>,
    pub cursor_line: usize,
    pub cursor_col: usize,
}

/// Supplies suggestions for the editor.
pub trait AutocompleteProvider {
    /// Characters that trigger suggestions at token boundaries.
    fn trigger_characters(&self) -> Option<Vec<String>> {
        None
    }
    fn get_suggestions(
        &self,
        lines: &[String],
        cursor_line: usize,
        cursor_col: usize,
        force: bool,
    ) -> Option<AutocompleteSuggestions>;
    fn apply_completion(
        &self,
        lines: &[String],
        cursor_line: usize,
        cursor_col: usize,
        item: &AutocompleteItem,
        prefix: &str,
    ) -> Completion;
    fn should_trigger_file_completion(
        &self,
        _lines: &[String],
        _cursor_line: usize,
        _cursor_col: usize,
    ) -> bool {
        true
    }
}

/// Slash commands plus file paths.
pub struct CombinedAutocompleteProvider {
    commands: Vec<CommandEntry>,
    base_path: String,
    fd_path: Option<String>,
    home: String,
}

struct FdEntry {
    path: String,
    is_dir: bool,
}

fn walk_with_fd(
    base: &str,
    fd: &str,
    query: &str,
    max: usize,
    max_depth: Option<usize>,
) -> Vec<FdEntry> {
    let mut cmd = std::process::Command::new(fd);
    cmd.args([
        "--base-directory",
        base,
        "--max-results",
        &max.to_string(),
        "--type",
        "f",
        "--type",
        "d",
        "--follow",
        "--hidden",
    ]);
    cmd.args([
        "--exclude",
        ".git",
        "--exclude",
        ".git/*",
        "--exclude",
        ".git/**",
    ]);
    if let Some(d) = max_depth {
        cmd.args(["--max-depth", &d.to_string()]);
    }
    if to_display_path(query).contains('/') {
        cmd.arg("--full-path");
    }
    if !query.is_empty() {
        cmd.arg(build_fd_path_query(query));
    }
    let Ok(out) = cmd
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .output()
    else {
        return Vec::new();
    };
    if !out.status.success() || out.stdout.is_empty() {
        return Vec::new();
    }
    let text = String::from_utf8_lossy(&out.stdout);
    let mut res = Vec::new();
    for line in js_trim(&text).split('\n').filter(|l| !l.is_empty()) {
        let d = to_display_path(line);
        let trailing = d.ends_with('/');
        let n = if trailing { &d[..d.len() - 1] } else { &d[..] };
        if n == ".git" || n.starts_with(".git/") || n.contains("/.git/") {
            continue;
        }
        res.push(FdEntry {
            is_dir: trailing,
            path: d,
        });
    }
    res
}

fn utf16_len(s: &str) -> usize {
    s.encode_utf16().count()
}

impl CombinedAutocompleteProvider {
    pub fn new(commands: Vec<CommandEntry>, base_path: &str, fd_path: Option<&str>) -> Self {
        let home = std::env::var("HOME").unwrap_or_default();
        CombinedAutocompleteProvider {
            commands,
            base_path: base_path.into(),
            fd_path: fd_path.map(Into::into),
            home,
        }
    }

    /// Override the home directory used for `~` (tests).
    pub fn with_home(mut self, home: &str) -> Self {
        self.home = home.into();
        self
    }

    fn extract_at_prefix<'a>(&self, text: &'a str) -> Option<&'a str> {
        if let Some(q) = extract_quoted_prefix(text)
            && q.starts_with("@\x22")
        {
            return Some(q);
        }
        let token = strip_leading_wrappers(&text[after_last_delimiter(text).unwrap_or(0)..]);
        token.starts_with('@').then_some(token)
    }

    fn extract_path_prefix<'a>(&self, text: &'a str, force: bool) -> Option<&'a str> {
        if let Some(q) = extract_quoted_prefix(text) {
            return Some(q);
        }
        let p = strip_leading_wrappers(&text[after_last_delimiter(text).unwrap_or(0)..]);
        if force {
            return Some(p);
        }
        if p.contains('/') || p.starts_with('.') || p.starts_with("~/") {
            return Some(p);
        }
        if p.is_empty() && !text.is_empty() && is_token_start_text(text) {
            return Some(p);
        }
        None
    }

    fn expand_home(&self, path: &str) -> String {
        if let Some(rest) = path.strip_prefix("~/") {
            let e = path_join(&[&self.home, rest]);
            return if path.ends_with('/') && !e.ends_with('/') {
                format!("{e}/")
            } else {
                e
            };
        }
        if path == "~" {
            return self.home.clone();
        }
        path.into()
    }

    fn resolve_scoped_query(&self, raw: &str) -> Option<(String, String, String)> {
        let n = to_display_path(raw);
        let slash = n.rfind('/')?;
        let display_base = n[..=slash].to_string();
        let query = n[slash + 1..].to_string();
        let base_dir = if display_base.starts_with("~/") {
            self.expand_home(&display_base)
        } else if display_base.starts_with('/') {
            display_base.clone()
        } else {
            path_join(&[&self.base_path, &display_base])
        };
        if !std::fs::metadata(&base_dir).is_ok_and(|m| m.is_dir()) {
            return None;
        }
        Some((base_dir, query, display_base))
    }

    fn scoped_display(display_base: &str, rel: &str) -> String {
        let r = to_display_path(rel);
        if display_base == "/" {
            format!("/{r}")
        } else {
            format!("{}{r}", to_display_path(display_base))
        }
    }

    fn file_suggestions(&self, prefix: &str) -> Vec<AutocompleteItem> {
        let pp = parse_path_prefix(prefix);
        let raw = pp.raw;
        let mut expanded = raw.to_string();
        if expanded.starts_with('~') {
            expanded = self.expand_home(&expanded);
        }
        let is_root =
            matches!(raw, "" | "./" | "../" | "~" | "~/" | "/") || (pp.at && raw.is_empty());
        let from_root = raw.starts_with('~') || expanded.starts_with('/');
        let (search_dir, search_prefix) = if is_root || raw.ends_with('/') {
            (
                if from_root {
                    expanded.clone()
                } else {
                    path_join(&[&self.base_path, &expanded])
                },
                String::new(),
            )
        } else {
            let dir = path_dirname(&expanded);
            (
                if from_root {
                    dir
                } else {
                    path_join(&[&self.base_path, &dir])
                },
                path_basename(&expanded),
            )
        };
        let Ok(rd) = std::fs::read_dir(&search_dir) else {
            return Vec::new();
        };
        let sp = search_prefix.to_lowercase();
        let mut out = Vec::new();
        for entry in rd.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if !name.to_lowercase().starts_with(&sp) {
                continue;
            }
            let ft = entry.file_type().ok();
            let mut is_dir = ft.is_some_and(|t| t.is_dir());
            if !is_dir && ft.is_some_and(|t| t.is_symlink()) {
                is_dir = std::fs::metadata(entry.path()).is_ok_and(|m| m.is_dir());
            }
            let dp = raw;
            let mut rel = if dp.ends_with('/') {
                format!("{dp}{name}")
            } else if dp.contains('/') || dp.contains('\\') {
                if let Some(home_rel) = dp.strip_prefix("~/") {
                    let dir = path_dirname(home_rel);
                    format!(
                        "~/{}",
                        if dir == "." {
                            name.clone()
                        } else {
                            path_join(&[&dir, &name])
                        }
                    )
                } else if dp.starts_with('/') {
                    let dir = path_dirname(dp);
                    if dir == "/" {
                        format!("/{name}")
                    } else {
                        format!("{dir}/{name}")
                    }
                } else {
                    let mut r = path_join(&[&path_dirname(dp), &name]);
                    if dp.starts_with("./") && !r.starts_with("./") {
                        r = format!("./{r}");
                    }
                    r
                }
            } else if dp.starts_with('~') {
                format!("~/{name}")
            } else {
                name.clone()
            };
            rel = to_display_path(&rel);
            let pv = if is_dir { format!("{rel}/") } else { rel };
            out.push(AutocompleteItem {
                value: build_completion_value(&pv, pp.at, pp.quoted),
                label: format!("{name}{}", if is_dir { "/" } else { "" }),
                description: None,
            });
        }
        out.sort_by(|a, b| {
            let (ad, bd) = (a.label.ends_with('/'), b.label.ends_with('/'));
            bd.cmp(&ad).then_with(|| locale_compare(&a.label, &b.label))
        });
        out
    }

    fn score_entry(path: &str, query: &str, is_dir: bool) -> i64 {
        let file = path_basename(path).to_lowercase();
        let q = query.to_lowercase();
        let mut score = if file == q {
            100
        } else if file.starts_with(&q) {
            80
        } else if file.contains(&q) {
            50
        } else if path.to_lowercase().contains(&q) {
            30
        } else {
            0
        };
        if is_dir && score > 0 {
            score += 10;
        }
        score
    }

    fn fuzzy_file_suggestions(&self, query: &str, quoted: bool) -> Vec<AutocompleteItem> {
        let Some(fd) = &self.fd_path else {
            return Vec::new();
        };
        let scoped = self.resolve_scoped_query(query);
        let base = scoped
            .as_ref()
            .map_or(self.base_path.clone(), |s| s.0.clone());
        let fq = scoped.as_ref().map_or(query.to_string(), |s| s.1.clone());
        let base_entries = walk_with_fd(&base, fd, &fq, 100, Some(1));
        let recursive = walk_with_fd(&base, fd, &fq, 100, None);
        let mut seen: std::collections::HashSet<String> =
            base_entries.iter().map(|e| e.path.clone()).collect();
        let mut entries = base_entries;
        for e in recursive {
            if seen.insert(e.path.clone()) {
                entries.push(e);
            }
        }
        let mut scored: Vec<(FdEntry, i64)> = entries
            .into_iter()
            .map(|e| {
                let s = if fq.is_empty() {
                    1
                } else {
                    Self::score_entry(&e.path, &fq, e.is_dir)
                };
                (e, s)
            })
            .filter(|(_, s)| *s > 0)
            .collect();
        let depth = |p: &str| {
            to_display_path(p)
                .split('/')
                .filter(|s| !s.is_empty())
                .count()
        };
        scored.sort_by(|(a, sa), (b, sb)| {
            sb.cmp(sa)
                .then_with(|| depth(&a.path).cmp(&depth(&b.path)))
                .then_with(|| utf16_len(&a.path).cmp(&utf16_len(&b.path)))
                .then_with(|| locale_compare(&a.path, &b.path))
        });
        scored
            .into_iter()
            .take(20)
            .map(|(e, _)| {
                let without = if e.is_dir {
                    &e.path[..e.path.len() - 1]
                } else {
                    &e.path[..]
                };
                let display = match &scoped {
                    Some(s) => Self::scoped_display(&s.2, without),
                    None => without.to_string(),
                };
                let name = path_basename(without);
                let completion = if e.is_dir {
                    format!("{display}/")
                } else {
                    display.clone()
                };
                AutocompleteItem {
                    value: build_completion_value(&completion, true, quoted),
                    label: format!("{name}{}", if e.is_dir { "/" } else { "" }),
                    description: Some(display),
                }
            })
            .collect()
    }
}

impl AutocompleteProvider for CombinedAutocompleteProvider {
    fn get_suggestions(
        &self,
        lines: &[String],
        cursor_line: usize,
        cursor_col: usize,
        force: bool,
    ) -> Option<AutocompleteSuggestions> {
        let line = lines.get(cursor_line).map_or("", |s| s);
        let before = &line[..cursor_col.min(line.len())];
        if let Some(at) = self.extract_at_prefix(before) {
            let pp = parse_path_prefix(at);
            let items = self.fuzzy_file_suggestions(pp.raw, pp.quoted);
            if items.is_empty() {
                return None;
            }
            return Some(AutocompleteSuggestions {
                items,
                prefix: at.into(),
            });
        }
        let command_text = before
            .trim_start_matches(|c: char| crate::unicode::is_js_whitespace(c) || c == '\u{FEFF}');
        if !force && command_text.starts_with('/') {
            let Some(space) = command_text.find(' ') else {
                let prefix = &command_text[1..];
                let items: Vec<(String, Option<String>)> = self
                    .commands
                    .iter()
                    .map(|c| {
                        let (desc, hint) = match c {
                            CommandEntry::Command(c) => (
                                c.description.clone().unwrap_or_default(),
                                c.argument_hint.clone().filter(|h| !h.is_empty()),
                            ),
                            CommandEntry::Item(i) => {
                                (i.description.clone().unwrap_or_default(), None)
                            }
                        };
                        let full = match hint {
                            Some(h) if !desc.is_empty() => format!("{h} — {desc}"),
                            Some(h) => h,
                            None => desc,
                        };
                        (c.name().to_string(), (!full.is_empty()).then_some(full))
                    })
                    .collect();
                let idx: Vec<usize> = (0..items.len()).collect();
                let bare = fuzzy_filter(&idx, prefix, |&i| {
                    let n = &items[i].0;
                    n.strip_prefix("skill:").unwrap_or(n).to_string()
                });
                let rest: Vec<usize> = idx
                    .iter()
                    .copied()
                    .filter(|i| items[*i].0.starts_with("skill:") && !bare.contains(i))
                    .collect();
                let full_only = fuzzy_filter(&rest, prefix, |&i| items[i].0.clone());
                let filtered: Vec<AutocompleteItem> = bare
                    .into_iter()
                    .chain(full_only)
                    .map(|i| AutocompleteItem {
                        value: items[i].0.clone(),
                        label: items[i].0.clone(),
                        description: items[i].1.clone(),
                    })
                    .collect();
                if filtered.is_empty() {
                    return None;
                }
                return Some(AutocompleteSuggestions {
                    items: filtered,
                    prefix: command_text.into(),
                });
            };
            let name = &command_text[1..space];
            let arg = &command_text[space + 1..];
            let cmd = self.commands.iter().find(|c| c.name() == name)?;
            let CommandEntry::Command(c) = cmd else {
                return None;
            };
            let f = c.argument_completions.as_ref()?;
            let items = f(arg).filter(|i| !i.is_empty())?;
            return Some(AutocompleteSuggestions {
                items,
                prefix: arg.into(),
            });
        }
        let path = self.extract_path_prefix(before, force)?;
        let items = self.file_suggestions(path);
        if items.is_empty() {
            return None;
        }
        Some(AutocompleteSuggestions {
            items,
            prefix: path.into(),
        })
    }

    fn apply_completion(
        &self,
        lines: &[String],
        cursor_line: usize,
        cursor_col: usize,
        item: &AutocompleteItem,
        prefix: &str,
    ) -> Completion {
        let line = lines.get(cursor_line).map_or("", |s| s);
        let start = cursor_col.saturating_sub(prefix.len());
        let before_prefix = &line[..start];
        let after = &line[cursor_col..];
        let quoted_prefix = prefix.starts_with('\x22') || prefix.starts_with("@\x22");
        let adj_after =
            if quoted_prefix && item.value.ends_with('\x22') && after.starts_with('\x22') {
                &after[1..]
            } else {
                after
            };
        let mut new_lines = lines.to_vec();
        if new_lines.len() <= cursor_line {
            new_lines.resize(cursor_line + 1, String::new());
        }
        let is_dir = item.label.ends_with('/');
        let trailing_quote = item.value.ends_with('\x22');
        let offset = if is_dir && trailing_quote {
            item.value.len() - 1
        } else {
            item.value.len()
        };
        let is_slash = prefix.starts_with('/')
            && js_trim(before_prefix).is_empty()
            && !prefix[1..].contains('/');
        if is_slash {
            new_lines[cursor_line] = format!("{before_prefix}/{} {adj_after}", item.value);
            return Completion {
                lines: new_lines,
                cursor_line,
                cursor_col: before_prefix.len() + item.value.len() + 2,
            };
        }
        if prefix.starts_with('@') {
            let suffix = if is_dir { "" } else { " " };
            new_lines[cursor_line] = format!("{before_prefix}{}{suffix}{adj_after}", item.value);
            return Completion {
                lines: new_lines,
                cursor_line,
                cursor_col: before_prefix.len() + offset + suffix.len(),
            };
        }
        new_lines[cursor_line] = format!("{before_prefix}{}{adj_after}", item.value);
        Completion {
            lines: new_lines,
            cursor_line,
            cursor_col: before_prefix.len() + offset,
        }
    }

    fn should_trigger_file_completion(
        &self,
        lines: &[String],
        cursor_line: usize,
        cursor_col: usize,
    ) -> bool {
        let line = lines.get(cursor_line).map_or("", |s| s);
        let t = js_trim(&line[..cursor_col.min(line.len())]);
        !(t.starts_with('/') && !t.contains(' '))
    }
}
