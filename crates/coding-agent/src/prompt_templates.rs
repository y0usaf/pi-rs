//! `core/prompt-templates.ts`: prompt templates from `<agentDir>/prompts`, the trusted
//! project's `.pi/prompts` and `--prompt-template` paths, and their argument substitution.

use crate::config::CONFIG_DIR_NAME;
use crate::paths::{PathInput, join, resolve, resolve_path};
use crate::skills::{Yaml, parse_frontmatter};

/// A template's name, description and body, as `loadTemplateFromFile` makes one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PromptTemplate {
    pub name: String,
    pub description: String,
    pub argument_hint: Option<String>,
    pub content: String,
    pub file_path: String,
    /// `getAutocompleteSourceTag`: `u` for the agent directory, `p` for the project, `t` for a
    /// `--prompt-template` path.
    pub tag: &'static str,
}

pub struct Input<'a> {
    pub cwd: &'a str,
    pub agent_dir: &'a str,
    pub project_trusted: bool,
    /// `--no-prompt-templates`: the default directories are skipped, `--prompt-template` paths kept.
    pub no_defaults: bool,
    /// `--prompt-template` paths, as given.
    pub cli_paths: &'a [String],
}

pub struct Loaded {
    pub templates: Vec<PromptTemplate>,
    pub warnings: Vec<String>,
}

/// `extractFrontmatter`'s body: everything after the closing `---`, trimmed.
fn body_of(content: &str) -> String {
    let text = content.strip_prefix('\u{feff}').unwrap_or(content);
    let normalized = text.replace("\r\n", "\n").replace('\r', "\n");
    if !normalized.starts_with("---") {
        return normalized;
    }
    match normalized[3..].find("\n---").map(|i| i + 3) {
        Some(end) => normalized[end + 4..].trim().to_owned(),
        None => normalized,
    }
}

/// The description a template has without one of its own: its first non-empty line, cut at 60
/// code units as JavaScript's `slice(0, 60)` cuts it.
fn first_line_description(body: &str) -> String {
    let Some(line) = body.split('\n').find(|line| !line.trim().is_empty()) else {
        return String::new();
    };
    let mut out = String::new();
    let mut units = 0;
    for c in line.chars() {
        let width = c.len_utf16();
        if units + width > 60 {
            break;
        }
        units += width;
        out.push(c);
    }
    if line.encode_utf16().count() > 60 {
        out.push_str("...");
    }
    out
}

fn basename(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

/// `loadTemplateFromFile`: one `.md` file, or a warning when it cannot be read or parsed.
fn load_template(
    path: &str,
    tag: &'static str,
    warnings: &mut Vec<String>,
) -> Option<PromptTemplate> {
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(e) => {
            warnings.push(format!(
                "Warning: Could not read prompt template {path}: {}",
                crate::node_fs::message(&e, "open", path)
            ));
            return None;
        }
    };
    let raw = String::from_utf8_lossy(&bytes).into_owned();
    let frontmatter = match parse_frontmatter(&raw) {
        Ok(frontmatter) => frontmatter,
        Err(e) => {
            warnings.push(format!("Warning: {e}: {path}"));
            return None;
        }
    };
    let get = |key: &str| frontmatter.iter().find(|(k, _)| k == key).map(|(_, v)| v);
    let body = body_of(&raw);
    let description = match get("description") {
        Some(Yaml::Str(description)) => description.clone(),
        _ => first_line_description(&body),
    };
    let argument_hint = match get("argument-hint") {
        Some(Yaml::Str(hint)) => Some(hint.clone()),
        _ => None,
    };
    Some(PromptTemplate {
        name: basename(path).trim_end_matches(".md").to_owned(),
        description,
        argument_hint,
        content: body,
        file_path: path.to_owned(),
        tag,
    })
}

/// `loadTemplatesFromDir`: the directory's own `.md` files, never its subdirectories.
fn load_dir(
    dir: &str,
    tag: &'static str,
    templates: &mut Vec<PromptTemplate>,
    warnings: &mut Vec<String>,
) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut names: Vec<String> = entries
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    for name in names {
        if !name.ends_with(".md") {
            continue;
        }
        let path = join(dir, &name);
        // Metadata follows a symlink, and a broken one is skipped.
        if !std::fs::metadata(&path).is_ok_and(|meta| meta.is_file()) {
            continue;
        }
        if let Some(template) = load_template(&path, tag, warnings) {
            templates.push(template);
        }
    }
}

