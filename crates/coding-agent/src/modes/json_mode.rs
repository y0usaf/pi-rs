//! `--mode json`: print mode with the session's events as JSON lines instead of the final text
//! (`runPrintMode` with mode "json"). The messages go as `-p` sends them, one run after another.

use pi_durable::cancel::CancelToken;
use pi_durable::harness::harness::{Conversation, Harness};

use super::Output;
use super::json_event;
use crate::print::{self, RunInput};

/// The events between starting and stopping the stream trail the runs that produce them by a
/// scheduler delivery, so the stream is drained until nothing new has arrived for a moment.
const QUIET: std::time::Duration = std::time::Duration::from_millis(150);
const SETTLE_LIMIT: std::time::Duration = std::time::Duration::from_secs(5);

pub fn run(
    harness: &Harness,
    root: &Conversation,
    input: &RunInput,
    cancel: &CancelToken,
) -> Result<(), String> {
    let stream = root.events(cancel).map_err(|e| e.to_string())?;
    let output = Output::default();
    let seen = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let counted = std::sync::Arc::clone(&seen);
    let printer = output.clone();
    stream
        .start(move |events, _| {
            for event in events {
                for line in json_event::lines(event) {
                    printer.line(&line);
                    counted.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                }
            }
            Ok(())
        })
        .map_err(|e| e.to_string())?;

    let outcome = print::run_turns(harness, root, input, cancel);

    let mut last = seen.load(std::sync::atomic::Ordering::SeqCst);
    let start = std::time::Instant::now();
    let mut quiet = std::time::Instant::now();
    while quiet.elapsed() < QUIET && start.elapsed() < SETTLE_LIMIT {
        let _ = root.wait_for_idle(cancel);
        std::thread::sleep(std::time::Duration::from_millis(20));
        let now = seen.load(std::sync::atomic::Ordering::SeqCst);
        if now != last {
            last = now;
            quiet = std::time::Instant::now();
        }
    }
    stream.stop();

    outcome.map_err(|e| e.to_string())
}
