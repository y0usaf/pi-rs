//! OS bindings: `pi.fs`, `pi.path`, `pi.env`, `pi.cwd()`.
//!
//! No spec counterpart file — in pi these are ambient Node (`node:fs`,
//! `node:path`, `process.env`, the loader's `cwd`); divergence 1 makes
//! them explicit bindings so example translations stay mechanical
//! (`fs.readFileSync(p)` → `pi.fs.read_file(p)`).
//!
//! - `pi.fs.*` is async through tokio on the coroutine seam: callers just
//!   call; the suspension while the host does I/O is watchdog-free.
//!   Errors are thrown as Lua errors (Node's sync fs throws → `pcall` is
//!   the `try/catch` translation); `exists` never throws. `read_file` is
//!   the `readFileSync(p, "utf-8")` translation (UTF-8-strict);
//!   `read_bytes` is the `readFileSync(p)` Buffer translation — a
//!   binary-safe Lua string. `realpath` is Node `fs.realpath` (throws
//!   when the path does not exist).
//! - `pi.path.*` matches Node's `path.posix` semantics, pinned by the
//!   unit tests below (examples from the Node docs).
//! - `pi.env` is a read-only view of the process environment
//!   (`process.env.HOME` → `pi.env.HOME`); mutation raises.
//! - `pi.cwd()` is the host cwd injected at startup (spec: the loader's
//!   `cwd` parameter).

use crate::effects::{EffectOptions, EffectRequest, EffectResult, FsRequest};

async fn fs_request(
    hub: &crate::effects::EffectHub,
    lua: &mlua::Lua,
    request: FsRequest,
) -> mlua::Result<crate::effects::FsResult> {
    let scope = hub.scope(lua)?;
    let result = hub
        .request(
            scope,
            EffectRequest::Fs(
                request,
                EffectOptions::bounded(std::time::Duration::from_secs(30)),
            ),
            crate::effects::cancellation(),
        )
        .await
        .map_err(crate::effects::lua_error)?;
    match result {
        EffectResult::Fs(result) => Ok(result),
        _ => Err(mlua::Error::runtime(
            "filesystem effect returned the wrong result",
        )),
    }
}

// ---------------------------------------------------------------------------
// path — Node path.posix semantics
// ---------------------------------------------------------------------------

