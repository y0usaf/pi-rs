//! Skills (`core/skills.ts`, and the skill half of `core/package-manager.ts`'s discovery):
//! finding `SKILL.md` files and listing them in the system prompt.

use std::collections::BTreeSet;
use std::path::Path;

use crate::paths::{PathInput, dirname, home_dir, join, relative, resolve, resolve_path};
use crate::settings::canonicalize;

const MAX_NAME_LENGTH: usize = 64;
const MAX_DESCRIPTION_LENGTH: usize = 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Skill {
    pub name: String,
    pub description: String,
    pub file_path: String,
    pub base_dir: String,
    pub disable_model_invocation: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostic {
    pub kind: &'static str,
    pub message: String,
    pub path: String,
}

/// One frontmatter value of the kinds skills use.
#[derive(Debug, Clone, PartialEq)]
pub enum Yaml {
    Str(String),
    Bool(bool),
    Other,
}

/// `parseFrontmatter`'s YAML, for the top-level keys of a mapping: plain, quoted and block
/// (`|`, `>`, with `-`/`+` chomping) scalars, and booleans. Nested values are skipped as
/// [`Yaml::Other`]. `Err` is a document this reader cannot take as a mapping.
pub fn parse_frontmatter(content: &str) -> Result<Vec<(String, Yaml)>, String> {
    let text = content.strip_prefix('\u{feff}').unwrap_or(content);
    let normalized = text.replace("\r\n", "\n").replace('\r', "\n");
    if !normalized.starts_with("---") {
        return Ok(Vec::new());
    }
    let Some(end) = normalized[3..].find("\n---").map(|i| i + 3) else {
        return Ok(Vec::new());
    };
    let yaml = if end >= 4 { &normalized[4..end] } else { "" };
    parse_mapping(yaml)
}

fn indent_of(line: &str) -> usize {
    line.len() - line.trim_start_matches(' ').len()
}

fn parse_mapping(yaml: &str) -> Result<Vec<(String, Yaml)>, String> {
    let lines: Vec<&str> = yaml.split('\n').collect();
    let mut out: Vec<(String, Yaml)> = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        let line = lines[i];
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            i += 1;
            continue;
        }
        if indent_of(line) != 0 {
            return Err(format!("unexpected indentation at line {}", i + 1));
        }
        let Some(colon) = find_key_colon(line) else {
            return Err(format!("expected a key at line {}", i + 1));
        };
        let key = unquote_key(line[..colon].trim());
        let rest = strip_comment(line[colon + 1..].trim());
        i += 1;
        // The lines that belong to this key: indented, or blank.
        let start = i;
        while i < lines.len() && (lines[i].trim().is_empty() || indent_of(lines[i]) > 0) {
            i += 1;
        }
        let block = &lines[start..i];
        let value = if rest.starts_with('|') || rest.starts_with('>') {
            Yaml::Str(block_scalar(rest, block))
        } else if rest.is_empty() {
            if block.iter().all(|l| l.trim().is_empty()) {
                Yaml::Other
            } else {
                let first = block
                    .iter()
                    .find(|l| !l.trim().is_empty())
                    .unwrap()
                    .trim_start();
                if first.starts_with("- ") || first == "-" || find_key_colon(first).is_some() {
                    Yaml::Other
                } else {
                    plain_or_quoted(&fold_plain(block.iter().map(|l| l.trim()).collect()))?
                }
            }
        } else if rest.starts_with('"') || rest.starts_with('\'') {
            let mut joined: Vec<&str> = vec![rest];
            joined.extend(block.iter().map(|l| l.trim()));
            plain_or_quoted(&fold_plain(joined))?
        } else if rest.starts_with('[') || rest.starts_with('{') {
            Yaml::Other
        } else {
            let mut joined: Vec<&str> = vec![rest];
            joined.extend(block.iter().map(|l| l.trim()));
            plain_or_quoted(&fold_plain(joined))?
        };
        match out.iter_mut().find(|(k, _)| *k == key) {
            Some(_) => return Err(format!("Map keys must be unique at line {start}")),
            None => out.push((key, value)),
        }
    }
    Ok(out)
}

