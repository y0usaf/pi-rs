//! `core/system-prompt.ts` and the prompt half of `core/resource-loader.ts`: context files
//! (AGENTS.md, CLAUDE.md), SYSTEM.md / APPEND_SYSTEM.md, and the sectioned system prompt.

use std::path::Path;

use crate::config::CONFIG_DIR_NAME;
use crate::paths::{PathInput, dirname, join, normalize_path, resolve};
use crate::settings::canonicalize;
use crate::skills::{Skill, format_skills_for_prompt};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContextFile {
    pub path: String,
    pub content: String,
}

fn strip_bom(text: String) -> String {
    match text.strip_prefix('\u{feff}') {
        Some(rest) => rest.to_string(),
        None => text,
    }
}

fn read_text(path: &str) -> std::io::Result<String> {
    std::fs::read(path).map(|b| strip_bom(String::from_utf8_lossy(&b).into_owned()))
}

fn basename(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

const PREAMBLE: &str = "You are an expert coding assistant operating inside pi, a coding agent harness. You help users by reading files, executing commands, editing code, and writing new files.";
const TOOLS_TAIL: &str = "In addition to the tools above, you may have access to other custom tools depending on the project.";
const DOCS: &str = "Pi documentation (read only when the user asks about pi itself, its SDK, extensions, themes, skills, or TUI):
- Main documentation: {readme}
- Additional docs: {docs}
- Examples: {examples} (extensions, custom tools, SDK)
- When reading pi docs or examples, resolve docs/... under Additional docs and examples/... under Examples, not the current working directory
- When asked about: extensions (docs/extensions.md, examples/extensions/), themes (docs/themes.md), skills (docs/skills.md), prompt templates (docs/prompt-templates.md), TUI components (docs/tui.md), keybindings (docs/keybindings.md), SDK integrations (docs/sdk.md), custom providers (docs/custom-provider.md), adding models (docs/models.md), pi packages (docs/packages.md), environment variables (docs/environment-variables.md), MCP servers (docs/mcp.md), codemode scripts and non-LLM models such as classifiers and image models (docs/codemode.md)
- When working on pi topics, read the docs and examples, and follow .md cross-references before implementing
- Always read pi .md files completely and follow links to related docs (e.g., tui.md for TUI API details)";

/// `loadContextFileFromDir`: the first of AGENTS.override.md, AGENTS.md, AGENTS.MD, CLAUDE.md,
/// CLAUDE.MD that is a readable file.
fn context_file_in(dir: &str, warnings: &mut Vec<String>) -> Option<ContextFile> {
    for name in [
        "AGENTS.override.md",
        "AGENTS.md",
        "AGENTS.MD",
        "CLAUDE.md",
        "CLAUDE.MD",
    ] {
        let path = join(dir, name);
        let Ok(meta) = std::fs::metadata(&path) else {
            continue;
        };
        if !meta.is_file() {
            continue;
        }
        match read_text(&path) {
            Ok(content) => return Some(ContextFile { path, content }),
            Err(e) => warnings.push(format!(
                "Warning: Could not read {path}: {}",
                crate::node_fs::message(&e, "open", &path)
            )),
        }
    }
    None
}

struct GitPaths {
    repo_dir: String,
    common_git_dir: String,
}

/// `findGitPaths` (footer-data-provider.ts).
fn find_git_paths(cwd: &str) -> Option<GitPaths> {
    let mut dir = cwd.to_string();
    loop {
        let git = join(&dir, ".git");
        if let Ok(meta) = std::fs::metadata(&git) {
            if meta.is_file() {
                let content = std::fs::read_to_string(&git).ok()?;
                let content = content.trim();
                if let Some(target) = content.strip_prefix("gitdir: ") {
                    let git_dir = resolve(&dir, target.trim());
                    if !Path::new(&join(&git_dir, "HEAD")).exists() {
                        return None;
                    }
                    let common = match std::fs::read_to_string(join(&git_dir, "commondir")) {
                        Ok(c) => resolve(&git_dir, c.trim()),
                        Err(_) => git_dir,
                    };
                    return Some(GitPaths {
                        repo_dir: dir,
                        common_git_dir: common,
                    });
                }
            } else if meta.is_dir() {
                if !Path::new(&join(&git, "HEAD")).exists() {
                    return None;
                }
                return Some(GitPaths {
                    repo_dir: dir,
                    common_git_dir: git,
                });
            }
        }
        let parent = dirname(&dir);
        if parent == dir {
            return None;
        }
        dir = parent;
    }
}

/// `findShadowedContextFile`: in a linked worktree nested inside its main checkout, the main
/// checkout's context file that the worktree's own copy stands for.
fn shadowed_context_file(cwd: &str) -> Option<String> {
    let git = find_git_paths(cwd)?;
    let common = canonicalize(&git.common_git_dir);
    let worktree_root = canonicalize(&git.repo_dir);
    let main_root = dirname(&common);
    if !worktree_root.starts_with(&format!("{main_root}/")) {
        return None;
    }
    if canonicalize(&join(&main_root, ".git")) != common {
        return None;
    }
    let own = context_file_in(&worktree_root, &mut Vec::new())?;
    Some(join(&main_root, basename(&own.path)))
}

/// `loadProjectContextFiles`: the agent directory's file, then the cwd's ancestors' from the
/// root down.
pub fn load_context_files(
    cwd: &str,
    agent_dir: &str,
    warnings: &mut Vec<String>,
) -> Vec<ContextFile> {
    let cwd = resolve("/", &normalize_path(cwd, PathInput::default()));
    let agent_dir = resolve("/", &normalize_path(agent_dir, PathInput::default()));
    let mut files: Vec<ContextFile> = Vec::new();
    if let Some(global) = context_file_in(&agent_dir, warnings) {
        files.push(global);
    }
    let shadowed = shadowed_context_file(&cwd);
    let mut ancestors: Vec<ContextFile> = Vec::new();
    let mut dir = cwd;
    loop {
        if let Some(file) = context_file_in(&dir, warnings) {
            let is_shadowed = shadowed
                .as_ref()
                .is_some_and(|s| join(&canonicalize(&dir), basename(&file.path)) == *s);
            let seen = files.iter().chain(&ancestors).any(|f| f.path == file.path);
            if !is_shadowed && !seen {
                ancestors.insert(0, file);
            }
        }
        let parent = dirname(&dir);
        if parent == dir {
            break;
        }
        dir = parent;
    }
    files.extend(ancestors);
    files
}

/// `resolvePromptInput`: a path's contents when it exists, else the text itself.
pub fn resolve_prompt_input(input: &str, description: &str, warnings: &mut Vec<String>) -> String {
    if !Path::new(input).exists() {
        return input.to_string();
    }
    match read_text(input) {
        Ok(text) => text,
        Err(e) => {
            warnings.push(format!(
                "Warning: Could not read {description} file {input}: Error: {}",
                crate::node_fs::message(&e, "read", input)
            ));
            input.to_string()
        }
    }
}

/// `discoverSystemPromptFile` / `discoverAppendSystemPromptFile`: the project's
/// `.pi/<name>` when trusted, else the agent directory's.
pub fn discover_prompt_file(
    cwd: &str,
    agent_dir: &str,
    name: &str,
    project_trusted: bool,
) -> Option<String> {
    let project = join(cwd, &format!("{CONFIG_DIR_NAME}/{name}"));
    if project_trusted && Path::new(&project).exists() {
        return Some(project);
    }
    let global = join(agent_dir, name);
    Path::new(&global).exists().then_some(global)
}

/// `getPackageDir()`: `$PI_PACKAGE_DIR`, else the directory holding the executable. Upstream
/// falls back to its npm package; this port has none, so the docs paths name the binary's directory.
pub fn package_dir() -> String {
    match std::env::var("PI_PACKAGE_DIR") {
        Ok(dir) if !dir.is_empty() => normalize_path(&dir, PathInput::default()),
        _ => std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|d| d.to_string_lossy().into_owned()))
            .unwrap_or_else(|| ".".into()),
    }
}