pub(crate) fn normalize(path: &str) -> String {
    if path.is_empty() {
        return ".".to_owned();
    }
    let absolute = path.starts_with('/');
    let trailing = path.len() > 1 && path.ends_with('/');
    let mut stack: Vec<&str> = Vec::new();
    for part in path.split('/') {
        match part {
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
    let mut res = stack.join("/");
    if res.is_empty() {
        if absolute {
            return "/".to_owned();
        }
        res = ".".to_owned();
    }
    if trailing && !res.ends_with('/') {
        res.push('/');
    }
    if absolute { format!("/{res}") } else { res }
}

pub(crate) fn join(parts: &[String]) -> String {
    let joined: Vec<&str> = parts
        .iter()
        .filter(|p| !p.is_empty())
        .map(String::as_str)
        .collect();
    if joined.is_empty() {
        return ".".to_owned();
    }
    normalize(&joined.join("/"))
}

pub(crate) fn dirname(path: &str) -> String {
    if path.is_empty() {
        return ".".to_owned();
    }
    let has_root = path.starts_with('/');
    let bytes = path.as_bytes();
    let mut end: Option<usize> = None;
    let mut matched_slash = true;
    let mut i = bytes.len();
    while i > 1 {
        i -= 1;
        if bytes[i] == b'/' {
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
            if has_root {
                "/".to_owned()
            } else {
                ".".to_owned()
            }
        }
        Some(1) if has_root => "//".to_owned(),
        Some(e) => path[..e].to_owned(),
    }
}

pub(crate) fn basename(path: &str, suffix: Option<&str>) -> String {
    let bytes = path.as_bytes();
    let mut start = 0usize;
    let mut end: Option<usize> = None;
    let mut matched_slash = true;
    let mut i = bytes.len();
    while i > 0 {
        i -= 1;
        if bytes[i] == b'/' {
            if !matched_slash {
                start = i + 1;
                break;
            }
        } else if end.is_none() {
            matched_slash = false;
            end = Some(i + 1);
        }
    }
    let Some(end) = end else {
        return String::new();
    };
    let mut res = &path[start..end];
    if let Some(sfx) = suffix
        && res != sfx
        && let Some(stripped) = res.strip_suffix(sfx)
    {
        res = stripped;
    }
    res.to_owned()
}

pub(crate) fn extname(path: &str) -> String {
    let base = basename(path, None);
    if base == ".." {
        return String::new();
    }
    match base.rfind('.') {
        None | Some(0) => String::new(),
        Some(i) => base[i..].to_owned(),
    }
}

pub(crate) fn resolve(parts: &[String], cwd: &str) -> String {
    let mut resolved = String::new();
    let mut absolute = false;
    for part in parts
        .iter()
        .rev()
        .map(String::as_str)
        .chain(std::iter::once(cwd))
    {
        if absolute {
            break;
        }
        if part.is_empty() {
            continue;
        }
        resolved = format!("{part}/{resolved}");
        absolute = part.starts_with('/');
    }
    let mut norm = normalize(&resolved);
    while norm.len() > 1 && norm.ends_with('/') {
        norm.pop();
    }
    if norm.is_empty() {
        ".".to_owned()
    } else {
        norm
    }
}

pub(crate) fn relative(from: &str, to: &str, cwd: &str) -> String {
    let from = resolve(std::slice::from_ref(&from.to_owned()), cwd);
    let to = resolve(std::slice::from_ref(&to.to_owned()), cwd);
    if from == to {
        return String::new();
    }
    let f: Vec<&str> = from.split('/').filter(|s| !s.is_empty()).collect();
    let t: Vec<&str> = to.split('/').filter(|s| !s.is_empty()).collect();
    let common = f.iter().zip(t.iter()).take_while(|(a, b)| a == b).count();
    let mut out: Vec<&str> = Vec::new();
    out.extend(std::iter::repeat_n("..", f.len() - common));
    out.extend(&t[common..]);
    out.join("/")
}

// ---------------------------------------------------------------------------
// install
// ---------------------------------------------------------------------------

fn install_fs(
    lua: &mlua::Lua,
    pi: &mlua::Table,
    hub: crate::effects::EffectHub,
) -> mlua::Result<()> {
    let fs = lua.create_table()?;

    let read_hub = hub.clone();
    fs.set(
        "read_file",
        lua.create_async_function(move |lua, path: String| {
            let hub = read_hub.clone();
            async move {
                let crate::effects::FsResult::Bytes(bytes) =
                    fs_request(&hub, &lua, FsRequest::Read { path, bytes: false }).await?
                else {
                    return Err(mlua::Error::runtime("invalid read result"));
                };
                String::from_utf8(bytes).map_err(mlua::Error::external)
            }
        })?,
    )?;

    let write_hub = hub.clone();
    fs.set(
        "write_file",
        lua.create_async_function(move |lua, (path, contents): (String, mlua::String)| {
            let hub = write_hub.clone();
            async move {
                fs_request(
                    &hub,
                    &lua,
                    FsRequest::Write {
                        path,
                        contents: contents.as_bytes().to_vec(),
                    },
                )
                .await?;
                Ok(())
            }
        })?,
    )?;

    let append_hub = hub.clone();
    fs.set(
        "append_file",
        lua.create_async_function(move |lua, (path, contents): (String, mlua::String)| {
            let hub = append_hub.clone();
            async move {
                fs_request(
                    &hub,
                    &lua,
                    FsRequest::Append {
                        path,
                        contents: contents.as_bytes().to_vec(),
                    },
                )
                .await?;
                Ok(())
            }
        })?,
    )?;

    fs.set(
        "tmpdir",
        lua.create_function(|_, ()| Ok(std::env::temp_dir().to_string_lossy().into_owned()))?,
    )?;

    let temp_hub = hub.clone();
    fs.set(
        "create_temp_file",
        lua.create_async_function(move |lua, (prefix, contents): (String, mlua::String)| {
            let hub = temp_hub.clone();
            async move {
                match fs_request(
                    &hub,
                    &lua,
                    FsRequest::CreateTempFile {
                        prefix,
                        contents: contents.as_bytes().to_vec(),
                    },
                )
                .await?
                {
                    crate::effects::FsResult::Path(path) => Ok(path),
                    _ => Err(mlua::Error::runtime("invalid temporary-file result")),
                }
            }
        })?,
    )?;

    let bytes_hub = hub.clone();
    fs.set(
        "read_bytes",
        lua.create_async_function(move |lua, path: String| {
            let hub = bytes_hub.clone();
            async move {
                let crate::effects::FsResult::Bytes(bytes) =
                    fs_request(&hub, &lua, FsRequest::Read { path, bytes: true }).await?
                else {
                    return Err(mlua::Error::runtime("invalid read result"));
                };
                lua.create_string(&bytes)
            }
        })?,
    )?;

    let exists_hub = hub.clone();
    fs.set(
        "exists",
        lua.create_async_function(move |lua, path: String| {
            let hub = exists_hub.clone();
            async move {
                match fs_request(&hub, &lua, FsRequest::Exists { path }).await? {
                    crate::effects::FsResult::Bool(value) => Ok(value),
                    _ => Err(mlua::Error::runtime("invalid exists result")),
                }
            }
        })?,
    )?;

    let dir_hub = hub.clone();
    fs.set(
        "read_dir",
        lua.create_async_function(move |lua, path: String| {
            let hub = dir_hub.clone();
            async move {
                let crate::effects::FsResult::Names(names) =
                    fs_request(&hub, &lua, FsRequest::ReadDir { path }).await?
                else {
                    return Err(mlua::Error::runtime("invalid directory result"));
                };
                let result = lua.create_table()?;
                for name in names {
                    result.push(name)?;
                }
                Ok(result)
            }
        })?,
    )?;

    let stat_hub = hub.clone();
    fs.set(
        "stat",
        lua.create_async_function(move |lua, path: String| {
            let hub = stat_hub.clone();
            async move {
                let crate::effects::FsResult::Stat(value) =
                    fs_request(&hub, &lua, FsRequest::Stat { path }).await?
                else {
                    return Err(mlua::Error::runtime("invalid stat result"));
                };
                let result = lua.create_table()?;
                result.set("type", value.kind)?;
                result.set("size", value.size)?;
                result.set("modified_ms", value.modified_ms)?;
                Ok(result)
            }
        })?,
    )?;

    let mkdir_hub = hub.clone();
    fs.set(
        "mkdir",
        lua.create_async_function(move |lua, path: String| {
            let hub = mkdir_hub.clone();
            async move {
                fs_request(&hub, &lua, FsRequest::Mkdir { path }).await?;
                Ok(())
            }
        })?,
    )?;

    let real_hub = hub.clone();
    fs.set(
        "realpath",
        lua.create_async_function(move |lua, path: String| {
            let hub = real_hub.clone();
            async move {
                match fs_request(&hub, &lua, FsRequest::Realpath { path }).await? {
                    crate::effects::FsResult::Path(path) => Ok(path),
                    _ => Err(mlua::Error::runtime("invalid realpath result")),
                }
            }
        })?,
    )?;

    fs.set(
        "remove_file",
        lua.create_async_function(move |lua, path: String| {
            let hub = hub.clone();
            async move {
                fs_request(&hub, &lua, FsRequest::RemoveFile { path }).await?;
                Ok(())
            }
        })?,
    )?;

    pi.set("fs", fs)
}

fn install_path(lua: &mlua::Lua, pi: &mlua::Table, cwd: &str) -> mlua::Result<()> {
    let path = lua.create_table()?;
    path.set("sep", "/")?;
    path.set(
        "join",
        lua.create_function(|_, parts: mlua::Variadic<String>| Ok(join(&parts)))?,
    )?;
    path.set(
        "normalize",
        lua.create_function(|_, p: String| Ok(normalize(&p)))?,
    )?;
    path.set(
        "dirname",
        lua.create_function(|_, p: String| Ok(dirname(&p)))?,
    )?;
    path.set(
        "basename",
        lua.create_function(|_, (p, suffix): (String, Option<String>)| {
            Ok(basename(&p, suffix.as_deref()))
        })?,
    )?;
    path.set(
        "extname",
        lua.create_function(|_, p: String| Ok(extname(&p)))?,
    )?;
    path.set(
        "is_absolute",
        lua.create_function(|_, p: String| Ok(p.starts_with('/')))?,
    )?;
    let resolve_cwd = cwd.to_owned();
    path.set(
        "resolve",
        lua.create_function(move |_, parts: mlua::Variadic<String>| {
            Ok(resolve(&parts, &resolve_cwd))
        })?,
    )?;
    let relative_cwd = cwd.to_owned();
    path.set(
        "relative",
        lua.create_function(move |_, (from, to): (String, String)| {
            Ok(relative(&from, &to, &relative_cwd))
        })?,
    )?;
    pi.set("path", path)?;
    Ok(())
}

fn install_env(lua: &mlua::Lua, pi: &mlua::Table) -> mlua::Result<()> {
    let env = lua.create_table()?;
    let mt = lua.create_table()?;
    mt.set(
        "__index",
        lua.create_function(|_, (_env, key): (mlua::Table, String)| Ok(std::env::var(&key).ok()))?,
    )?;
    mt.set(
        "__newindex",
        lua.create_function(
            |_,
             (_env, _key, _value): (mlua::Table, mlua::Value, mlua::Value)|
             -> mlua::Result<()> { Err(mlua::Error::runtime("pi.env is read-only")) },
        )?,
    )?;
    env.set_metatable(Some(mt))?;
    pi.set("env", env)?;
    Ok(())
}

/// Install `pi.fs`, `pi.path`, `pi.env`, and `pi.cwd()` on the API table.
pub(crate) fn install(
    lua: &mlua::Lua,
    pi: &mlua::Table,
    cwd: &str,
    hub: crate::effects::EffectHub,
) -> mlua::Result<()> {
    install_fs(lua, pi, hub)?;
    install_path(lua, pi, cwd)?;
    install_env(lua, pi)?;
    let host_cwd = cwd.to_owned();
    pi.set(
        "cwd",
        lua.create_function(move |_, ()| Ok(host_cwd.clone()))?,
    )?;
    // Node `process.platform` vocabulary (the spec's platform switch
    // points — click hints, alt→option display) as an OS binding.
    pi.set(
        "platform",
        lua.create_function(|_, ()| {
            Ok(if cfg!(target_os = "macos") {
                "darwin"
            } else if cfg!(windows) {
                "win32"
            } else {
                "linux"
            })
        })?,
    )?;
    // Port of `utils/open-browser.ts` — platform handler launch as an OS
    // binding. Deliberately never a shell (the spec's injection note);
    // launch is best-effort and failures are swallowed — callers always
    // present the target to the user as well.
    pi.set(
        "open_browser",
        lua.create_function(|_, target: String| {
            let (cmd, args): (&str, Vec<&str>) = if cfg!(target_os = "macos") {
                ("open", vec![target.as_str()])
            } else if cfg!(windows) {
                (
                    "rundll32",
                    vec!["url.dll,FileProtocolHandler", target.as_str()],
                )
            } else {
                ("xdg-open", vec![target.as_str()])
            };
            let _ = std::process::Command::new(cmd)
                .args(args)
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn();
            Ok(())
        })?,
    )?;
    Ok(())
}

// ---------------------------------------------------------------------------
// tests — Node path.posix examples from the Node docs, pinned
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn s(parts: &[&str]) -> Vec<String> {
        parts.iter().map(|p| (*p).to_owned()).collect()
    }

    #[test]
    fn normalize_matches_node() {
        assert_eq!(normalize("/foo/bar//baz/asdf/quux/.."), "/foo/bar/baz/asdf");
        assert_eq!(normalize(""), ".");
        assert_eq!(normalize("."), ".");
        assert_eq!(normalize("./"), "./");
        assert_eq!(normalize("/"), "/");
        assert_eq!(normalize("/.."), "/");
        assert_eq!(normalize("../a"), "../a");
        assert_eq!(normalize("a/../../b"), "../b");
        assert_eq!(normalize("a//b/"), "a/b/");
    }

    #[test]
    fn join_matches_node() {
        assert_eq!(
            join(&s(&["/foo", "bar", "baz/asdf", "quux", ".."])),
            "/foo/bar/baz/asdf"
        );
        assert_eq!(join(&s(&[])), ".");
        assert_eq!(join(&s(&[""])), ".");
        assert_eq!(join(&s(&["a", "", "b"])), "a/b");
    }

    #[test]
    fn dirname_matches_node() {
        assert_eq!(dirname("/foo/bar/baz/asdf/quux"), "/foo/bar/baz/asdf");
        assert_eq!(dirname("/a/b/"), "/a");
        assert_eq!(dirname("a"), ".");
        assert_eq!(dirname("/"), "/");
        assert_eq!(dirname(""), ".");
    }

    #[test]
    fn basename_matches_node() {
        assert_eq!(basename("/foo/bar/baz/asdf/quux.html", None), "quux.html");
        assert_eq!(
            basename("/foo/bar/baz/asdf/quux.html", Some(".html")),
            "quux"
        );
        assert_eq!(basename("/a/b/", None), "b");
        assert_eq!(basename("/", None), "");
        assert_eq!(basename(".html", Some(".html")), ".html");
    }

    #[test]
    fn extname_matches_node() {
        assert_eq!(extname("index.html"), ".html");
        assert_eq!(extname("index.coffee.md"), ".md");
        assert_eq!(extname("index."), ".");
        assert_eq!(extname("index"), "");
        assert_eq!(extname(".index"), "");
        assert_eq!(extname(".index.md"), ".md");
        assert_eq!(extname(".."), "");
    }

    #[test]
    fn resolve_matches_node() {
        assert_eq!(resolve(&s(&["/foo/bar", "./baz"]), "/w"), "/foo/bar/baz");
        assert_eq!(resolve(&s(&["/foo/bar", "/tmp/file/"]), "/w"), "/tmp/file");
        assert_eq!(
            resolve(
                &s(&["wwwroot", "static_files/png/", "../gif/image.gif"]),
                "/home/myself/node"
            ),
            "/home/myself/node/wwwroot/static_files/gif/image.gif"
        );
        assert_eq!(resolve(&s(&[]), "/home/x"), "/home/x");
    }

    #[test]
    fn relative_matches_node() {
        assert_eq!(
            relative("/data/orandea/test/aaa", "/data/orandea/impl/bbb", "/w"),
            "../../impl/bbb"
        );
        assert_eq!(relative("/a/b", "/a/b/c/d", "/w"), "c/d");
        assert_eq!(relative("/a/b", "/a/b", "/w"), "");
        assert_eq!(relative("/", "/a", "/w"), "a");
    }
}
