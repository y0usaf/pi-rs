//! Port of the spec's `core/exec.ts` — shared command execution for
//! extensions and custom tools, surfaced as `pi.exec(command, args, options?)`
//! (spec: `ExtensionAPI.exec`, wired in `loader.ts`).
//!
//! Spec semantics, matched 1:1:
//! - `shell: false`; stdin ignored; stdout/stderr captured as strings.
//! - Never throws: spawn failure resolves `{ code = 1 }`; death by signal
//!   resolves `code ?? 0` (the `killed` flag is the discriminator).
//! - `options.timeout` (ms, applied when > 0): on expiry the child gets
//!   SIGTERM, then SIGKILL 5 seconds later if it hasn't exited;
//!   `killed = true`.
//! - `options.cwd` defaults to the host cwd (spec: the loader's injected
//!   `cwd`).
//! - After exit, stdio gets a short grace window instead of an unbounded
//!   wait, so pipes inherited by detached descendants can't hang the call
//!   (spec: `waitForChildProcess` + `EXIT_STDIO_GRACE_MS`).
//! - `options.signal` (an abort-signal userdata, e.g. a tool `signal` or
//!   `pi.abort_signal()`): on abort the child gets the same SIGTERM →
//!   SIGKILL treatment as a timeout and `killed = true` (spec: the shared
//!   `killProcess` handler for both triggers).

use std::cell::RefCell;
use std::process::Stdio;
use std::rc::Rc;
use std::time::Duration;

use pi_rs_ai::transport::AbortSignal;
use tokio::io::AsyncReadExt;

/// Spec `EXIT_STDIO_GRACE_MS`.
const EXIT_STDIO_GRACE_MS: u64 = 100;

/// Spec: force kill 5 seconds after SIGTERM if the child hasn't exited.
const SIGKILL_DELAY_MS: u64 = 5000;

/// The spec's `ExecResult`.
pub(crate) struct ExecResult {
    pub(crate) stdout: String,
    pub(crate) stderr: String,
    pub(crate) code: i64,
    pub(crate) killed: bool,
}

fn spawn_failure() -> ExecResult {
    ExecResult {
        stdout: String::new(),
        stderr: String::new(),
        code: 1,
        killed: false,
    }
}

#[cfg(unix)]
fn send_signal(pid: Option<u32>, signal: i32) {
    if let Some(pid) = pid
        && let Ok(pid) = i32::try_from(pid)
    {
        // SAFETY: kill(2) on a pid we spawned; no pointers, no aliasing —
        // the only effect is signal delivery.
        unsafe {
            // The child starts a new process group; negative pid targets the tree.
            if libc::kill(-pid, signal) != 0 {
                libc::kill(pid, signal);
            }
        }
    }
}

/// Drain a pipe into a shared buffer in chunks. The buffer borrow is
/// never held across an await, so partial output stays readable even if
/// this future is abandoned mid-stream (the stdio grace path).
async fn drain<R: tokio::io::AsyncRead + Unpin>(
    pipe: Option<R>,
    buf: Rc<RefCell<Vec<u8>>>,
    on_data: Option<mlua::Function>,
) -> mlua::Result<()> {
    let Some(mut pipe) = pipe else {
        return Ok(());
    };
    let mut chunk = [0u8; 8192];
    loop {
        match pipe.read(&mut chunk).await {
            Ok(0) | Err(_) => break,
            Ok(n) => {
                buf.borrow_mut().extend_from_slice(&chunk[..n]);
                if let Some(callback) = &on_data {
                    callback.call::<()>(mlua::String::wrap(&chunk[..n]))?;
                }
            }
        }
    }
    Ok(())
}

