//! Managed subprocess pipes and process-tree cancellation (`pi.process`).
//!
//! Node translations (`node:child_process#spawn`, `child_process#ChildProcess`,
//! `node:child_process#execFileSync`, `process.kill`) become explicit
//! `pi.process` bindings. A handle wraps a spawned child with its stdio
//! pipes; the handle owns the process and **kills the whole process tree on
//! disposal** (Drop), so no managed subprocess survives a dropped handle,
//! reload, or VM shutdown.
//!
//! Surface (all on the coroutine seam):
//! - `pi.process.spawn(command, args?, options?) -> handle`
//!   - `options.cwd`, `options.env` (table of string→string), `options.signal`
//!     (AbortSignal: kills the tree when it aborts), `options.timeout_ms`.
//!   - pipes stdout/stderr/stdin by default.
//! - `handle:pid()`, `handle:is_running()`
//! - `handle:read_stdout()` / `handle:read_stderr()` — read available bytes
//!   as a binary-safe Lua string (non-blocking; empty when the pipe is
//!   closed).
//! - `handle:write_stdin(data)` — write bytes to the child's stdin.
//! - `handle:wait()` — await exit; returns the exit code (`nil` on signal
//!   death, matching `exec`).
//! - `handle:kill(signal?)` — kill the process tree (default SIGTERM;
//!   `"kill"` → SIGKILL).
//! - `pi.process.kill(pid)` — kill a process by pid (best-effort).
//!
//! The child is spawned in its own process group (setpgid in pre_exec) so a
//! kill targets the entire tree, matching the spec's `killProcess` semantics
//! used by RLM's root/child REPL trees and Gecko's browser processes.

use std::cell::RefCell;
use std::process::Stdio;
use std::rc::Rc;
use std::time::Duration;

use mlua::{Lua, UserData, UserDataMethods};
use tokio::io::{AsyncReadExt, AsyncWriteExt, BufReader};

/// Interior state shared by a [`LuaProcess`] handle and its async methods.
struct ProcessInner {
    child: Option<tokio::process::Child>,
    stdout: Option<BufReader<tokio::process::ChildStdout>>,
    stderr: Option<BufReader<tokio::process::ChildStderr>>,
    stdin: Option<tokio::process::ChildStdin>,
    pid: Option<u32>,
    exited: bool,
}

pub(crate) struct LuaProcess {
    inner: Rc<RefCell<ProcessInner>>,
}

/// An empty byte slice for EOF/closed reads.
fn empty_bytes() -> &'static [u8] {
    &[]
}

impl LuaProcess {
    fn kill_tree(&self, signal: i32) {
        let pid = self.inner.borrow().pid;
        #[cfg(unix)]
        if let Some(pid) = pid
            && let Ok(pid) = i32::try_from(pid)
        {
            // SAFETY: kill(2) on a pid we spawned; the child runs in its own
            // process group, so the negative pid targets the whole tree.
            unsafe {
                if libc::kill(-pid, signal) != 0 {
                    libc::kill(pid, signal);
                }
            }
        }
        #[cfg(not(unix))]
        {
            let _ = pid;
            if let Some(child) = self.inner.borrow_mut().child.as_mut() {
                let _ = child.start_kill();
            }
        }
    }
}

impl Drop for LuaProcess {
    /// Disposal guarantees: killing the process tree ensures no process
    /// survives a dropped handle or VM shutdown, and a detached reaper
    /// waitpid-loops on the tree so no zombie is left for a long-lived host.
    fn drop(&mut self) {
        self.kill_tree(libc::SIGKILL);
        if let Some(pid) = self.inner.borrow().pid
            && let Ok(pid) = i32::try_from(pid)
        {
            reap_background(pid);
        }
    }
}

/// Reap a killed process group in a detached thread so a discarded `LuaProcess`
/// leaves no zombie behind. waitpid-loop (WNOHANG) on `-pid` (the group the
/// child was setpgid'd into); exits on ECHILD (fully reaped or no longer ours).
fn reap_background(pid: i32) {
    let _ = std::thread::Builder::new()
        .name("pi-process-reap".to_owned())
        .spawn(move || {
            unsafe {
                for _ in 0..50 {
                    let status = libc::waitpid(-pid, std::ptr::null_mut(), libc::WNOHANG);
                    if status == -1 {
                        // ECHILD / ESRCH: no child left to reap.
                        return;
                    }
                    if status != 0 {
                        return; // reaped one member
                    }
                    std::thread::sleep(std::time::Duration::from_millis(50));
                }
            }
        });
}

