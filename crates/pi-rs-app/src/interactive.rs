//! Generic interactive product loop.
//!
//! After the startup dispatch, the launcher enters a bounded read–dispatch–
//! settle cycle: terminal bytes in, immutable input snapshots dispatched to
//! the active application root, validated action batches out. Frame
//! presentation and shutdown are Rust mechanism; the meaning of every other
//! action stays Lua policy.

use std::io::{Read, Write};

use pi_rs_host::Host;
use pi_rs_host::kernel::{DispatchBatch, DispatchRequest, RootKind};

use crate::launcher::LauncherError;

/// Well-known action kinds the loop interprets as mechanism.
const ACTION_ANSI: &str = "ansi";
const ACTION_SHUTDOWN: &str = "shutdown";

/// Maximum bytes read from stdin in one read before dispatching.
const MAX_INPUT_BATCH: usize = 4_096;

/// Process one action batch, writing ANSI frames to the terminal.
///
/// Returns `true` when the batch contains a shutdown action.
pub(crate) fn settle_batch(
    batch: &DispatchBatch,
    output: &mut dyn Write,
) -> Result<bool, LauncherError> {
    let mut shutdown = false;
    for action in &batch.actions {
        match action.kind.as_str() {
            ACTION_ANSI => {
                if let Some(data) = action.payload.get("data").and_then(|v| v.as_str()) {
                    output
                        .write_all(data.as_bytes())
                        .map_err(LauncherError::Output)?;
                    output.flush().map_err(LauncherError::Output)?;
                }
            }
            ACTION_SHUTDOWN => {
                shutdown = true;
            }
            _ => {
                // Unknown actions are product policy; the loop ignores them.
            }
        }
    }
    Ok(shutdown)
}

/// Run the interactive loop until a shutdown action or stdin EOF.
///
/// The loop reads raw terminal bytes, dispatches bounded input events to the
/// application root, and presents returned ANSI frames. Raw mode is enabled
/// for the loop duration and restored on exit.
pub fn run_loop(
    host: &Host,
    context: &serde_json::Value,
    output: &mut dyn Write,
) -> Result<(), LauncherError> {
    let was_raw = crossterm::terminal::is_raw_mode_enabled().unwrap_or(false);
    if !was_raw {
        crossterm::terminal::enable_raw_mode().map_err(|error| {
            LauncherError::Arguments(format!("cannot enable raw mode: {error}"))
        })?;
    }
    let result = loop_inner(host, context, output);
    if !was_raw {
        let _ = crossterm::terminal::disable_raw_mode();
    }
    result
}

fn loop_inner(
    host: &Host,
    context: &serde_json::Value,
    output: &mut dyn Write,
) -> Result<(), LauncherError> {
    let mut stdin = std::io::stdin().lock();
    let mut buffer = [0_u8; MAX_INPUT_BATCH];
    loop {
        let count = stdin.read(&mut buffer).map_err(LauncherError::Output)?;
        if count == 0 {
            // stdin EOF — clean exit.
            return Ok(());
        }
        let data = String::from_utf8_lossy(&buffer[..count]).into_owned();
        let event = serde_json::json!({
            "kind": "input",
            "data": data,
        });
        let batch = host
            .dispatch(DispatchRequest::new(
                RootKind::Application,
                event,
                context.clone(),
            ))
            .map_err(LauncherError::Dispatch)?;
        if settle_batch(&batch, output)? {
            return Ok(());
        }
    }
}
