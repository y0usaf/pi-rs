//! pi.process — managed subprocess pipes and process-tree cancellation
//! (PLAN 9.9 process.spawn / spawnSync / execFileSync / kill / stdio
//! pipes, resource.child_process lifetime).
//!
//! spawn starts the child in its own process group (setpgid) so kill()
//! targets the whole tree: signal goes to -pid, so a shell and its
//! descendants die together — the process-tree cancellation contract.
//! Every child is a tracked resource; dispose() or VM shutdown kills the
//! tree, closes the pipes, and reaps the child.
//!
//! Lua surface (pi.process):
//! - spawn(command, args?, options?) -> child (async start)
//!   options: cwd, env (table), signal (abort userdata)
//!   child methods: pid(), write_stdin(data) async, close_stdin(),
//!   read_stdout(max?, timeout_ms?) async, read_stderr(max?, timeout_ms?)
//!   async, kill(signal_name?) async-tree, kill_tree(),
//!   wait(timeout_ms?) async -> { code, killed, signal }, is_running(),
//!   dispose() async
//! - spawn_sync(command, args?, options?) async -> { stdout, stderr, code,
//!   killed } (captures to completion; options.timeout/signal honored)
//! - exec_file_sync(command, args?, options?) — same capture contract
//! - kill(pid, signal_name?) -> bool (negative pid = process group)
//! - platform() -> "linux" | "darwin" | "win32"
//! - pid() -> host pid
//! - env — read-only view of the process environment (same as pi.env)

use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use mlua::{Lua, Table, UserData, UserDataMethods};
use pi_rs_ai::transport::AbortSignal;
use tokio::io::AsyncWriteExt;

#[cfg(unix)]
fn send_signal(pid: Option<u32>, signal: i32) {
    if let Some(pid) = pid
        && let Ok(pid) = i32::try_from(pid)
    {
        // SAFETY: kill(2) on a pid we spawned; no pointers, no aliasing.
        unsafe {
            // Negative pid targets the process group (the whole tree).
            if libc::kill(-pid, signal) != 0 {
                libc::kill(pid, signal);
            }
        }
    }
}

#[cfg(not(unix))]
fn send_signal(pid: Option<u32>, _signal: i32) {
    let _ = pid;
}

fn signal_number(name: Option<String>) -> mlua::Result<i32> {
    #[cfg(unix)]
    let map = |name: &str| match name {
        "SIGTERM" | "terminate" => Ok(libc::SIGTERM),
        "SIGKILL" | "kill" => Ok(libc::SIGKILL),
        "SIGINT" | "interrupt" => Ok(libc::SIGINT),
        "SIGHUP" | "hangup" => Ok(libc::SIGHUP),
        other => Err(mlua::Error::runtime(format!("kill: unknown signal {other}"))),
    };
    #[cfg(not(unix))]
    let map = |name: &str| match name {
        "SIGTERM" | "terminate" => Ok(15),
        "SIGKILL" | "kill" => Ok(9),
        "SIGINT" | "interrupt" => Ok(2),
        other => Err(mlua::Error::runtime(format!("kill: unknown signal {other}"))),
    };
    match name {
        Some(name) => map(&name),
        None => Ok(15), // SIGTERM default, Node child.kill()
    }
}

struct ExitInfo {
    code: Option<i32>,
    killed: bool,
}

struct ChildState {
    child: Option<tokio::process::Child>,
    stdin: Option<tokio::process::ChildStdin>,
    stdout: Option<Box<dyn tokio::io::AsyncRead + Unpin + Send>>,
    stderr: Option<Box<dyn tokio::io::AsyncRead + Unpin + Send>>,
    pid: Option<u32>,
    signal: Option<AbortSignal>,
    exit: Option<ExitInfo>,
}

struct LuaChild {
    state: Arc<Mutex<ChildState>>,
    label: String,
}

