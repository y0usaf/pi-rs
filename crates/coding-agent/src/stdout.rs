//! stdout as upstream's `console.log` leaves it (`cli/list-models.ts`, `modes/print-mode.ts`): a
//! reader that closed the pipe ends the output instead of failing the run, since Node drops EPIPE
//! on stdout.

use std::io::Write;

/// Writes `text` to stdout; a broken pipe is not an error.
pub fn write(text: &str) -> Result<(), String> {
    let mut out = std::io::stdout().lock();
    out.write_all(text.as_bytes())
        .and_then(|()| out.flush())
        .or_else(|error| match error.kind() {
            std::io::ErrorKind::BrokenPipe => Ok(()),
            _ => Err(error.to_string()),
        })
}
