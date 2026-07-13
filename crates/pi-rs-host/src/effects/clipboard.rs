use std::collections::HashMap;
use std::io::Write as _;
use std::time::Duration;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;

use super::process::{ProcessRequest, collect};
use super::{ClipboardRequest, EffectError, EffectOptions, EffectTimeout, RequestContext};

const SUPPORTED_IMAGE_MIME_TYPES: [&str; 4] =
    ["image/png", "image/jpeg", "image/webp", "image/gif"];
const MAX_CLIPBOARD_BYTES: usize = 50 * 1024 * 1024;
const MAX_OSC52_ENCODED_LENGTH: usize = 100_000;

type Env = HashMap<String, String>;

#[derive(Debug)]
pub enum ClipboardResult {
    Image(Option<ClipboardImage>),
    Unit,
}

#[derive(Debug)]
pub struct ClipboardImage {
    pub bytes: Vec<u8>,
    pub mime_type: String,
}

fn env_truthy(env: &Env, key: &str) -> bool {
    env.get(key).is_some_and(|value| !value.is_empty())
}

fn is_wayland(env: &Env) -> bool {
    env_truthy(env, "WAYLAND_DISPLAY")
        || env
            .get("XDG_SESSION_TYPE")
            .is_some_and(|value| value == "wayland")
}

fn is_wsl(env: &Env) -> bool {
    env_truthy(env, "WSL_DISTRO_NAME")
        || env_truthy(env, "WSLENV")
        || std::fs::read_to_string("/proc/version")
            .is_ok_and(|value| value.to_ascii_lowercase().contains("microsoft"))
}

fn base_mime_type(value: &str) -> String {
    value
        .split(';')
        .next()
        .unwrap_or(value)
        .trim()
        .to_ascii_lowercase()
}

pub fn extension_for_image_mime_type(mime_type: &str) -> Option<&'static str> {
    match base_mime_type(mime_type).as_str() {
        "image/png" => Some("png"),
        "image/jpeg" => Some("jpg"),
        "image/webp" => Some("webp"),
        "image/gif" => Some("gif"),
        _ => None,
    }
}

fn preferred_mime(types: &[String]) -> Option<String> {
    for preferred in SUPPORTED_IMAGE_MIME_TYPES {
        if let Some(found) = types
            .iter()
            .find(|value| base_mime_type(value) == preferred)
        {
            return Some(found.trim().to_owned());
        }
    }
    types
        .iter()
        .find(|value| base_mime_type(value).starts_with("image/"))
        .map(|value| value.trim().to_owned())
}

async fn run(
    program: &str,
    args: &[&str],
    stdin: Option<Vec<u8>>,
    timeout: Duration,
    context: RequestContext,
) -> Result<Option<Vec<u8>>, EffectError> {
    let request = ProcessRequest {
        program: program.to_owned(),
        args: args.iter().map(|value| (*value).to_owned()).collect(),
        cwd: None,
        stdin,
        options: EffectOptions {
            timeout: EffectTimeout::After(timeout),
            stream_capacity: 2,
            max_output_bytes: MAX_CLIPBOARD_BYTES,
        },
    };
    match collect(request, context).await {
        Ok((stdout, _, output)) if output.code == 0 => Ok(Some(stdout)),
        Ok(_) => Ok(None),
        Err(EffectError::Timeout | EffectError::Cancelled) => Ok(None),
        Err(error) => Err(error),
    }
}

