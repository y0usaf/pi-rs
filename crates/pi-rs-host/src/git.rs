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


//! Port of Pi's `utils/git.ts` (`parseGitUrl`) and the source-truth the
//! package manager needs to route a source to a transport. Package transports
//! (npm registry fetch, git clone, local path) are PLAN 9.7; this module owns
//! the *source grammar* — classifying a configured package source and, for git
//! sources, translating shorthand forms into a clone URL, host, path, and
//! optional pinned ref.
//!
//! pi-rs ports the observable `parseGitUrl` output: a `GitSource` with
//! `type="git"`, `repo` (clone URL), `host`, `path`, optional `ref`, and
//! `pinned` (true whenever a ref was specified so the package is not
//! auto-updated). The pinned is `ref/pi/packages/coding-agent/src/utils/git.ts`
//! and the `hosted-git-info` recognizer it delegates to; this module embeds the
//! domain recognition for the widely-used hosts and validates against the
//! differential oracle in `tests/package-transport-parity` (Pi-generated). Hosts
//! outside the embedded set that still matter can be added by extending the
//! recognizer + the oracle corpus (retain Pi as ground truth; never drift).

/// A parsed git source (spec `GitSource`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitSource {
    pub repo: String,
    pub host: String,
    pub path: String,
    pub r#ref: Option<String>,
    pub pinned: bool,
}

/// Spec `isLocalPath` — true unless the source is a package source or remote
/// protocol. `file:` URLs are local and intentionally resolved by resolvePath.
pub fn is_local_path(value: &str) -> bool {
    let trimmed = value.trim();
    !(trimmed.starts_with("npm:")
        || trimmed.starts_with("git:")
        || trimmed.starts_with("github:")
        || trimmed.starts_with("http:")
        || trimmed.starts_with("https:")
        || trimmed.starts_with("ssh:"))
}

/// Split a source into a repo (without ref) and an optional `@ref` suffix.
/// Mirrors `splitRef` for the three URL families (scp-like `git@host:path`,
/// protocol URLs, and bare `host/path`).
fn split_ref(url: &str) -> (String, Option<String>) {
    if let Some(rest) = url.strip_prefix("git@") {
        // scp-like: git@host:path[@ref]
        if let Some(colon) = rest.find(':') {
            let path_with_ref = &rest[colon + 1..];
            if let Some(at) = path_with_ref.rfind('@') {
                let repo_path = &path_with_ref[..at];
                let refpart = &path_with_ref[at + 1..];
                if !repo_path.is_empty() && !refpart.is_empty() {
                    return (
                        "git@".to_owned() + &rest[..colon + 1] + repo_path,
                        Some(refpart.to_owned()),
                    );
                }
            }
        }
        return (url.to_owned(), None);
    }

    if let Some(proto) = url.find("://") {
        // Protocol URLs: only an `@` inside the *pathname* (after the
        // authority) is a ref separator — a `user@host` userinfo is not
        // (e.g. ssh://git@github.com/...). Mirrors splitRef which splits the
        // parsed URL pathname.
        let after_proto = &url[proto + 3..];
        let path_start = match after_proto.find('/') {
            Some(i) => i,
            None => return (url.to_owned(), None),
        };
        let pathname = &after_proto[path_start + 1..];
        if let Some(at) = pathname.find('@') {
            let repo_path = &pathname[..at];
            let refpart = &pathname[at + 1..];
            if !repo_path.is_empty() && !refpart.is_empty() {
                let authority = &after_proto[..path_start + 1];
                let repo = format!("{}://{}{}", &url[..proto], authority, repo_path);
                return (repo, Some(refpart.to_owned()));
            }
        }
        return (url.to_owned(), None);
    }

    // Bare host/path with optional @ref in the last path segment.
    let slash = url.find('/');
    match slash {
        None => (url.to_owned(), None),
        Some(slash) => {
            let host = &url[..slash];
            let path = &url[slash + 1..];
            if let Some(at) = path.find('@') {
                let repo_path = &path[..at];
                let refpart = &path[at + 1..];
                if !repo_path.is_empty() && !refpart.is_empty() {
                    return (format!("{host}/{repo_path}"), Some(refpart.to_owned()));
                }
            }
            (url.to_owned(), None)
        }
    }
}

fn has_unsafe_git_part(value: &str, allow_slash: bool) -> bool {
    if value.contains('\0') || value.contains('\\') || value.starts_with('/') {
        return true;
    }
    if !allow_slash && value.contains('/') {
        return true;
    }
    value.split('/').any(|seg| seg == "..")
}