/// The `:` ending a mapping key: followed by a space or the end of the line, outside quotes.
fn find_key_colon(line: &str) -> Option<usize> {
    let bytes = line.as_bytes();
    let mut quote: Option<u8> = None;
    for (i, &b) in bytes.iter().enumerate() {
        match quote {
            Some(q) if b == q => quote = None,
            Some(_) => {}
            None if (b == b'"' || b == b'\'') && i == indent_of(line) => quote = Some(b),
            None if b == b':' && (i + 1 == bytes.len() || bytes[i + 1] == b' ') => return Some(i),
            None => {}
        }
    }
    None
}

fn unquote_key(key: &str) -> String {
    match plain_or_quoted(key) {
        Ok(Yaml::Str(s)) => s,
        _ => key.to_string(),
    }
}

fn strip_comment(value: &str) -> &str {
    if value.starts_with('"') || value.starts_with('\'') {
        return value;
    }
    match value.find(" #") {
        Some(i) => value[..i].trim_end(),
        None if value.starts_with('#') => "",
        None => value,
    }
}

/// Plain multi-line scalars fold: line breaks become spaces, blank lines become `\n`.
fn fold_plain(lines: Vec<&str>) -> String {
    let mut out = String::new();
    let mut pending_newlines = 0;
    let mut started = false;
    for line in lines {
        if line.is_empty() {
            if started {
                pending_newlines += 1;
            }
            continue;
        }
        if started {
            if pending_newlines > 0 {
                out.push_str(&"\n".repeat(pending_newlines));
            } else {
                out.push(' ');
            }
        }
        pending_newlines = 0;
        out.push_str(strip_comment(line));
        started = true;
    }
    out
}

/// `|` keeps line breaks, `>` folds them; `-` strips the final newline, `+` keeps all trailing ones.
fn block_scalar(header: &str, block: &[&str]) -> String {
    let literal = header.starts_with('|');
    let chomp = if header.contains('-') {
        '-'
    } else if header.contains('+') {
        '+'
    } else {
        ' '
    };
    let indent = block
        .iter()
        .filter(|l| !l.trim().is_empty())
        .map(|l| indent_of(l))
        .min()
        .unwrap_or(0);
    let lines: Vec<&str> = block
        .iter()
        .map(|l| if l.len() >= indent { &l[indent..] } else { "" })
        .collect();
    let mut body = String::new();
    if literal {
        for line in &lines {
            body.push_str(line);
            body.push('\n');
        }
    } else {
        let mut prev_more_indented = false;
        let mut first = true;
        let mut blank_run = 0;
        for line in &lines {
            if line.is_empty() {
                blank_run += 1;
                continue;
            }
            let more_indented = line.starts_with(' ');
            if !first {
                if blank_run > 0 {
                    body.push_str(&"\n".repeat(if prev_more_indented || more_indented {
                        blank_run + 1
                    } else {
                        blank_run
                    }));
                } else if prev_more_indented || more_indented {
                    body.push('\n');
                } else {
                    body.push(' ');
                }
            } else if blank_run > 0 {
                body.push_str(&"\n".repeat(blank_run));
            }
            blank_run = 0;
            body.push_str(line);
            prev_more_indented = more_indented;
            first = false;
        }
        body.push_str(&"\n".repeat(blank_run + 1));
    }
    let content = body.trim_end_matches('\n');
    match chomp {
        '-' => content.to_string(),
        '+' => body,
        _ if content.is_empty() => String::new(),
        _ => format!("{content}\n"),
    }
}

fn plain_or_quoted(value: &str) -> Result<Yaml, String> {
    if let Some(inner) = value.strip_prefix('"') {
        let inner = inner.strip_suffix('"').ok_or("Missing closing \"quote")?;
        let mut out = String::new();
        let mut chars = inner.chars();
        while let Some(c) = chars.next() {
            if c != '\\' {
                out.push(c);
                continue;
            }
            match chars.next() {
                Some('n') => out.push('\n'),
                Some('t') => out.push('\t'),
                Some('"') => out.push('"'),
                Some('\\') => out.push('\\'),
                Some('/') => out.push('/'),
                Some('0') => out.push('\0'),
                Some(other) => {
                    out.push('\\');
                    out.push(other);
                }
                None => out.push('\\'),
            }
        }
        return Ok(Yaml::Str(out));
    }
    if let Some(inner) = value.strip_prefix('\'') {
        let inner = inner.strip_suffix('\'').ok_or("Missing closing 'quote")?;
        return Ok(Yaml::Str(inner.replace("''", "'")));
    }
    Ok(match value {
        "true" | "True" | "TRUE" => Yaml::Bool(true),
        "false" | "False" | "FALSE" => Yaml::Bool(false),
        "" | "~" | "null" | "Null" | "NULL" => Yaml::Other,
        _ if value.parse::<f64>().is_ok() => Yaml::Other,
        _ => Yaml::Str(value.to_string()),
    })
}