async fn read_image(
    env: Env,
    platform: &str,
    context: RequestContext,
) -> Result<Option<ClipboardImage>, EffectError> {
    if env.contains_key("TERMUX_VERSION") || platform != "linux" {
        return Ok(None);
    }
    let wayland = is_wayland(&env);
    let wsl = is_wsl(&env);
    let mut image = None;
    if wayland || wsl {
        if let Some(types) = run(
            "wl-paste",
            &["--list-types"],
            None,
            Duration::from_secs(1),
            context.clone(),
        )
        .await?
        {
            let types = String::from_utf8_lossy(&types)
                .lines()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_owned)
                .collect::<Vec<_>>();
            if let Some(mime) = preferred_mime(&types)
                && let Some(bytes) = run(
                    "wl-paste",
                    &["--type", &mime, "--no-newline"],
                    None,
                    Duration::from_secs(3),
                    context.clone(),
                )
                .await?
                && !bytes.is_empty()
            {
                image = Some(ClipboardImage {
                    bytes,
                    mime_type: base_mime_type(&mime),
                });
            }
        }
        if image.is_none() {
            let targets = run(
                "xclip",
                &["-selection", "clipboard", "-t", "TARGETS", "-o"],
                None,
                Duration::from_secs(1),
                context.clone(),
            )
            .await?
            .unwrap_or_default();
            let candidate_types = String::from_utf8_lossy(&targets)
                .lines()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_owned)
                .collect::<Vec<_>>();
            let mut try_types = preferred_mime(&candidate_types)
                .into_iter()
                .collect::<Vec<_>>();
            try_types.extend(
                SUPPORTED_IMAGE_MIME_TYPES
                    .iter()
                    .map(|value| (*value).to_owned()),
            );
            for mime in try_types {
                if let Some(bytes) = run(
                    "xclip",
                    &["-selection", "clipboard", "-t", &mime, "-o"],
                    None,
                    Duration::from_secs(3),
                    context.clone(),
                )
                .await?
                    && !bytes.is_empty()
                {
                    image = Some(ClipboardImage {
                        bytes,
                        mime_type: base_mime_type(&mime),
                    });
                    break;
                }
            }
        }
    }
    let Some(image) = image else {
        return Ok(None);
    };
    if !SUPPORTED_IMAGE_MIME_TYPES.contains(&image.mime_type.as_str()) {
        return Ok(
            crate::image::convert_bytes_to_png(&image.bytes).map(|bytes| ClipboardImage {
                bytes,
                mime_type: "image/png".to_owned(),
            }),
        );
    }
    Ok(Some(image))
}

fn emit_osc52(text: &str) -> bool {
    let encoded = BASE64.encode(text);
    if encoded.len() > MAX_OSC52_ENCODED_LENGTH {
        return false;
    }
    let mut stdout = std::io::stdout().lock();
    write!(stdout, "\x1b]52;c;{encoded}\x07")
        .and_then(|()| stdout.flush())
        .is_ok()
}

async fn write_text(
    text: String,
    env: Env,
    platform: &str,
    context: RequestContext,
) -> Result<(), EffectError> {
    let remote = env_truthy(&env, "SSH_CONNECTION")
        || env_truthy(&env, "SSH_CLIENT")
        || env_truthy(&env, "MOSH_CONNECTION");
    let input = || Some(text.as_bytes().to_vec());
    let mut copied = match platform {
        "darwin" => run(
            "pbcopy",
            &[],
            input(),
            Duration::from_secs(5),
            context.clone(),
        )
        .await?
        .is_some(),
        "win32" => run(
            "clip",
            &[],
            input(),
            Duration::from_secs(5),
            context.clone(),
        )
        .await?
        .is_some(),
        _ => false,
    };
    if platform == "linux" {
        if env.contains_key("TERMUX_VERSION") {
            copied = run(
                "termux-clipboard-set",
                &[],
                input(),
                Duration::from_secs(5),
                context.clone(),
            )
            .await?
            .is_some();
        }
        if !copied && is_wayland(&env) && env_truthy(&env, "WAYLAND_DISPLAY") {
            copied = run(
                "wl-copy",
                &[],
                input(),
                Duration::from_secs(5),
                context.clone(),
            )
            .await?
            .is_some();
        }
        if !copied && env_truthy(&env, "DISPLAY") {
            copied = run(
                "xclip",
                &["-selection", "clipboard"],
                input(),
                Duration::from_secs(5),
                context.clone(),
            )
            .await?
            .is_some();
            if !copied {
                copied = run(
                    "xsel",
                    &["--clipboard", "--input"],
                    input(),
                    Duration::from_secs(5),
                    context,
                )
                .await?
                .is_some();
            }
        }
    }
    if remote || !copied {
        copied = emit_osc52(&text) || copied;
    }
    if copied {
        Ok(())
    } else {
        Err(EffectError::Io("Failed to copy to clipboard".to_owned()))
    }
}

pub async fn execute(
    request: ClipboardRequest,
    context: RequestContext,
) -> Result<ClipboardResult, EffectError> {
    match request {
        ClipboardRequest::ReadImage { env, platform } => read_image(env, &platform, context)
            .await
            .map(ClipboardResult::Image),
        ClipboardRequest::WriteText {
            text,
            env,
            platform,
        } => {
            write_text(text, env, &platform, context).await?;
            Ok(ClipboardResult::Unit)
        }
    }
}