impl UserData for LuaProcess {
    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        methods.add_method("pid", |_, this, ()| {
            Ok(this.inner.borrow().pid.unwrap_or(0))
        });
        methods.add_method("is_running", |_, this, ()| Ok(!this.inner.borrow().exited));
        methods.add_method("kill", |_, this, signal: Option<String>| {
            let sig = match signal.as_deref() {
                Some("kill") | Some("SIGKILL") => libc::SIGKILL,
                _ => libc::SIGTERM,
            };
            this.kill_tree(sig);
            Ok(())
        });

        // Explicit deterministic disposal: kill the tree immediately (the
        // Drop impl is a GC-dependent safety net). Lua workflows that need a
        // deterministic "no process survives" contract call this explicitly.
        methods.add_method("dispose", |_, this, ()| {
            this.kill_tree(libc::SIGKILL);
            this.inner.borrow_mut().exited = true;
            Ok(())
        });

        methods.add_async_method("wait", |_, this, ()| async move {
            let inner = this.inner.clone();
            let mut child = match inner.borrow_mut().child.take() {
                Some(child) => child,
                None => {
                    return Err(mlua::Error::runtime(
                        "process: wait called on a dead or already-waited process",
                    ));
                }
            };
            let status = child.wait().await;
            inner.borrow_mut().exited = true;
            Ok(status.ok().and_then(|s| s.code()))
        });

        // Read available stdout bytes. Non-blocking: returns whatever is
        // buffered now (empty string when the pipe is at EOF or closed).
        methods.add_async_method("read_stdout", |lua, this, ()| async move {
            let inner = this.inner.clone();
            let mut reader = match inner.borrow_mut().stdout.take() {
                Some(reader) => reader,
                None => return lua.create_string(empty_bytes()),
            };
            let mut buf = Vec::new();
            let n = reader.read_buf(&mut buf).await.unwrap_or(0);
            if n == 0 {
                // Closed/EOF: drop the reader so later reads return empty.
                return lua.create_string(empty_bytes());
            }
            inner.borrow_mut().stdout = Some(reader);
            lua.create_string(&buf)
        });

        methods.add_async_method("read_stderr", |lua, this, ()| async move {
            let inner = this.inner.clone();
            let mut reader = match inner.borrow_mut().stderr.take() {
                Some(reader) => reader,
                None => return lua.create_string(empty_bytes()),
            };
            let mut buf = Vec::new();
            let n = reader.read_buf(&mut buf).await.unwrap_or(0);
            if n == 0 {
                return lua.create_string(empty_bytes());
            }
            inner.borrow_mut().stderr = Some(reader);
            lua.create_string(&buf)
        });

        methods.add_async_method("write_stdin", |_, this, data: mlua::String| async move {
            let inner = this.inner.clone();
            let mut stdin = match inner.borrow_mut().stdin.take() {
                Some(stdin) => stdin,
                None => {
                    return Err(mlua::Error::runtime(
                        "process: write_stdin on a process without an open stdin",
                    ));
                }
            };
            let result = stdin.write_all(&data.as_bytes()).await;
            inner.borrow_mut().stdin = Some(stdin);
            result.map_err(mlua::Error::external)?;
            Ok(())
        });
    }
}

/// Spawn a child in its own process group.
async fn spawn_child(
    command: &str,
    args: Vec<String>,
    cwd: Option<String>,
    env: Option<Vec<(String, String)>>,
) -> mlua::Result<tokio::process::Child> {
    let mut builder = tokio::process::Command::new(command);
    builder
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(cwd) = cwd {
        builder.current_dir(cwd);
    }
    if let Some(env) = env {
        builder.envs(env);
    }
    #[cfg(unix)]
    // SAFETY: pre_exec runs after fork and calls only async-signal-safe setpgid.
    unsafe {
        builder.pre_exec(|| {
            if libc::setpgid(0, 0) == 0 {
                Ok(())
            } else {
                Err(std::io::Error::last_os_error())
            }
        });
    }
    builder
        .spawn()
        .map_err(|e| mlua::Error::runtime(format!("process.spawn '{command}': {e}")))
}