fn validate_name(name: &str) -> Vec<String> {
    let mut errors = Vec::new();
    let len = name.encode_utf16().count();
    if len > MAX_NAME_LENGTH {
        errors.push(format!("name exceeds {MAX_NAME_LENGTH} characters ({len})"));
    }
    if name.is_empty()
        || !name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
    {
        errors.push(
            "name contains invalid characters (must be lowercase a-z, 0-9, hyphens only)".into(),
        );
    }
    if name.starts_with('-') || name.ends_with('-') {
        errors.push("name must not start or end with a hyphen".into());
    }
    if name.contains("--") {
        errors.push("name must not contain consecutive hyphens".into());
    }
    errors
}

fn basename(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

fn warn(diags: &mut Vec<Diagnostic>, message: impl Into<String>, path: &str) {
    diags.push(Diagnostic {
        kind: "warning",
        message: message.into(),
        path: path.to_string(),
    });
}

/// `loadSkillFromFile`.
fn load_skill_from_file(path: &str, diags: &mut Vec<Diagnostic>) -> Option<Skill> {
    let declared = basename(path) == "SKILL.md";
    let raw = match std::fs::read(path) {
        Ok(bytes) => String::from_utf8_lossy(&bytes).into_owned(),
        Err(e) => {
            warn(diags, crate::node_fs::message(&e, "open", path), path);
            return None;
        }
    };
    let frontmatter = match parse_frontmatter(&raw) {
        Ok(f) => f,
        Err(e) => {
            if declared {
                warn(diags, e, path);
            }
            return None;
        }
    };
    let get = |k: &str| frontmatter.iter().find(|(key, _)| key == k).map(|(_, v)| v);
    let description = match get("description") {
        Some(Yaml::Str(s)) => Some(s.clone()),
        _ => None,
    };
    let has_description = description.as_ref().is_some_and(|d| !d.trim().is_empty());
    if !declared && !has_description {
        return None;
    }
    let skill_dir = dirname(path);
    match &description {
        Some(d) if has_description => {
            let len = d.encode_utf16().count();
            if len > MAX_DESCRIPTION_LENGTH {
                warn(
                    diags,
                    format!("description exceeds {MAX_DESCRIPTION_LENGTH} characters ({len})"),
                    path,
                );
            }
        }
        _ => warn(diags, "description is required", path),
    }
    let name = match get("name") {
        Some(Yaml::Str(s)) if !s.is_empty() => s.clone(),
        _ => basename(&skill_dir).to_string(),
    };
    for e in validate_name(&name) {
        warn(diags, e, path);
    }
    if !has_description {
        return None;
    }
    Some(Skill {
        name,
        description: description.unwrap_or_default(),
        file_path: path.to_string(),
        base_dir: skill_dir,
        disable_model_invocation: get("disable-model-invocation") == Some(&Yaml::Bool(true)),
    })
}

/// The rules of `.gitignore`, `.ignore` and `.fdignore` files met while walking a skill tree,
/// each prefixed with its directory relative to the walk's root, as upstream feeds the
/// `ignore` package. Supported: `!` negation, a trailing `/` for directories, anchoring by a
/// leading or inner `/`, and `*`, `?`, `**`, `[...]`.
#[derive(Default)]
struct Ignore {
    rules: Vec<(bool, String)>,
}

impl Ignore {
    fn add_dir(&mut self, dir: &str, root: &str) {
        let rel = relative(root, dir);
        let prefix = if rel.is_empty() {
            String::new()
        } else {
            format!("{rel}/")
        };
        for name in [".gitignore", ".ignore", ".fdignore"] {
            let Ok(content) = std::fs::read_to_string(join(dir, name)) else {
                continue;
            };
            for line in content.split('\n') {
                let line = line.strip_suffix('\r').unwrap_or(line);
                let trimmed = line.trim();
                if trimmed.is_empty() || (trimmed.starts_with('#') && !trimmed.starts_with("\\#")) {
                    continue;
                }
                let (negated, mut pattern) = match line.strip_prefix('!') {
                    Some(rest) => (true, rest),
                    None => (
                        false,
                        line.strip_prefix('\\')
                            .filter(|r| r.starts_with('!'))
                            .unwrap_or(line),
                    ),
                };
                pattern = pattern.strip_prefix('/').unwrap_or(pattern);
                self.rules.push((negated, format!("{prefix}{pattern}")));
            }
        }
    }

    /// `ig.ignores(path)`; directories are given with a trailing `/`.
    fn ignores(&self, path: &str) -> bool {
        let is_dir = path.ends_with('/');
        let path = path.trim_end_matches('/');
        let mut ignored = false;
        for (negated, rule) in &self.rules {
            if ignored != *negated {
                continue;
            }
            if rule_matches(rule.trim_end(), path, is_dir) {
                ignored = !negated;
            }
        }
        ignored
    }
}

fn rule_matches(rule: &str, path: &str, is_dir: bool) -> bool {
    let dir_only = rule.ends_with('/');
    let rule = rule.trim_end_matches('/');
    if rule.is_empty() {
        return false;
    }
    let anchored = rule.contains('/');
    let segments: Vec<&str> = path.split('/').collect();
    // A rule matching a parent directory ignores everything below it.
    for end in 1..=segments.len() {
        let is_last = end == segments.len();
        if dir_only && is_last && !is_dir {
            continue;
        }
        let candidate = segments[..end].join("/");
        let hit = if anchored {
            crate::model_resolver::glob_match(&candidate, rule)
        } else {
            crate::model_resolver::glob_match(segments[end - 1], rule)
        };
        if hit {
            return true;
        }
    }
    false
}

struct Entry {
    name: String,
    path: String,
    is_dir: bool,
    is_file: bool,
}

fn read_entries(dir: &str) -> Option<Vec<Entry>> {
    let mut entries = Vec::new();
    for e in std::fs::read_dir(dir).ok()? {
        let Ok(e) = e else { continue };
        let name = e.file_name().to_string_lossy().into_owned();
        let path = join(dir, &name);
        let Ok(ft) = e.file_type() else { continue };
        let (is_dir, is_file) = if ft.is_symlink() {
            match std::fs::metadata(&path) {
                Ok(m) => (m.is_dir(), m.is_file()),
                Err(_) => continue,
            }
        } else {
            (ft.is_dir(), ft.is_file())
        };
        entries.push(Entry {
            name,
            path,
            is_dir,
            is_file,
        });
    }
    // Node's readdirSync goes through libuv's scandir, which sorts names with strcmp.
    entries.sort_by(|a, b| a.name.cmp(&b.name));
    Some(entries)
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Pi,
    Agents,
}

/// `collectSkillEntries` (package-manager.ts): a directory holding `SKILL.md` is one skill and
/// is not descended; `.md` files count at the root in `pi` mode and below it in `agents` mode.
fn collect_skill_entries(dir: &str, mode: Mode, ig: &mut Ignore, root: &str) -> Vec<String> {
    let mut out = Vec::new();
    if !Path::new(dir).exists() {
        return out;
    }
    ig.add_dir(dir, root);
    let Some(entries) = read_entries(dir) else {
        return out;
    };
    for e in &entries {
        if e.name == "SKILL.md" && e.is_file && !ig.ignores(&relative(root, &e.path)) {
            out.push(e.path.clone());
            return out;
        }
    }
    for e in &entries {
        if e.name.starts_with('.') || e.name == "node_modules" {
            continue;
        }
        let rel = relative(root, &e.path);
        let at_root = dir == root;
        if e.is_file
            && e.name.ends_with(".md")
            && !ig.ignores(&rel)
            && ((mode == Mode::Pi && at_root) || (mode == Mode::Agents && !at_root))
        {
            out.push(e.path.clone());
            continue;
        }
        if !e.is_dir || ig.ignores(&format!("{rel}/")) {
            continue;
        }
        out.extend(collect_skill_entries(&e.path, mode, ig, root));
    }
    out
}

/// `loadSkillsFromDirInternal` (skills.ts), for `--skill DIR`.
fn load_skills_from_dir(
    dir: &str,
    root_files: bool,
    ig: &mut Ignore,
    root: &str,
    diags: &mut Vec<Diagnostic>,
) -> Vec<Skill> {
    let mut skills = Vec::new();
    if !Path::new(dir).exists() {
        return skills;
    }
    ig.add_dir(dir, root);
    let Some(entries) = read_entries(dir) else {
        return skills;
    };
    for e in &entries {
        if e.name == "SKILL.md" {
            if !e.is_file || ig.ignores(&relative(root, &e.path)) {
                continue;
            }
            skills.extend(load_skill_from_file(&e.path, diags));
            return skills;
        }
    }
    for e in &entries {
        if e.name.starts_with('.') || e.name == "node_modules" {
            continue;
        }
        let rel = relative(root, &e.path);
        if ig.ignores(&if e.is_dir { format!("{rel}/") } else { rel }) {
            continue;
        }
        if e.is_dir {
            skills.extend(load_skills_from_dir(&e.path, false, ig, root, diags));
            continue;
        }
        if e.is_file && root_files && e.name.ends_with(".md") {
            skills.extend(load_skill_from_file(&e.path, diags));
        }
    }
    skills
}

fn find_git_repo_root(start: &str) -> Option<String> {
    let mut dir = start.to_string();
    loop {
        if Path::new(&join(&dir, ".git")).exists() {
            return Some(dir);
        }
        let parent = dirname(&dir);
        if parent == dir {
            return None;
        }
        dir = parent;
    }
}

/// `collectAncestorAgentsSkillDirs`: `.agents/skills` from the cwd up to the git root (or `/`).
fn ancestor_agents_skill_dirs(start: &str) -> Vec<String> {
    let start = resolve("/", start);
    let root = find_git_repo_root(&start);
    let mut dirs = Vec::new();
    let mut dir = start;
    loop {
        dirs.push(join(&dir, ".agents/skills"));
        if root.as_deref() == Some(dir.as_str()) {
            break;
        }
        let parent = dirname(&dir);
        if parent == dir {
            break;
        }
        dir = parent;
    }
    dirs
}

pub struct SkillsInput<'a> {
    pub cwd: &'a str,
    pub agent_dir: &'a str,
    pub project_trusted: bool,
    /// `--no-skills`.
    pub no_skills: bool,
    /// `--skill` paths, as given.
    pub cli_paths: &'a [String],
}

