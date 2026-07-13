//! Public `pi.exec` adapter over the scope-owned process effect.

use std::time::Duration;

use pi_rs_ai::transport::AbortSignal;

use crate::effects::{
    EffectError, EffectOptions, EffectRequest, EffectResult, EffectTimeout, ProcessEvent,
    ProcessOutputKind, ProcessRequest,
};

pub(crate) struct ExecResult {
    pub(crate) stdout: String,
    pub(crate) stderr: String,
    pub(crate) code: i64,
    pub(crate) killed: bool,
}

struct ExecRequest {
    command: String,
    args: Vec<String>,
    cwd: String,
    timeout_ms: Option<u64>,
    signal: Option<AbortSignal>,
    on_data: Option<mlua::Function>,
    max_output_bytes: usize,
}

async fn exec_command(
    hub: &crate::effects::EffectHub,
    scope: crate::kernel::ScopeId,
    request: ExecRequest,
) -> mlua::Result<ExecResult> {
    let ExecRequest {
        command,
        args,
        cwd,
        timeout_ms,
        signal,
        on_data,
        max_output_bytes,
    } = request;
    let cancellation = crate::effects::cancellation();
    let request = ProcessRequest {
        program: command,
        args,
        cwd: Some(cwd),
        stdin: None,
        options: EffectOptions {
            timeout: timeout_ms.map_or(EffectTimeout::Disabled, |milliseconds| {
                EffectTimeout::After(Duration::from_millis(milliseconds))
            }),
            stream_capacity: crate::effects::DEFAULT_STREAM_CAPACITY,
            max_output_bytes,
        },
    };
    let result = hub
        .request(scope, EffectRequest::Process(request), cancellation.clone())
        .await;
    let mut stream = match result {
        Ok(EffectResult::Process(stream)) => stream,
        Ok(_) => {
            return Err(mlua::Error::runtime(
                "process effect returned the wrong result",
            ));
        }
        Err(EffectError::Cancelled) if signal.is_some() => {
            return Ok(ExecResult {
                stdout: String::new(),
                stderr: String::new(),
                code: 0,
                killed: true,
            });
        }
        Err(error) => return Err(crate::effects::lua_error(error)),
    };
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    loop {
        let event = if let Some(signal) = &signal {
            tokio::select! {
                event = stream.next() => event,
                () = signal.aborted() => {
                    cancellation.cancel();
                    stream.next().await
                }
            }
        } else {
            stream.next().await
        };
        match event {
            Some(ProcessEvent::Output(kind, bytes)) => {
                match kind {
                    ProcessOutputKind::Stdout => stdout.extend(&bytes),
                    ProcessOutputKind::Stderr => stderr.extend(&bytes),
                }
                if let Some(callback) = &on_data {
                    callback
                        .call_async::<()>(mlua::String::wrap(&bytes))
                        .await?;
                }
            }
            Some(ProcessEvent::Exit(output)) => {
                return Ok(ExecResult {
                    stdout: String::from_utf8_lossy(&stdout).into_owned(),
                    stderr: String::from_utf8_lossy(&stderr).into_owned(),
                    code: output.code,
                    killed: output.killed,
                });
            }
            Some(ProcessEvent::Error(EffectError::Timeout | EffectError::Cancelled)) => {
                return Ok(ExecResult {
                    stdout: String::from_utf8_lossy(&stdout).into_owned(),
                    stderr: String::from_utf8_lossy(&stderr).into_owned(),
                    code: 0,
                    killed: true,
                });
            }
            Some(ProcessEvent::Error(error)) => return Err(crate::effects::lua_error(error)),
            None => {
                return Err(mlua::Error::runtime(
                    "process effect ended without a result",
                ));
            }
        }
    }
}

pub(crate) fn install(
    lua: &mlua::Lua,
    pi: &mlua::Table,
    default_cwd: &str,
    hub: crate::effects::EffectHub,
) -> mlua::Result<()> {
    let default_cwd = default_cwd.to_owned();
    let exec = lua.create_async_function(
        move |lua, (command, args, options): (String, Option<mlua::Table>, Option<mlua::Table>)| {
            let default_cwd = default_cwd.clone();
            let hub = hub.clone();
            async move {
                let scope = hub.scope(&lua)?;
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
                let mut max_output_bytes = crate::effects::DEFAULT_MAX_OUTPUT_BYTES;
                if let Some(options) = options {
                    if let Some(dir) = options.get::<Option<String>>("cwd")? {
                        cwd = dir;
                    }
                    if let Some(milliseconds) = options.get::<Option<f64>>("timeout")?
                        && milliseconds.is_finite()
                        && milliseconds > 0.0
                    {
                        timeout_ms = Some(milliseconds.min(u64::MAX as f64) as u64);
                    }
                    if let Some(userdata) = options.get::<Option<mlua::AnyUserData>>("signal")? {
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
                    on_data = options.get::<Option<mlua::Function>>("onData")?;
                    if let Some(limit) = options.get::<Option<usize>>("max_output_bytes")? {
                        max_output_bytes = limit;
                    }
                }
                let result = exec_command(
                    &hub,
                    scope,
                    ExecRequest {
                        command,
                        args: arg_vec,
                        cwd,
                        timeout_ms,
                        signal,
                        on_data,
                        max_output_bytes,
                    },
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
    pi.set("exec", exec)
}