#[derive(Debug, Clone, Default)]
pub struct PromptInput {
    pub custom_prompt: Option<String>,
    pub selected_tools: Vec<String>,
    /// Tool snippets in tool registry order (tools without a snippet are not listed).
    pub tool_snippets: Vec<(String, String)>,
    pub tool_guidelines: Vec<(String, Vec<String>)>,
    pub prompt_guidelines: Vec<String>,
    pub append_system_prompt: String,
    pub cwd: String,
    pub context_files: Vec<ContextFile>,
    pub skills: Vec<Skill>,
    pub package_dir: String,
}

fn build_rules(tools: &[String], guidelines: &[(String, Vec<String>)], extra: &[String]) -> String {
    let mut rules: Vec<String> = Vec::new();
    let mut add = |rule: &str| {
        let r = rule.trim();
        if !r.is_empty() && !rules.iter().any(|x| x == r) {
            rules.push(r.to_string());
        }
    };
    let has = |n: &str| tools.iter().any(|t| t == n);
    if (has("bash") || has("powershell")) && !has("grep") && !has("find") && !has("ls") {
        if has("bash") && has("powershell") {
            add(
                "Use bash or PowerShell for file operations like listing, searching, and finding files",
            );
        } else if has("powershell") {
            add("Use PowerShell for file operations like listing, searching, and finding files");
        } else {
            add("Use bash for file operations like ls, rg, find");
        }
    }
    for name in tools {
        for (_, list) in guidelines.iter().filter(|(n, _)| n == name) {
            for rule in list {
                add(rule);
            }
        }
    }
    for rule in extra {
        add(rule);
    }
    add("Be concise in your responses");
    add("Show file paths clearly when working with files");
    rules
        .iter()
        .map(|r| format!("- {r}"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// `buildSystemPromptSections`: ordered `(name, text)`; every section but `preamble` is
/// wrapped in a tag of its name.
pub fn build_sections(input: &PromptInput) -> Vec<(String, String)> {
    let mut raw: Vec<(&str, String)> = Vec::new();
    match input.custom_prompt.as_deref().filter(|p| !p.is_empty()) {
        Some(custom) => raw.push(("preamble", custom.to_string())),
        None => {
            raw.push(("preamble", PREAMBLE.into()));
            let listed: Vec<String> = input
                .selected_tools
                .iter()
                .filter_map(|name| {
                    input
                        .tool_snippets
                        .iter()
                        .find(|(n, s)| n == name && !s.is_empty())
                        .map(|(n, s)| format!("- {n}: {s}"))
                })
                .collect();
            let tools = if listed.is_empty() {
                "(none)".to_string()
            } else {
                listed.join("\n")
            };
            raw.push(("tools", format!("{tools}\n\n{TOOLS_TAIL}")));
            raw.push((
                "rules",
                build_rules(
                    &input.selected_tools,
                    &input.tool_guidelines,
                    &input.prompt_guidelines,
                ),
            ));
            let pkg = &input.package_dir;
            let docs = DOCS
                .replace("{readme}", &resolve("/", &join(pkg, "README.md")))
                .replace("{docs}", &resolve("/", &join(pkg, "docs")))
                .replace("{examples}", &resolve("/", &join(pkg, "examples")));
            raw.push(("docs", docs));
        }
    }
    if !input.append_system_prompt.is_empty() {
        raw.push(("addendum", input.append_system_prompt.clone()));
    }
    if !input.context_files.is_empty() {
        let mut parts = vec!["Project-specific instructions and guidelines:".to_string()];
        parts.extend(input.context_files.iter().map(|f| {
            format!(
                "<project_instructions path=\"{}\">\n{}\n</project_instructions>",
                f.path, f.content
            )
        }));
        raw.push(("project_context", parts.join("\n\n")));
    }
    let reader = ["read", "bash"]
        .into_iter()
        .find(|r| input.selected_tools.iter().any(|t| t == r));
    if let Some(reader) = reader {
        let text = format_skills_for_prompt(&input.skills, reader);
        let text = text.trim();
        if !text.is_empty() {
            raw.push(("skills", text.to_string()));
        }
    }
    raw.push(("cwd", input.cwd.replace('\\', "/")));
    raw.into_iter()
        .map(|(name, text)| {
            let text = if name == "preamble" {
                text
            } else {
                format!("<{name}>\n{text}\n</{name}>")
            };
            (name.to_string(), text)
        })
        .collect()
}

/// `buildSystemPrompt`: the sections joined as the transcript's system message renders them.
pub fn build_system_prompt(input: &PromptInput) -> String {
    build_sections(input)
        .into_iter()
        .map(|(_, t)| t)
        .filter(|t| !t.is_empty())
        .collect::<Vec<_>>()
        .join("\n\n")
}

pub struct Assembled {
    pub input: PromptInput,
    pub warnings: Vec<String>,
    pub skill_diagnostics: Vec<crate::skills::Diagnostic>,
}

pub struct AssembleOptions<'a> {
    pub cwd: &'a str,
    pub agent_dir: &'a str,
    pub project_trusted: bool,
    /// `--system-prompt`, text or a file.
    pub system_prompt: Option<&'a str>,
    /// `--append-system-prompt`, each text or a file.
    pub append_system_prompt: Option<&'a [String]>,
    pub no_context_files: bool,
    pub no_skills: bool,
    pub skill_paths: &'a [String],
    /// The active tools, in selection order.
    pub tools: &'a [String],
}

/// What `DefaultResourceLoader.reload()` and `AgentSession._rebuildSystemPrompt` gather for the
/// built-in tools: the prompt's inputs, and the warnings upstream prints while reading them.
pub fn assemble(o: &AssembleOptions) -> Assembled {
    let cwd = resolve("/", &normalize_path(o.cwd, PathInput::default()));
    let agent_dir = resolve("/", &normalize_path(o.agent_dir, PathInput::default()));
    let mut warnings = Vec::new();
    let context_files = if o.no_context_files {
        Vec::new()
    } else {
        load_context_files(&cwd, &agent_dir, &mut warnings)
    };
    let system_source = o
        .system_prompt
        .map(str::to_string)
        .or_else(|| discover_prompt_file(&cwd, &agent_dir, "SYSTEM.md", o.project_trusted));
    let custom_prompt =
        system_source.map(|s| resolve_prompt_input(&s, "system prompt", &mut warnings));
    let append_sources: Vec<String> = match o.append_system_prompt {
        Some(list) => list.to_vec(),
        None => discover_prompt_file(&cwd, &agent_dir, "APPEND_SYSTEM.md", o.project_trusted)
            .into_iter()
            .collect(),
    };
    let append: Vec<String> = append_sources
        .iter()
        .map(|s| resolve_prompt_input(s, "append system prompt", &mut warnings))
        .collect();
    let loaded = crate::skills::load_skills(&crate::skills::SkillsInput {
        cwd: &cwd,
        agent_dir: &agent_dir,
        project_trusted: o.project_trusted,
        no_skills: o.no_skills,
        cli_paths: o.skill_paths,
    });
    let mut tool_snippets = Vec::new();
    let mut tool_guidelines = Vec::new();
    for name in crate::tools::ALL_TOOL_NAMES {
        if let Some(spec) = crate::tools::spec(name) {
            if !spec.prompt.snippet.is_empty() {
                tool_snippets.push((name.to_string(), spec.prompt.snippet.to_string()));
            }
            tool_guidelines.push((
                name.to_string(),
                spec.prompt
                    .guidelines
                    .iter()
                    .map(|g| g.to_string())
                    .collect(),
            ));
        }
    }
    let known: Vec<String> = o
        .tools
        .iter()
        .filter(|t| crate::tools::spec(t).is_some())
        .cloned()
        .collect();
    Assembled {
        input: PromptInput {
            custom_prompt,
            selected_tools: known,
            tool_snippets,
            tool_guidelines,
            prompt_guidelines: Vec::new(),
            append_system_prompt: append.join("\n\n"),
            cwd,
            context_files,
            skills: loaded.skills,
            package_dir: package_dir(),
        },
        warnings,
        skill_diagnostics: loaded.diagnostics,
    }
}
