//! Stored API-key value resolution.
//!
//! Literal values may interpolate environment variables; values beginning with
//! `!` execute through the configured shell with a hard timeout. Command output
//! is cached for the process lifetime and never included in errors or debug
//! output.

use std::collections::HashMap;
use std::sync::{LazyLock, Mutex, PoisonError};
use std::time::Duration;

static COMMAND_CACHE: LazyLock<Mutex<HashMap<String, Option<String>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// Resolve a stored API-key expression without exposing command failures.
pub async fn resolve_config_value(config: &str) -> Option<String> {
    if let Some(command) = config.strip_prefix('!') {
        return resolve_command(config, command).await;
    }
    resolve_template(config)
}

fn resolve_template(config: &str) -> Option<String> {
    let mut output = String::new();
    let mut characters = config.char_indices().peekable();
    while let Some((_, character)) = characters.next() {
        if character != '$' {
            output.push(character);
            continue;
        }
        let Some(&(next_index, next)) = characters.peek() else {
            output.push('$');
            continue;
        };
        if matches!(next, '$' | '!') {
            output.push(next);
            characters.next();
            continue;
        }
        if next == '{' {
            characters.next();
            let name_start = next_index + 1;
            let Some(end) = config[name_start..].find('}').map(|end| name_start + end) else {
                output.push('$');
                output.push('{');
                continue;
            };
            while characters.peek().is_some_and(|(index, _)| *index <= end) {
                characters.next();
            }
            let name = &config[name_start..end];
            if valid_env_name(name) {
                output.push_str(&std::env::var(name).ok().filter(|value| !value.is_empty())?);
            } else {
                output.push_str(&config[next_index - 1..=end]);
            }
            continue;
        }
        if next == '_' || next.is_ascii_alphabetic() {
            let start = next_index;
            let mut end = start;
            while let Some(&(index, candidate)) = characters.peek() {
                if candidate != '_' && !candidate.is_ascii_alphanumeric() {
                    break;
                }
                characters.next();
                end = index + candidate.len_utf8();
            }
            let name = &config[start..end];
            output.push_str(&std::env::var(name).ok().filter(|value| !value.is_empty())?);
            continue;
        }
        output.push('$');
    }
    Some(output)
}

fn valid_env_name(name: &str) -> bool {
    let mut characters = name.chars();
    characters
        .next()
        .is_some_and(|character| character == '_' || character.is_ascii_alphabetic())
        && characters.all(|character| character == '_' || character.is_ascii_alphanumeric())
}

async fn resolve_command(cache_key: &str, command: &str) -> Option<String> {
    if let Some(cached) = COMMAND_CACHE
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .get(cache_key)
        .cloned()
    {
        return cached;
    }

    #[cfg(windows)]
    let (shell, argument) = ("cmd", "/C");
    #[cfg(not(windows))]
    let (shell, argument) = (
        std::env::var("SHELL")
            .ok()
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| "/bin/sh".into()),
        "-c",
    );

    let mut process = tokio::process::Command::new(shell);
    process
        .arg(argument)
        .arg(command)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true);
    let resolved = match tokio::time::timeout(Duration::from_secs(10), process.output()).await {
        Ok(Ok(output)) if output.status.success() => {
            let value = String::from_utf8_lossy(&output.stdout).trim().to_owned();
            (!value.is_empty()).then_some(value)
        }
        _ => None,
    };
    COMMAND_CACHE
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .insert(cache_key.to_owned(), resolved.clone());
    resolved
}
