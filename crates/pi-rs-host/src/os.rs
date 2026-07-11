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

use std::time::UNIX_EPOCH;

fn io_err(op: &str, path: &str, e: &std::io::Error) -> mlua::Error {
    mlua::Error::runtime(format!("{op} '{path}': {e}"))
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

fn install_fs(lua: &mlua::Lua, pi: &mlua::Table) -> mlua::Result<()> {
    let fs = lua.create_table()?;

    fs.set(
        "read_file",
        lua.create_async_function(|_, path: String| async move {
            tokio::fs::read_to_string(&path)
                .await
                .map_err(|e| io_err("read_file", &path, &e))
        })?,
    )?;

    fs.set(
        "write_file",
        lua.create_async_function(|_, (path, contents): (String, mlua::String)| async move {
            tokio::fs::write(&path, contents.as_bytes())
                .await
                .map_err(|e| io_err("write_file", &path, &e))
        })?,
    )?;

    fs.set(
        "append_file",
        lua.create_function(|_, (path, contents): (String, mlua::String)| {
            use std::io::Write;
            let mut file = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&path)
                .map_err(|e| io_err("append_file", &path, &e))?;
            file.write_all(&contents.as_bytes())
                .map_err(|e| io_err("append_file", &path, &e))
        })?,
    )?;

    // Secure persisted scratch file for streaming tools. These two bindings
    // are synchronous because `append_file` is called from pi.exec's onData
    // callback, where yielding across the callback's C boundary is invalid.
    // Node `os.tmpdir()` — the paste-image policy joins temp paths in Lua.
    fs.set(
        "tmpdir",
        lua.create_function(|_, ()| Ok(std::env::temp_dir().to_string_lossy().into_owned()))?,
    )?;

    fs.set(
        "create_temp_file",
        lua.create_function(|_, (prefix, contents): (String, mlua::String)| {
            use std::io::Write;
            let safe_prefix: String = prefix
                .chars()
                .map(|c| {
                    if c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.') {
                        c
                    } else {
                        '-'
                    }
                })
                .collect();
            let mut file = tempfile::Builder::new()
                .prefix(&safe_prefix)
                .suffix(".log")
                .tempfile()
                .map_err(|e| io_err("create_temp_file", &safe_prefix, &e))?;
            file.write_all(&contents.as_bytes())
                .map_err(|e| io_err("create_temp_file", &safe_prefix, &e))?;
            let (_file, path) = file
                .keep()
                .map_err(|e| io_err("create_temp_file", &safe_prefix, &e.error))?;
            Ok(path.to_string_lossy().into_owned())
        })?,
    )?;

    fs.set(
        "read_bytes",
        lua.create_async_function(|lua, path: String| async move {
            let bytes = tokio::fs::read(&path)
                .await
                .map_err(|e| io_err("read_bytes", &path, &e))?;
            lua.create_string(&bytes)
        })?,
    )?;

    fs.set(
        "exists",
        lua.create_async_function(|_, path: String| async move {
            Ok(tokio::fs::try_exists(&path).await.unwrap_or(false))
        })?,
    )?;

    fs.set(
        "read_dir",
        lua.create_async_function(|lua, path: String| async move {
            let mut rd = tokio::fs::read_dir(&path)
                .await
                .map_err(|e| io_err("read_dir", &path, &e))?;
            let names = lua.create_table()?;
            while let Some(entry) = rd
                .next_entry()
                .await
                .map_err(|e| io_err("read_dir", &path, &e))?
            {
                names.push(entry.file_name().to_string_lossy().into_owned())?;
            }
            Ok(names)
        })?,
    )?;

    fs.set(
        "stat",
        lua.create_async_function(|lua, path: String| async move {
            let md = tokio::fs::metadata(&path)
                .await
                .map_err(|e| io_err("stat", &path, &e))?;
            let stat = lua.create_table()?;
            stat.set(
                "type",
                if md.is_dir() {
                    "dir"
                } else if md.is_file() {
                    "file"
                } else {
                    "other"
                },
            )?;
            stat.set("size", md.len())?;
            let modified_ms = md
                .modified()
                .ok()
                .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                .map(|d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
                .unwrap_or(0);
            stat.set("modified_ms", modified_ms)?;
            Ok(stat)
        })?,
    )?;

    fs.set(
        "mkdir",
        lua.create_async_function(|_, path: String| async move {
            // Node's `mkdirSync(p, { recursive: true })` — the form the
            // spec's examples use.
            tokio::fs::create_dir_all(&path)
                .await
                .map_err(|e| io_err("mkdir", &path, &e))
        })?,
    )?;

    fs.set(
        "realpath",
        lua.create_async_function(|_, path: String| async move {
            let real = tokio::fs::canonicalize(&path)
                .await
                .map_err(|e| io_err("realpath", &path, &e))?;
            Ok(real.to_string_lossy().into_owned())
        })?,
    )?;

    fs.set(
        "remove_file",
        lua.create_async_function(|_, path: String| async move {
            tokio::fs::remove_file(&path)
                .await
                .map_err(|e| io_err("remove_file", &path, &e))
        })?,
    )?;

    pi.set("fs", fs)?;
    Ok(())
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
pub(crate) fn install(lua: &mlua::Lua, pi: &mlua::Table, cwd: &str) -> mlua::Result<()> {
    install_fs(lua, pi)?;
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