impl UserData for LuaChild {
    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        methods.add_method("pid", |_, this, ()| {
            let state = this.state.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            Ok(state.pid.unwrap_or(0))
        });
        methods.add_async_method_mut("write_stdin", |_, this, data: mlua::String| async move {
            let stdin = {
                let mut state = this.state.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
                state.stdin.take()
            };
            let Some(mut stdin) = stdin else {
                return Err(mlua::Error::runtime("stdin closed"));
            };
            stdin
                .write_all(data.as_bytes().as_ref())
                .await
                .map_err(mlua::Error::external)?;
            let mut state = this.state.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            state.stdin = Some(stdin);
            Ok(())
        });
        methods.add_method_mut("close_stdin", |_, this, ()| {
            let mut state = this.state.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            state.stdin.take();
            Ok(())
        });
        methods.add_async_method_mut(
            "read_stdout",
            |lua, this, (max_bytes, timeout_ms): (Option<usize>, Option<u64>)| async move {
                read_pipe(&lua, &this.state, Pipe::Stdout, max_bytes, timeout_ms).await
            },
        );
        methods.add_async_method_mut(
            "read_stderr",
            |lua, this, (max_bytes, timeout_ms): (Option<usize>, Option<u64>)| async move {
                read_pipe(&lua, &this.state, Pipe::Stderr, max_bytes, timeout_ms).await
            },
        );
        methods.add_async_method_mut(
            "kill",
            |_, this, signal: Option<String>| async move {
                let number = signal_number(signal)?;
                let pid = {
                    let state = this.state.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
                    state.pid
                };
                send_signal(pid, number);
                Ok(())
            },
        );
        methods.add_method_mut("kill_tree", |_, this, ()| {
            let pid = {
                let state = this.state.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
                state.pid
            };
            send_signal(pid, libc::SIGKILL);
            Ok(())
        });
        methods.add_method("is_running", |_, this, ()| {
            let state = this.state.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            Ok(state.exit.is_none())
        });
        methods.add_async_method_mut(
            "wait",
            |lua, this, timeout_ms: Option<u64>| async move {
                let exit = wait_child(&this.state, timeout_ms).await?;
                let out = lua.create_table()?;
                out.set("code", exit.code.unwrap_or(0))?;
                out.set("killed", exit.killed)?;
                Ok(out)
            },
        );
        methods.add_async_method_mut("dispose", |_, this, ()| async move {
            dispose_child(&this.state).await;
            crate::resources::unregister("resource.child_process", &this.label);
            Ok(())
        });
    }
}

enum Pipe {
    Stdout,
    Stderr,
}

async fn read_pipe(
    lua: &Lua,
    state: &Arc<Mutex<ChildState>>,
    pipe: Pipe,
    max_bytes: Option<usize>,
    timeout_ms: Option<u64>,
) -> mlua::Result<mlua::Value> {
    use tokio::io::AsyncReadExt as _;
    let (mut stdout, mut stderr) = {
        let mut state = state.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        (state.stdout.take(), state.stderr.take())
    };
    let stream = match pipe {
        Pipe::Stdout => stdout.as_mut(),
        Pipe::Stderr => stderr.as_mut(),
    };
    let Some(stream) = stream else {
        return Ok(mlua::Value::Nil);
    };
    let max = max_bytes.unwrap_or(4096).clamp(1, 1024 * 1024);
    let mut buf = vec![0u8; max];
    let read = match timeout_ms {
        Some(ms) => {
            match tokio::time::timeout(Duration::from_millis(ms), stream.read(&mut buf)).await {
                Ok(read) => read.map_err(mlua::Error::external)?,
                Err(_) => return Err(mlua::Error::runtime("child pipe read timeout")),
            }
        }
        None => stream.read(&mut buf).await.map_err(mlua::Error::external)?,
    };
    {
        let mut state = state.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        state.stdout = stdout;
        state.stderr = stderr;
    }
    if read == 0 {
        Ok(mlua::Value::Nil)
    } else {
        Ok(mlua::Value::String(lua.create_string(&buf[..read])?))
    }
}