/// The spec's `execCommand`.
pub(crate) async fn exec_command(
    command: &str,
    args: &[String],
    cwd: &str,
    timeout_ms: Option<u64>,
    signal: Option<AbortSignal>,
    on_data: Option<mlua::Function>,
) -> mlua::Result<ExecResult> {
    let mut command_builder = tokio::process::Command::new(command);
    command_builder
        .args(args)
        .current_dir(cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(unix)]
    // SAFETY: pre_exec runs after fork and calls only async-signal-safe setpgid.
    unsafe {
        command_builder.pre_exec(|| {
            if libc::setpgid(0, 0) == 0 {
                Ok(())
            } else {
                Err(std::io::Error::last_os_error())
            }
        });
    }
    let spawned = command_builder.spawn();
    let mut child = match spawned {
        Ok(child) => child,
        Err(_) => return Ok(spawn_failure()),
    };
    let pid = child.id();

    let out = Rc::new(RefCell::new(Vec::new()));
    let err = Rc::new(RefCell::new(Vec::new()));
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let read = async {
        let (outcome, errcome) = tokio::join!(
            drain(stdout, Rc::clone(&out), on_data.clone()),
            drain(stderr, Rc::clone(&err), on_data)
        );
        outcome.and(errcome)
    };
    tokio::pin!(read);

    // Spec `waitForChildProcess`: resolve when the child has exited and
    // stdio has ended — or EXIT_STDIO_GRACE_MS after exit, so pipes
    // inherited by detached descendants can't hang the call.
    let collect_fut = async {
        tokio::select! {
            status = child.wait() => {
                if let Ok(read_result) = tokio::time::timeout(
                    Duration::from_millis(EXIT_STDIO_GRACE_MS),
                    &mut read,
                )
                .await { read_result?; }
                Ok::<_, mlua::Error>(status.ok())
            }
            read_result = &mut read => {
                read_result?;
                Ok(child.wait().await.ok())
            },
        }
    };
    tokio::pin!(collect_fut);

    // Spec: timeout and abort share one kill path (`killProcess`); either
    // trigger sets `killed = true`.
    let kill_trigger = async {
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
        tokio::select! {
            () = timeout_wait => (),
            () = abort_wait => (),
        }
    };
    tokio::pin!(kill_trigger);

    let (status, killed) = tokio::select! {
        status = &mut collect_fut => (status?, false),
        () = &mut kill_trigger => {
            #[cfg(unix)]
            {
                send_signal(pid, libc::SIGTERM);
                let status = tokio::select! {
                    status = &mut collect_fut => {
                        // The leader may exit on TERM while a descendant ignores it.
                        // Kill the still-owned process group before returning.
                        send_signal(pid, libc::SIGKILL);
                        status?
                    },
                    () = tokio::time::sleep(Duration::from_millis(SIGKILL_DELAY_MS)) => {
                        send_signal(pid, libc::SIGKILL);
                        (&mut collect_fut).await?
                    }
                };
                (status, true)
            }
            #[cfg(not(unix))]
            {
                let _ = child.start_kill();
                ((&mut collect_fut).await?, true)
            }
        }
    };

    let stdout_bytes = std::mem::take(&mut *out.borrow_mut());
    let stderr_bytes = std::mem::take(&mut *err.borrow_mut());
    Ok(ExecResult {
        stdout: String::from_utf8_lossy(&stdout_bytes).into_owned(),
        stderr: String::from_utf8_lossy(&stderr_bytes).into_owned(),
        // Spec: `code ?? 0` (signal death → null → 0); wait failure → 1.
        code: status.map_or(1, |s| s.code().map_or(0, i64::from)),
        killed,
    })
}

/// Install `pi.exec(command, args?, options?)` on the API table.
pub(crate) fn install(lua: &mlua::Lua, pi: &mlua::Table, default_cwd: &str) -> mlua::Result<()> {
    let default_cwd = default_cwd.to_owned();
    let exec = lua.create_async_function(
        move |lua, (command, args, options): (String, Option<mlua::Table>, Option<mlua::Table>)| {
            let default_cwd = default_cwd.clone();
            async move {
                let mut arg_vec = Vec::new();
                if let Some(args) = args {
                    for arg in args.sequence_values::<String>() {
                        arg_vec.push(arg?);
                    }
                }
                let mut cwd = default_cwd;
                let mut timeout_ms = None;
                let mut signal = None;
                let mut on_data = None;
                if let Some(opts) = options {
                    if let Some(dir) = opts.get::<Option<String>>("cwd")? {
                        cwd = dir;
                    }
                    if let Some(ms) = opts.get::<Option<f64>>("timeout")?
                        && ms.is_finite()
                        && ms > 0.0
                    {
                        timeout_ms = Some(ms.min(u64::MAX as f64) as u64);
                    }
                    if let Some(userdata) = opts.get::<Option<mlua::AnyUserData>>("signal")? {
                        signal = Some(
                            userdata
                                .borrow::<crate::ai::LuaAbortSignal>()
                                .map_err(|_| {
                                    mlua::Error::runtime("exec: signal must be an abort signal")
                                })?
                                .0
                                .clone(),
                        );
                    }
                    on_data = opts.get::<Option<mlua::Function>>("onData")?;
                }
                let result =
                    exec_command(&command, &arg_vec, &cwd, timeout_ms, signal, on_data).await?;
                let reply = lua.create_table()?;
                reply.set("stdout", result.stdout)?;
                reply.set("stderr", result.stderr)?;
                reply.set("code", result.code)?;
                reply.set("killed", result.killed)?;
                Ok(reply)
            }
        },
    )?;
    pi.set("exec", exec)?;
    Ok(())
}
