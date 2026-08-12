//! Git-branch discovery for the live footer data provider — spec
//! `core/footer-data-provider.ts` (`findGitPaths` + `resolveGitBranch`).
//!
//! Pi's default footer shows the current git branch, discovered by walking up
//! from the cwd for a `.git` directory, or a `.git` *file* (a worktree link
//! `gitdir: <path>`), then reading `HEAD`. A detached HEAD (a direct commit
//! in HEAD) reports `"detached"`; a `.invalid` ref marker falls back to
//! asking `git symbolic-ref`. This module supplies the pure path-walk +
//! HEAD-parse decision core and exposes it to Lua as `pi.git.current_branch`
//! — the mechanism the differential oracle pins.

use mlua::prelude::*;

/// Spec `GitPaths` — the resolved repository metadata.
#[allow(dead_code)]
pub(crate) struct GitPaths {
    pub repo_dir: String,
    common_git_dir: String,
    pub head_path: String,
}

/// Pure port of Pi's `findGitPaths(cwd)`: walk up from `cwd`; a `.git`
/// directory yields that repo, a `.git` file with a `gitdir: <path>` line
/// yields the linked repo (resolving through `commondir` when present).
/// Returns `None` when no repo is found. Free of process/IO side effects so
/// it can be pinned directly; the caller supplies filesystem reads.
pub(crate) fn find_git_paths(
    cwd: &str,
    read_file: &dyn Fn(&str) -> Result<String, std::io::Error>,
    file_type: &dyn Fn(&str) -> Result<String, std::io::Error>,
    resolve: &dyn Fn(&str, &str) -> String,
) -> Option<GitPaths> {
    let mut dir = cwd.to_owned();
    loop {
        let git_path = format!("{dir}/.git");
        let exists = file_type(&git_path).unwrap_or_default();
        if exists == "file" {
            let content = read_file(&git_path).ok()?;
            let content = content.trim();
            if let Some(rest) = content.strip_prefix("gitdir: ") {
                let git_dir = resolve(dir.as_str(), rest.trim());
                let head_path = format!("{git_dir}/HEAD");
                if file_type(&head_path).is_err() {
                    return None;
                }
                let common_dir_path = format!("{git_dir}/commondir");
                let common_git_dir = match file_type(&common_dir_path) {
                    Ok(_) => {
                        let text = read_file(&common_dir_path).ok()?;
                        resolve(&git_dir, text.trim())
                    }
                    Err(_) => git_dir,
                };
                return Some(GitPaths {
                    repo_dir: dir,
                    common_git_dir,
                    head_path,
                });
            }
        } else if exists == "dir" {
            let head_path = format!("{git_path}/HEAD");
            if file_type(&head_path).is_err() {
                return None;
            }
            return Some(GitPaths {
                repo_dir: dir,
                common_git_dir: git_path,
                head_path,
            });
        }
        let parent = dirname(&dir);
        if parent == dir {
            return None;
        }
        dir = parent;
    }
}

/// Node-style `dirname`.
fn dirname(path: &str) -> String {
    let trimmed = path.trim_end_matches('/');
    if trimmed.is_empty() {
        return "/".to_owned();
    }
    match trimmed.rfind('/') {
        Some(0) => "/".to_owned(),
        Some(i) => trimmed[..i].to_owned(),
        None => ".".to_owned(),
    }
}

/// Pure port of Pi's `resolveGitBranchSync(gitPaths)`:
///   - no repo -> None;
///   - `HEAD == ref: refs/heads/<branch>` -> branch;
///   - branch `.invalid` -> fall back to the git symbolic-ref result or
///     `"detached"`;
///   - any other (detached commit) HEAD -> `"detached"`.
pub(crate) fn resolve_git_branch_sync(
    paths: &GitPaths,
    read_file: &dyn Fn(&str) -> Result<String, std::io::Error>,
    symbolic_ref: &dyn Fn(&str) -> Result<Option<String>, std::io::Error>,
) -> Option<String> {
    let content = read_file(&paths.head_path).ok()?;
    let content = content.trim();
    if let Some(rest) = content.strip_prefix("ref: ") {
        if let Some(branch) = rest.strip_prefix("refs/heads/") {
            if branch == ".invalid" {
                return Some(
                    symbolic_ref(&paths.repo_dir)
                        .ok()
                        .flatten()
                        .unwrap_or_else(|| "detached".to_owned()),
                );
            }
            return Some(branch.to_owned());
        }
        // A ref to a non-head namespace (rare) resolves to detached in Pi? Pi
        // only handles refs/heads; a different ref is treated as detached (the
        // `resolveBranchWithGitSync` fallback is only for `.invalid`).
        return Some("detached".to_owned());
    }
    Some("detached".to_owned())
}

/// Install `pi.git.current_branch(cwd)` on the API table. Synchronous (reads
/// `HEAD` directly; only the `.invalid` fallback spawns `git`), matching how
/// the footer renders branch data without a process hop in the common case.
pub(crate) fn install(lua: &Lua, pi: &mlua::Table) -> mlua::Result<()> {
    let git = lua.create_table()?;
    git.set(
        "current_branch",
        lua.create_function(|lua, cwd: String| -> mlua::Result<mlua::Value> {
            let read_file = |p: &str| std::fs::read_to_string(p);
            let file_type = |p: &str| -> Result<String, std::io::Error> {
                let md = std::fs::metadata(p)?;
                Ok(if md.is_dir() {
                    "dir".to_owned()
                } else if md.is_file() {
                    "file".to_owned()
                } else {
                    "other".to_owned()
                })
            };
            let resolve = |base: &str, p: &str| {
                // Node `path.resolve(dir, p)`: an absolute `p` wins as-is.
                if p.starts_with('/')
                    || (cfg!(windows)
                        && p.len() >= 2
                        && p[1..].starts_with(':')
                        && (p.starts_with('/') || p.starts_with('\\')))
                {
                    p.to_owned()
                } else {
                    let joined = format!("{base}/{p}");
                    std::path::Path::new(&joined).to_string_lossy().into_owned()
                }
            };
            let Some(paths) = find_git_paths(&cwd, &read_file, &file_type, &resolve) else {
                return Ok(mlua::Value::Nil);
            };
            let symbolic_ref = |repo_dir: &str| -> Result<Option<String>, std::io::Error> {
                let out = std::process::Command::new("git")
                    .arg("--no-optional-locks")
                    .arg("symbolic-ref")
                    .arg("--quiet")
                    .arg("--short")
                    .arg("HEAD")
                    .current_dir(repo_dir)
                    .stdin(std::process::Stdio::null())
                    .stdout(std::process::Stdio::piped())
                    .stderr(std::process::Stdio::null())
                    .output()?;
                if !out.status.success() {
                    return Ok(None);
                }
                let branch = String::from_utf8_lossy(&out.stdout).trim().to_owned();
                Ok(if branch.is_empty() {
                    None
                } else {
                    Some(branch)
                })
            };
            let branch = resolve_git_branch_sync(&paths, &read_file, &symbolic_ref);
            match branch {
                Some(b) => Ok(mlua::Value::String(lua.create_string(&b)?)),
                None => Ok(mlua::Value::Nil),
            }
        })?,
    )?;
    pi.set("git", git)?;
    Ok(())
}