async fn wait_child(
    state: &Arc<Mutex<ChildState>>,
    timeout_ms: Option<u64>,
) -> mlua::Result<ExitInfo> {
    // Fast path: already exited.
    {
        let state = state.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(exit) = &state.exit {
            return Ok(ExitInfo { code: exit.code, killed: exit.killed });
        }
    }
    let child = {
        let mut state = state.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        state.child.take()
    };
    let Some(mut child) = child else {
        let state = state.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        return Ok(state
            .exit
            .as_ref()
            .map(|e| ExitInfo { code: e.code, killed: e.killed })
            .unwrap_or(ExitInfo { code: Some(1), killed: false }));
    };
    let pid = child.id();
    let signal = {
        let state = state.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        state.signal.clone()
    };
    let timeout_wait = async {
        match timeout_ms {
            Some(ms) if ms > 0 => tokio::time::sleep(Duration::from_millis(ms)).await,
            _ => std::future::pending().await,
        }
    };
    let abort_wait = async {
        match &signal {
            Some(signal) => signal.aborted().await,
            None => std::future::pending().await,
        }
    };
    tokio::pin!(timeout_wait);
    tokio::pin!(abort_wait);
    let mut killed = false;
    tokio::select! {
        status = child.wait() => {
            let status = status.map_err(mlua::Error::external)?;
            finish_wait(state, &mut child, status.code(), killed)
        }
        () = &mut timeout_wait => {
            send_signal(pid, libc::SIGTERM);
            let status = tokio::select! {
                status = child.wait() => status,
                () = tokio::time::sleep(Duration::from_millis(5000)) => {
                    send_signal(pid, libc::SIGKILL);
                    child.wait().await
                }
            };
            killed = true;
            let status = status.map_err(mlua::Error::external)?;
            finish_wait(state, &mut child, status.code(), killed)
        }
        () = &mut abort_wait => {
            send_signal(pid, libc::SIGKILL);
            let status = child.wait().await.map_err(mlua::Error::external)?;
            killed = true;
            finish_wait(state, &mut child, status.code(), killed)
        }
    }
}

fn finish_wait(
    state: &Arc<Mutex<ChildState>>,
    _child: &mut tokio::process::Child,
    code: Option<i32>,
    killed: bool,
) -> mlua::Result<ExitInfo> {
    let exit = ExitInfo { code, killed };
    let mut state = state.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    state.exit = Some(ExitInfo { code, killed });
    Ok(exit)
}

async fn dispose_child(state: &Arc<Mutex<ChildState>>) {
    let pid = {
        let state = state.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        state.pid
    };
    send_signal(pid, libc::SIGKILL);
    let child = {
        let mut state = state.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        state.stdin.take();
        state.stdout.take();
        state.stderr.take();
        let child = state.child.take();
        state.exit = Some(ExitInfo { code: None, killed: true });
        child
    }; // guard dropped here, before the await
    if let Some(mut child) = child {
        let _ = tokio::time::timeout(Duration::from_secs(1), child.wait()).await;
    }
}

fn install_spawn(lua: &Lua, process: &Table, default_cwd: &str) -> mlua::Result<()> {
    let default_cwd = default_cwd.to_owned();
    let spawn = lua.create_async_function(
        move |lua, (command, args, options): (String, Option<Table>, Option<Table>)| {
            let default_cwd = default_cwd.clone();
            async move {
                let mut arg_vec = Vec::new();
                if let Some(args) = args {
                    for arg in args.sequence_values::<String>() {
                        arg_vec.push(arg?);
                    }
                }
                let mut cwd = default_cwd;
                let mut env: Option<Vec<(String, String)>> = None;
                let mut signal = None;
                if let Some(opts) = options {
                    if let Some(dir) = opts.get::<Option<String>>("cwd")? {
                        cwd = dir;
                    }
                    if let Some(env_table) = opts.get::<Option<Table>>("env")? {
                        let mut entries = Vec::new();
                        for pair in env_table.pairs::<String, String>() {
                            let (key, value) = pair?;
                            entries.push((key, value));
                        }
                        env = Some(entries);
                    }
                    if let Some(userdata) = opts.get::<Option<mlua::AnyUserData>>("signal")? {
                        signal = Some(
                            userdata
                                .borrow::<crate::ai::LuaAbortSignal>()
                                .map_err(|_| {
                                    mlua::Error::runtime(
                                        "spawn: signal must be an abort signal",
                                    )
                                })?
                                .0
                                .clone(),
                        );
                    }
                }
                let mut builder = tokio::process::Command::new(&command);
                builder
                    .args(&arg_vec)
                    .current_dir(&cwd)
                    .stdin(Stdio::piped())
                    .stdout(Stdio::piped())
                    .stderr(Stdio::piped());
                if let Some(entries) = &env {
                    builder.env_clear();
                    for (key, value) in entries {
                        builder.env(key, value);
                    }
                }
                #[cfg(unix)]
                unsafe {
                    // SAFETY: pre_exec runs after fork and calls only async-signal-safe setpgid.
                    builder.pre_exec(|| {
                        if libc::setpgid(0, 0) == 0 {
                            Ok(())
                        } else {
                            Err(std::io::Error::last_os_error())
                        }
                    });
                }
                let mut child = match builder.spawn() {
                    Ok(child) => child,
                    Err(error) => {
                        return Err(mlua::Error::runtime(format!(
                            "spawn {command}: {error}"
                        )));
                    }
                };
                let pid = child.id();
                let stdin = child.stdin.take();
                let stdout = child
                    .stdout
                    .take()
                    .map(|s| Box::new(s) as Box<dyn tokio::io::AsyncRead + Unpin + Send>);
                let stderr = child
                    .stderr
                    .take()
                    .map(|s| Box::new(s) as Box<dyn tokio::io::AsyncRead + Unpin + Send>);
                let label = format!("child:{command}");
                let state = Arc::new(Mutex::new(ChildState {
                    child: Some(child),
                    stdin,
                    stdout,
                    stderr,
                    pid,
                    signal,
                    exit: None,
                }));
                let resource_state = Arc::clone(&state);
                crate::resources::register("resource.child_process", label.clone(), move || {
                    let pid = {
                        let state = resource_state.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
                        state.pid
                    };
                    send_signal(pid, libc::SIGKILL);
                    // Reap so the child is not a zombie after the kill.
                    #[cfg(unix)]
                    if let Some(pid) = pid
                        && let Ok(pid) = i32::try_from(pid)
                    {
                        // SAFETY: waitpid on a process this host spawned; the
                        // child is dead (or already reaped -> ECHILD, ignored).
                        unsafe {
                            let mut status = 0;
                            let _ = libc::waitpid(pid, &mut status, 0);
                        }
                    }
                });
                Ok(mlua::Value::UserData(
                    lua.create_userdata(LuaChild { state, label })?,
                ))
            }
        },
    )?;
    process.set("spawn", spawn)?;
    Ok(())
}


