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

/// How long one input wait blocks before the loop re-measures the terminal.
///
/// A resize arrives as an `ioctl` value, not as a byte on stdin, so an
/// idle loop must wake to notice it. 100 ms is under one human frame of
/// perceived delay and costs ten wakeups per idle second.
const SIZE_POLL_INTERVAL: std::time::Duration = std::time::Duration::from_millis(100);

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
/// The loop waits for either of the two things the outside world can say:
/// bytes on stdin, or a new terminal size. Both leave as bounded events to
/// the application root, and returned ANSI frames are presented. Raw mode is
/// enabled for the loop duration and restored on exit.
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

/// Build the event that reports one new terminal size.
///
/// Pure on purpose: the loop decides *when* a size changed, this decides
/// *what* the product is told, and a test can check the second without a
/// terminal.
pub(crate) fn resize_event(columns: u16, rows: u16) -> serde_json::Value {
    serde_json::json!({
        "kind": "resize",
        "columns": columns,
        "rows": rows,
    })
}

fn loop_inner(
    host: &Host,
    context: &serde_json::Value,
    output: &mut dyn Write,
) -> Result<(), LauncherError> {
    let mut stdin = std::io::stdin().lock();
    let mut buffer = [0_u8; MAX_INPUT_BATCH];
    // The launcher already measured the size for the startup frame, so the
    // loop starts from the same value and only reports differences.
    let mut size = context
        .get("terminal")
        .and_then(|terminal| {
            let columns = terminal.get("columns")?.as_u64()?;
            let rows = terminal.get("rows")?.as_u64()?;
            Some((u16::try_from(columns).ok()?, u16::try_from(rows).ok()?))
        })
        .unwrap_or_else(pi_rs_tui::terminal::live_terminal_dimensions);
    loop {
        let readable = match pi_rs_tui::terminal::stdin_readable(Some(SIZE_POLL_INTERVAL)) {
            Ok(readable) => readable,
            // A signal interrupted the wait; re-measure and wait again.
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => false,
            Err(error) => return Err(LauncherError::Output(error)),
        };
        let current = pi_rs_tui::terminal::live_terminal_dimensions();
        if current != size {
            size = current;
            let batch = host
                .dispatch(DispatchRequest::new(
                    RootKind::Application,
                    resize_event(current.0, current.1),
                    context.clone(),
                ))
                .map_err(LauncherError::Dispatch)?;
            if settle_batch(&batch, output)? {
                return Ok(());
            }
        }
        if !readable {
            continue;
        }
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