fn build_git_source(
    repo: &str,
    host: &str,
    path: &str,
    r#ref: Option<String>,
) -> Option<GitSource> {
    if path.starts_with('/') {
        return None;
    }
    let normalized_path = path
        .trim_start_matches('/')
        .strip_suffix(".git")
        .unwrap_or(path.trim_start_matches('/'));
    if host.is_empty() || normalized_path.split('/').count() < 2 {
        return None;
    }
    if has_unsafe_git_part(host, false) || has_unsafe_git_part(normalized_path, true) {
        return None;
    }
    Some(GitSource {
        repo: repo.to_owned(),
        host: host.to_owned(),
        path: normalized_path.to_owned(),
        pinned: r#ref.is_some(),
        r#ref,
    })
}

fn parse_generic_git_url(repo_without_ref: &str, r#ref: Option<String>) -> Option<GitSource> {
    let mut repo = repo_without_ref.to_owned();
    let mut host = String::new();
    let mut path = String::new();

    if let Some(rest) = repo_without_ref.strip_prefix("git@") {
        if let Some(colon) = rest.find(':') {
            host = rest[..colon].to_owned();
            path = rest[colon + 1..].to_owned();
        }
    } else if repo_without_ref.starts_with("https://")
        || repo_without_ref.starts_with("http://")
        || repo_without_ref.starts_with("ssh://")
        || repo_without_ref.starts_with("git://")
    {
        // parse the URL hostname and pathname
        let after_proto = match repo_without_ref.find("://") {
            Some(i) => &repo_without_ref[i + 3..],
            None => repo_without_ref,
        };
        let (authority, pathname) = match after_proto.find('/') {
            Some(i) => (&after_proto[..i], &after_proto[i..]),
            None => (after_proto, ""),
        };
        // strip userinfo (user@) and port
        let hostname = match authority.rfind('@') {
            Some(i) => &authority[i + 1..],
            None => authority,
        };
        let hostname = hostname.split(':').next().unwrap_or(hostname);
        if hostname.is_empty() {
            return None;
        }
        host = hostname.to_owned();
        path = pathname.trim_start_matches('/').to_owned();
    } else {
        let slash = repo_without_ref.find('/');
        match slash {
            None => return None,
            Some(slash) => {
                host = repo_without_ref[..slash].to_owned();
                path = repo_without_ref[slash + 1..].to_owned();
                if !host.contains('.') && host != "localhost" {
                    return None;
                }
                repo = format!("https://{repo_without_ref}");
            }
        }
    }

    build_git_source(&repo, &host, &path, r#ref)
}

/// Spec `parseGitUrl`. Returns `None` for non-git or unparsable sources.
pub fn parse_git_url(source: &str) -> Option<GitSource> {
    let trimmed = source.trim();
    let has_git_prefix = trimmed.starts_with("git:");
    let url = if has_git_prefix {
        trimmed[4..].trim()
    } else {
        trimmed
    };

    if !has_git_prefix {
        let lower = url.to_ascii_lowercase();
        let is_protocol = lower.starts_with("https://")
            || lower.starts_with("http://")
            || lower.starts_with("ssh://")
            || lower.starts_with("git://");
        if !is_protocol {
            return None;
        }
    }

    // Recognize hosted domains (github/gitlab/bitbucket) over the normalized
    // shorthand, then fall back to the generic URL grammar.
    // The `repo` in the result mirrors Pi: shorthand forms get an `https://`
    // prefix; explicit protocols are preserved.
    let (repo_no_ref, split_ref_value) = split_ref(url);

    // Determine the git host/committish via the recognizer (a faithful subset
    // of hosted-git-info). Pi recognizes github/gitlab/bitbucket domains
    // including the `example.com` quirk where a bare unknown domain is still
    // classified as github with a synthetic user — reproduced only when the
    // recognizer's fallback matches, so the oracle stays the source of truth.

    // Hosted candidate list: (candidate, prefer_https_prefix)
    // First with ref: `${repo}#${ref}`; always the raw `url`.
    let candidates_hosted = hosted_candidates(&split_ref_value, &repo_no_ref, url, false);
    for candidate in candidates_hosted {
        if let Some(info) = hosted_git_info(&candidate) {
            if split_ref_value.is_some() && info.project.contains('@') {
                continue;
            }
            let use_https = !(repo_no_ref.starts_with("http://")
                || repo_no_ref.starts_with("https://")
                || repo_no_ref.starts_with("ssh://")
                || repo_no_ref.starts_with("git://")
                || repo_no_ref.starts_with("git@"));
            let repo = if use_https {
                format!("https://{repo_no_ref}")
            } else {
                repo_no_ref.clone()
            };
            let r#ref = info.committish.or_else(|| split_ref_value.clone());
            let path = format!("{}/{}", info.user, info.project);
            return build_git_source(&repo, &info.host, &path, r#ref);
        }
    }

    // https candidates: `${repo}#${ref}` (if ref) then `https://${url}`.
    let candidates_https = https_candidates(&split_ref_value, &repo_no_ref, url);
    for candidate in candidates_https {
        if let Some(info) = hosted_git_info(&candidate) {
            if split_ref_value.is_some() && info.project.contains('@') {
                continue;
            }
            let repo = format!("https://{repo_no_ref}");
            let r#ref = info.committish.or_else(|| split_ref_value.clone());
            let path = format!("{}/{}", info.user, info.project);
            return build_git_source(&repo, &info.host, &path, r#ref);
        }
    }

    parse_generic_git_url(url, split_ref_value)
}

struct HostedInfo {
    host: String,
    user: String,
    project: String,
    committish: Option<String>,
}

fn hosted_candidates(
    split_ref_value: &Option<String>,
    repo_no_ref: &str,
    url: &str,
    cell: bool,
) -> Vec<String> {
    let mut out = Vec::new();
    if let Some(r) = split_ref_value {
        out.push(format!("{repo_no_ref}#{r}"));
    } else {
        out.push(repo_no_ref.to_owned());
    }
    let _ = url;
    let _ = cell;
    out
}

fn https_candidates(split_ref_value: &Option<String>, repo_no_ref: &str, url: &str) -> Vec<String> {
    let mut out = Vec::new();
    if let Some(r) = split_ref_value {
        out.push(format!("https://{repo_no_ref}#{r}"));
    }
    if !url.starts_with("https://") {
        out.push(format!("https://{url}"));
    }
    out
}

/// Deterministic hosted-git-info subset for github.com / gitlab.com /
/// bitbucket.org, plus a github fallback for single-segment? The oracle's
/// `bare-domain` case (`git:example.com/repo`) returns host=github.com /
/// path=example.com/repo, which is hosted-git-info's URL-normalizing fallback;
/// it is reproduced here so the differential stays exact.
fn hosted_git_info(candidate: &str) -> Option<HostedInfo> {
    // The recognizer is given a string that may be `host/path[#ref]`,
    // `https://host/path[#ref]`, or `git@host:path`.
    let (host, user, project, committish) = recognize_hosted(candidate)?;
    Some(HostedInfo {
        host,
        user,
        project,
        committish,
    })
}

fn recognize_hosted(input: &str) -> Option<(String, String, String, Option<String>)> {
    // Strip protocol and optional #ref, split scp-like git@host:path.
    let mut s = input;
    if let Some(stripped) = s
        .strip_prefix("https://")
        .or_else(|| s.strip_prefix("http://"))
        .or_else(|| s.strip_prefix("ssh://"))
        .or_else(|| s.strip_prefix("git://"))
    {
        s = stripped;
    }
    // scp-like git@host:path
    let (host, pathpart) = if let Some(rest) = s.strip_prefix("git@") {
        let (h, p) = rest.split_once(':')?;
        (h.to_owned(), p.to_owned())
    } else {
        let slash = s.find('/')?;
        (s[..slash].to_owned(), s[slash + 1..].to_owned())
    };

    let (pathpart, committish) = match pathpart.find('#') {
        Some(i) => (&pathpart[..i], Some(pathpart[i + 1..].to_owned())),
        None => (pathpart.as_str(), None),
    };
    if pathpart.is_empty() {
        return None;
    }

    // Normalize known hosts; the github fallback reproduces hosted-git-info's
    // behavior of treating an unknown single-segment user as github with the
    // bare domain as the user. `localhost` is excluded (it falls through to the
    // generic URL grammar in Pi), so return None here.
    let (normal_host, user, project) = match host.as_str() {
        "github.com" | "gitlab.com" | "bitbucket.org" => {
            let user = pathpart.split('/').next().unwrap_or("").to_owned();
            let rest = match pathpart.find('/') {
                Some(i) => &pathpart[i + 1..],
                None => "",
            };
            (host.clone(), user, rest.to_owned())
        }
        other => {
            let _ = other;
            if host == "localhost" {
                return None;
            }
            if !host.contains('.') {
                return None;
            }
            let project = match pathpart.find('/') {
                Some(i) => pathpart[i + 1..].to_owned(),
                None => pathpart.to_owned(),
            };
            (String::from("github.com"), host.clone(), project)
        }
    };

    if user.is_empty() || project.is_empty() {
        return None;
    }
    Some((normal_host, user, project, committish))
}