pub struct LoadedSkills {
    pub skills: Vec<Skill>,
    pub diagnostics: Vec<Diagnostic>,
}

/// The skills a print-mode session sees: auto-discovered ones (project `.pi/skills` and
/// `.agents/skills` up to the git root when the project is trusted, then `<agentDir>/skills`
/// and `~/.agents/skills`), then `--skill` paths; the first skill of a name wins.
pub fn load_skills(input: &SkillsInput) -> LoadedSkills {
    let cwd = resolve("/", input.cwd);
    let agent_dir = resolve("/", input.agent_dir);
    let mut auto: Vec<String> = Vec::new();
    if !input.no_skills {
        let user_agents = join(&home_dir(), ".agents/skills");
        let collect =
            |dir: &str, mode: Mode| collect_skill_entries(dir, mode, &mut Ignore::default(), dir);
        if input.project_trusted {
            auto.extend(collect(&join(&cwd, ".pi/skills"), Mode::Pi));
            for dir in ancestor_agents_skill_dirs(&cwd) {
                if resolve("/", &dir) != resolve("/", &user_agents) {
                    auto.extend(collect(&dir, Mode::Agents));
                }
            }
        }
        auto.extend(collect(&join(&agent_dir, "skills"), Mode::Pi));
        auto.extend(collect(&user_agents, Mode::Agents));
    }
    // toResolvedPaths then mergePaths: drop paths whose real path was already seen.
    let mut seen: BTreeSet<String> = BTreeSet::new();
    let mut paths: Vec<String> = Vec::new();
    let cli: Vec<String> = input
        .cli_paths
        .iter()
        .map(|p| {
            resolve_path(
                p,
                &cwd,
                PathInput {
                    trim: true,
                    ..PathInput::default()
                },
            )
        })
        .collect();
    for p in auto.into_iter().chain(cli) {
        if seen.insert(canonicalize(&p)) {
            paths.push(p);
        }
    }

    let mut diags: Vec<Diagnostic> = Vec::new();
    let mut collisions: Vec<Diagnostic> = Vec::new();
    let mut skills: Vec<Skill> = Vec::new();
    let mut real_paths: BTreeSet<String> = BTreeSet::new();
    let mut add = |found: Vec<Skill>, skills: &mut Vec<Skill>| {
        for skill in found {
            let real = canonicalize(&skill.file_path);
            if real_paths.contains(&real) {
                continue;
            }
            if skills.iter().any(|s| s.name == skill.name) {
                collisions.push(Diagnostic {
                    kind: "collision",
                    message: format!("name \"{}\" collision", skill.name),
                    path: skill.file_path.clone(),
                });
            } else {
                real_paths.insert(real);
                skills.push(skill);
            }
        }
    };
    for path in paths {
        let Ok(meta) = std::fs::metadata(&path) else {
            warn(&mut diags, "skill path does not exist", &path);
            continue;
        };
        if meta.is_dir() {
            let found =
                load_skills_from_dir(&path, true, &mut Ignore::default(), &path, &mut diags);
            add(found, &mut skills);
        } else if meta.is_file() && path.ends_with(".md") {
            let mut file_diags = Vec::new();
            let found = load_skill_from_file(&path, &mut file_diags);
            diags.extend(file_diags);
            add(found.into_iter().collect(), &mut skills);
        } else {
            warn(&mut diags, "skill path is not a markdown file", &path);
        }
    }
    diags.extend(collisions);
    LoadedSkills {
        skills,
        diagnostics: diags,
    }
}