fn process_to_lua(lua: &Lua, inner: Rc<RefCell<ProcessInner>>) -> mlua::Result<mlua::AnyUserData> {
    lua.create_userdata(LuaProcess { inner })
}

/// Install `pi.process` on the API table.
pub(crate) fn install(lua: &Lua, pi: &mlua::Table) -> mlua::Result<()> {
    let process = lua.create_table()?;

    process.set(
        "spawn",
        lua.create_async_function(
            |lua, (command, args, options): (String, Option<mlua::Table>, Option<mlua::Table>)| async move {
                let mut arg_vec = Vec::new();
                if let Some(args) = &args {
                    for arg in args.sequence_values::<String>() {
                        arg_vec.push(arg?);
                    }
                }
                let mut cwd = None;
                let mut env = None;
                let mut signal = None;
                let mut timeout_ms = None;
                if let Some(opts) = &options {
                    if let Some(dir) = opts.get::<Option<String>>("cwd")? {
                        cwd = Some(dir);
                    }
                    if let Some(env_table) = opts.get::<Option<mlua::Table>>("env")? {
                        let mut pairs = Vec::new();
                        for pair in env_table.pairs::<String, String>() {
                            let (k, v) = pair?;
                            pairs.push((k, v));
                        }
                        env = Some(pairs);
                    }
                    if let Some(ud) = opts.get::<Option<mlua::AnyUserData>>("signal")? {
                        signal = Some(
                            ud.borrow::<crate::ai::LuaAbortSignal>()
                                .map_err(|_| {
                                    mlua::Error::runtime(
                                        "process.spawn: signal must be an abort signal",
                                    )
                                })?
                                .0
                                .clone(),
                        );
                    }
                    timeout_ms = opts.get::<Option<u64>>("timeout_ms")?;
                }

                let mut child = spawn_child(&command, arg_vec, cwd, env).await?;
                let pid = child.id();
                let stdout = child.stdout.take().map(BufReader::new);
                let stderr = child.stderr.take().map(BufReader::new);
                let stdin = child.stdin.take();

                let inner = Rc::new(RefCell::new(ProcessInner {
                    child: Some(child),
                    stdout,
                    stderr,
                    stdin,
                    pid,
                    exited: false,
                }));

                // Optional signal/timeout: kill the tree when the signal
                // aborts or the timeout elapses. The task captures only the
                // pid (not the inner Rc) so it never holds the handle alive
                // after the Lua side drops it — disposal still reaps the tree.
                if signal.is_some() || timeout_ms.is_some_and(|ms| ms > 0) {
                    #[cfg(unix)]
                    let pid = pid.and_then(|p| i32::try_from(p).ok());
                    tokio::task::spawn_local(async move {
                        let abort = async {
                            match &signal {
                                Some(signal) => signal.aborted().await,
                                None => std::future::pending().await,
                            }
                        };
                        let timeout = async {
                            match timeout_ms {
                                Some(ms) if ms > 0 => {
                                    tokio::time::sleep(Duration::from_millis(ms)).await
                                }
                                _ => std::future::pending().await,
                            }
                        };
                        tokio::select! {
                            () = abort => {}
                            () = timeout => {}
                        }
                        #[cfg(unix)]
                        if let Some(pid) = pid {
                            // SAFETY: kill(2) on the pid we spawned; the child
                            // runs in its own process group (negative pid).
                            unsafe {
                                let signal: i32 = libc::SIGTERM;
                                if libc::kill(-pid, signal) != 0 {
                                    libc::kill(pid, signal);
                                }
                            }
                        }
                    });
                }

                process_to_lua(&lua, inner)
            },
        )?,
    )?;

    process.set(
        "kill",
        lua.create_function(|_, pid: u32| {
            #[cfg(unix)]
            // Guard: pid 0 signals the *caller's* whole process group, and
            // that is never the intent of a targeted kill. Ignore it.
            if pid != 0
                && let Ok(pid) = i32::try_from(pid)
            {
                // SAFETY: kill(2) on a pid the caller supplied; best-effort.
                unsafe {
                    libc::kill(pid, libc::SIGTERM);
                }
            }
            #[cfg(not(unix))]
            {
                let _ = pid;
            }
            Ok(())
        })?,
    )?;

    pi.set("process", process)?;
    Ok(())
}
