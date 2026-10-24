//! Program Status Protocol (OSC 7501): tell the terminal whether the program is idle, working,
//! blocked, done or failed (upstream `program-status.ts`).

/// The status to report.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProgramStatus {
    /// `idle`, `working`, `blocked`, `done`, `error`, or `clear` to remove the status.
    pub state: String,
    /// Stable program name matching `[A-Za-z0-9_.+-]{1,32}`; other values are left out.
    pub app: Option<String>,
    /// For `blocked`: `permission`, `question` or `auth`.
    pub kind: Option<String>,
    /// One human-readable line.
    pub message: Option<String>,
}

impl ProgramStatus {
    pub fn new(state: &str) -> Self {
        ProgramStatus {
            state: state.into(),
            app: None,
            kind: None,
            message: None,
        }
    }
}

/// Feature detection query; a supporting terminal replies with the same body.
pub const PROGRAM_STATUS_QUERY: &str = "\x1b]7501;?\x1b\\";

/// Whether `seq` is the reply to [`PROGRAM_STATUS_QUERY`].
pub fn is_program_status_reply(seq: &str) -> bool {
    let Some(rest) = seq.strip_prefix("\x1b]7501;?") else {
        return false;
    };
    let body = rest
        .strip_suffix('\x07')
        .or_else(|| rest.strip_suffix("\x1b\\"));
    body.is_some_and(|b| !b.contains(['\x07', '\x1b']))
}

const MAX_MESSAGE_BYTES: usize = 2048;

fn valid_app(app: &str) -> bool {
    (1..=32).contains(&app.chars().count())
        && app
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '+' | '-'))
}

fn base64(bytes: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for chunk in bytes.chunks(3) {
        let b = [
            chunk[0],
            *chunk.get(1).unwrap_or(&0),
            *chunk.get(2).unwrap_or(&0),
        ];
        let n = (b[0] as u32) << 16 | (b[1] as u32) << 8 | b[2] as u32;
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(T[(n >> (18 - 6 * i) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

/// Encode a status report. Control characters in the message become spaces.
pub fn format_program_status(status: &ProgramStatus) -> String {
    let mut pairs = vec![format!("state={}", status.state)];
    if let Some(app) = status.app.as_deref().filter(|a| valid_app(a)) {
        pairs.push(format!("app={app}"));
    }
    if status.state == "blocked"
        && let Some(kind) = status.kind.as_deref().filter(|k| !k.is_empty())
    {
        pairs.push(format!("kind={kind}"));
    }
    let mut cleaned = String::new();
    let mut in_ctrl = false;
    for c in status.message.as_deref().unwrap_or("").chars() {
        let ctrl = (c as u32) <= 0x1f || (0x7f..=0x9f).contains(&(c as u32));
        if ctrl {
            if !in_ctrl {
                cleaned.push(' ');
            }
        } else {
            cleaned.push(c);
        }
        in_ctrl = ctrl;
    }
    let trimmed = crate::utils::js_trim(&cleaned);
    let mut end = 0;
    for (i, c) in trimmed.char_indices() {
        if i + c.len_utf8() > MAX_MESSAGE_BYTES {
            break;
        }
        end = i + c.len_utf8();
    }
    let message = &trimmed[..end];
    if !message.is_empty() {
        pairs.push(format!("msg={}", base64(message.as_bytes())));
    }
    format!("\x1b]7501;{}\x1b\\", pairs.join(":"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats() {
        let mut s = ProgramStatus::new("blocked");
        s.app = Some("pi".into());
        s.kind = Some("question".into());
        s.message = Some("  hi\nthere\x07 ".into());
        assert_eq!(
            format_program_status(&s),
            "\x1b]7501;state=blocked:app=pi:kind=question:msg=aGkgdGhlcmU=\x1b\\"
        );
        assert!(is_program_status_reply("\x1b]7501;?\x07"));
        assert!(is_program_status_reply("\x1b]7501;?v=1\x1b\\"));
        assert!(!is_program_status_reply("\x1b]7501;x\x07"));
    }
}