fn escape_xml(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

/// `formatSkillsForPrompt`; `reader` is `read`, `bash` or `indirect`.
pub fn format_skills_for_prompt(skills: &[Skill], reader: &str) -> String {
    let visible: Vec<&Skill> = skills
        .iter()
        .filter(|s| !s.disable_model_invocation)
        .collect();
    if visible.is_empty() {
        return String::new();
    }
    let mut lines: Vec<String> = vec![
        "\n\nThe following skills provide specialized instructions for specific tasks.".into(),
        match reader {
            "read" => "Use the read tool to load a skill's file when the task matches its description.",
            "bash" => "Use bash to load a skill's file when the task matches its description.",
            _ => "Load a skill's file when the task matches its description.",
        }
        .into(),
        "When a skill file references a relative path, resolve it against the skill directory (parent of SKILL.md / dirname of the path) and use that absolute path in tool commands.".into(),
        String::new(),
        "<available_skills>".into(),
    ];
    for s in visible {
        lines.push("  <skill>".into());
        lines.push(format!("    <name>{}</name>", escape_xml(&s.name)));
        lines.push(format!(
            "    <description>{}</description>",
            escape_xml(&s.description)
        ));
        lines.push(format!(
            "    <location>{}</location>",
            escape_xml(&s.file_path)
        ));
        lines.push("  </skill>".into());
    }
    lines.push("</available_skills>".into());
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: &str) -> Yaml {
        Yaml::Str(v.into())
    }

    #[test]
    fn frontmatter_scalars() {
        let doc = "---\na: plain text\nb: >-\n  folded\n  lines\n\n  para\nc: |\n  keep\n  lines\nd: 'it''s'\ne: \"q\\\"x\"\nf: true\ng:\n  nested: 1\nh: [1, 2]\n---\nbody";
        let f = parse_frontmatter(doc).unwrap();
        let get = |k: &str| {
            f.iter()
                .find(|(n, _)| n == k)
                .map(|(_, v)| v.clone())
                .unwrap()
        };
        assert_eq!(get("a"), s("plain text"));
        assert_eq!(get("b"), s("folded lines\npara"));
        assert_eq!(get("c"), s("keep\nlines\n"));
        assert_eq!(get("d"), s("it's"));
        assert_eq!(get("e"), s("q\"x"));
        assert_eq!(get("f"), Yaml::Bool(true));
        assert_eq!(get("g"), Yaml::Other);
        assert_eq!(get("h"), Yaml::Other);
    }

    #[test]
    fn frontmatter_absent_or_unclosed() {
        assert!(parse_frontmatter("no frontmatter").unwrap().is_empty());
        assert!(parse_frontmatter("---\nname: x\n").unwrap().is_empty());
        assert!(parse_frontmatter("---\nname: a\nname: b\n---\n").is_err());
    }

    #[test]
    fn ignore_rules() {
        let ig = Ignore {
            rules: vec![
                (false, "skip/".into()),
                (false, "*.draft.md".into()),
                (true, "keep.draft.md".into()),
            ],
        };
        assert!(ig.ignores("skip/"));
        assert!(ig.ignores("skip/SKILL.md"));
        assert!(!ig.ignores("skip"));
        assert!(ig.ignores("a.draft.md"));
        assert!(!ig.ignores("keep.draft.md"));
        assert!(!ig.ignores("ok/SKILL.md"));
    }
}