pub fn load(input: &Input) -> Loaded {
    let cwd = resolve("/", input.cwd);
    let agent_dir = resolve("/", input.agent_dir);
    let mut paths: Vec<(String, &'static str)> = Vec::new();
    if !input.no_defaults {
        if input.project_trusted {
            paths.push((join(&cwd, &format!("{CONFIG_DIR_NAME}/prompts")), "p"));
        }
        paths.push((join(&agent_dir, "prompts"), "u"));
    }
    paths.extend(input.cli_paths.iter().map(|p| {
        (
            resolve_path(
                p,
                &cwd,
                PathInput {
                    trim: true,
                    ..PathInput::default()
                },
            ),
            "t",
        )
    }));
    let mut templates: Vec<PromptTemplate> = Vec::new();
    let mut warnings: Vec<String> = Vec::new();
    for (path, tag) in paths {
        match std::fs::metadata(&path) {
            Ok(meta) if meta.is_dir() => load_dir(&path, tag, &mut templates, &mut warnings),
            Ok(meta) if meta.is_file() && path.ends_with(".md") => {
                if let Some(template) = load_template(&path, tag, &mut warnings) {
                    templates.push(template);
                }
            }
            _ => {}
        }
    }
    let mut seen: Vec<String> = Vec::new();
    templates.retain(|template| {
        if seen.contains(&template.name) {
            return false;
        }
        seen.push(template.name.clone());
        true
    });
    Loaded {
        templates,
        warnings,
    }
}

/// `parseCommandArgs`: whitespace splits, quotes group, and an empty `""` is an argument.
pub fn parse_args(input: &str) -> Vec<String> {
    let mut args = Vec::new();
    let mut current = String::new();
    let mut started = false;
    let mut quote: Option<char> = None;
    for c in input.chars() {
        match quote {
            Some(q) if c == q => quote = None,
            Some(_) => current.push(c),
            None if c == '"' || c == '\'' => {
                quote = Some(c);
                started = true;
            }
            None if c.is_whitespace() => {
                if started {
                    args.push(std::mem::take(&mut current));
                    started = false;
                }
            }
            None => {
                current.push(c);
                started = true;
            }
        }
    }
    if started {
        args.push(current);
    }
    args
}

/// A run of ASCII digits: its value and how many bytes it took.
fn leading_digits(text: &str) -> Option<(usize, usize)> {
    let len = text.bytes().take_while(u8::is_ascii_digit).count();
    (len > 0).then(|| (text[..len].parse().unwrap_or(usize::MAX), len))
}

fn positional(args: &[String], index: usize) -> String {
    args.get(index.wrapping_sub(1)).cloned().unwrap_or_default()
}

/// The inside of a ``${...}``: `${X:-default}` or `${@:N[:L]}`, or None when it is neither.
fn substitute_braced(body: &str, args: &[String], all: &str) -> Option<String> {
    if let Some((head, default)) = body.split_once(":-") {
        let value = if head == "@" || head == "ARGUMENTS" {
            all.to_owned()
        } else {
            let (index, len) = leading_digits(head)?;
            if len != head.len() {
                return None;
            }
            positional(args, index)
        };
        return Some(if value.is_empty() {
            default.to_owned()
        } else {
            value
        });
    }
    let slice = body.strip_prefix("@:")?;
    let (start, len) = leading_digits(slice)?;
    let from = start.saturating_sub(1);
    let rest = &slice[len..];
    if rest.is_empty() {
        return Some(args.get(from..).unwrap_or_default().join(" "));
    }
    let (count, _) = leading_digits(rest.strip_prefix(':')?)?;
    Some(
        args.iter()
            .skip(from)
            .take(count)
            .cloned()
            .collect::<Vec<String>>()
            .join(" "),
    )
}

/// `substituteArgs`: `$1`, `$@` and `$ARGUMENTS`, ``${N:-default}``, `${@:N}` and `${@:N:L}`,
/// in one pass, so nothing substituted is substituted again.
pub fn substitute(content: &str, args: &[String]) -> String {
    let all = args.join(" ");
    let mut out = String::new();
    let mut rest = content;
    while let Some(at) = rest.find('$') {
        out.push_str(&rest[..at]);
        let tail = &rest[at + 1..];
        if let Some(inner) = tail.strip_prefix('{') {
            let Some(end) = inner.find('}') else {
                out.push_str(&rest[at..]);
                return out;
            };
            match substitute_braced(&inner[..end], args, &all) {
                Some(text) => {
                    out.push_str(&text);
                    rest = &inner[end + 1..];
                }
                None => {
                    out.push('$');
                    rest = tail;
                }
            }
            continue;
        }
        let consumed = if let Some(after) = tail.strip_prefix("ARGUMENTS") {
            out.push_str(&all);
            Some(after)
        } else if let Some(after) = tail.strip_prefix('@') {
            out.push_str(&all);
            Some(after)
        } else {
            leading_digits(tail).map(|(index, len)| {
                out.push_str(&positional(args, index));
                &tail[len..]
            })
        };
        match consumed {
            Some(after) => rest = after,
            None => {
                out.push('$');
                rest = tail;
            }
        }
    }
    out.push_str(rest);
    out
}

/// `expandPromptTemplate`: the first template of the line's name takes it, with the arguments
/// filled in; anything else, an unknown name included, stays the text it was.
pub fn expand(text: &str, templates: &[PromptTemplate]) -> String {
    let Some(rest) = text.strip_prefix('/') else {
        return text.to_owned();
    };
    let name_end = rest.find(char::is_whitespace).unwrap_or(rest.len());
    let (name, arguments) = (&rest[..name_end], rest[name_end..].trim_start());
    match templates.iter().find(|template| template.name == name) {
        Some(template) => substitute(&template.content, &parse_args(arguments)),
        None => text.to_owned(),
    }
}