pub(crate) fn install(lua: &Lua, pi: &Table, default_cwd: &str) -> mlua::Result<()> {
    let process = lua.create_table()?;
    install_spawn(lua, &process, default_cwd)?;
    let default_cwd = default_cwd.to_owned();
    let capture = lua.create_async_function(
        move |lua, (command, args, options): (String, Option<Table>, Option<Table>)| {
            let default_cwd = default_cwd.clone();
            async move {
                let mut arg_vec = Vec::new();
                if let Some(args) = args {
                    for arg in args.sequence_values::<String>() {
                        arg_vec.push(arg?);
                    }
                }
                let mut timeout_ms = None;
                let mut signal = None;
                if let Some(opts) = options {
                    if let Some(ms) = opts.get::<Option<u64>>("timeout")? {
                        timeout_ms = Some(ms);
                    }
                    if let Some(userdata) = opts.get::<Option<mlua::AnyUserData>>("signal")? {
                        signal = Some(
                            userdata
                                .borrow::<crate::ai::LuaAbortSignal>()
                                .map_err(|_| {
                                    mlua::Error::runtime(
                                        "capture: signal must be an abort signal",
                                    )
                                })?
                                .0
                                .clone(),
                        );
                    }
                }
                let result = crate::exec::exec_command(
                    &command, &arg_vec, &default_cwd, timeout_ms, signal, None,
                )
                .await?;
                let reply = lua.create_table()?;
                reply.set("stdout", result.stdout)?;
                reply.set("stderr", result.stderr)?;
                reply.set("code", result.code)?;
                reply.set("killed", result.killed)?;
                Ok(reply)
            }
        },
    )?;
    process.set("spawn_sync", capture.clone())?;
    process.set("exec_file_sync", capture)?;
    process.set(
        "kill",
        lua.create_function(|_, (pid, signal): (i64, Option<String>)| {
            let number = signal_number(signal)?;
            let target = i32::try_from(pid)
                .map_err(|_| mlua::Error::runtime("kill: pid out of range"))?;
            #[cfg(unix)]
            unsafe {
                let result = libc::kill(target, number);
                Ok(result == 0)
            }
            #[cfg(not(unix))]
            {
                let _ = (target, number);
                Ok(false)
            }
        })?,
    )?;
    process.set(
        "platform",
        lua.create_function(|_, ()| {
            let platform = match std::env::consts::OS {
                "macos" => "darwin",
                "windows" => "win32",
                other => other,
            };
            Ok(platform.to_owned())
        })?,
    )?;
    process.set(
        "pid",
        lua.create_function(|_, ()| Ok(std::process::id()))?,
    )?;
    let env = crate::os::env_table(lua)?;
    process.set("env", env)?;
    pi.set("process", process)?;
    Ok(())
}
