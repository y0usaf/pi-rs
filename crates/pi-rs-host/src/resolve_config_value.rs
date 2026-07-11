//! Port of `core/resolve-config-value.ts` — resolve configuration
//! values that may be shell commands (`!cmd`), environment-variable
//! templates (`$NAME` / `${NAME}`), or literals.
//!
//! WS2.6 subset: the consumers landed so far are `auth-storage.ts`
//! (stored `api_key` credentials) and the registry's auth-status
//! checks. The header-resolution helpers and the legacy env-var-name
//! migration land with models.json / `registerProvider` glue (WS7).
//!
//! Divergence (recorded): the spec's Windows configured-shell branch is
//! not ported — commands run through `sh -c` with the spec's 10s
//! timeout and trimmed-stdout semantics.

use std::collections::HashMap;
use std::io::Read;
use std::process::{Command, Stdio};
use std::sync::{LazyLock, Mutex, PoisonError};
use std::time::{Duration, Instant};

use thiserror::Error;

/// Spec: the errors thrown by `resolveConfigValueOrThrow`.
#[derive(Debug, Error)]
pub enum ConfigValueError {
    #[error("Failed to resolve {description} from shell command: {command}")]
    Command {
        description: String,
        command: String,
    },
    #[error("Failed to resolve {description} from environment variable: {name}")]
    EnvVar { description: String, name: String },
    #[error("Failed to resolve {description} from environment variables: {names}")]
    EnvVars { description: String, names: String },
    #[error("Failed to resolve {description}")]
    Other { description: String },
}

// Cache for shell command results (persists for process lifetime).
static COMMAND_RESULT_CACHE: LazyLock<Mutex<HashMap<String, Option<String>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

fn is_env_var_name_char(c: char, first: bool) -> bool {
    c == '_' || c.is_ascii_alphabetic() || (!first && c.is_ascii_digit())
}

/// Spec: `ENV_VAR_NAME_RE` — `^[A-Za-z_][A-Za-z0-9_]*$`.
fn is_env_var_name(s: &str) -> bool {
    let mut chars = s.chars();
    match chars.next() {
        Some(c) if is_env_var_name_char(c, true) => {}
        _ => return false,
    }
    chars.all(|c| is_env_var_name_char(c, false))
}

#[derive(Clone, Debug, PartialEq)]
enum TemplatePart {
    Literal(String),
    Env(String),
}

enum ConfigValueReference {
    Command(String),
    Template(Vec<TemplatePart>),
}

fn append_literal(parts: &mut Vec<TemplatePart>, value: &str) {
    if value.is_empty() {
        return;
    }
    if let Some(TemplatePart::Literal(previous)) = parts.last_mut() {
        previous.push_str(value);
        return;
    }
    parts.push(TemplatePart::Literal(value.to_owned()));
}

/// Spec: `parseConfigValueTemplate`.
fn parse_config_value_template(config: &str) -> Vec<TemplatePart> {
    let mut parts: Vec<TemplatePart> = Vec::new();
    let mut index = 0usize;

    while index < config.len() {
        let Some(dollar_offset) = config[index..].find('$') else {
            append_literal(&mut parts, &config[index..]);
            break;
        };
        let dollar_index = index + dollar_offset;

        append_literal(&mut parts, &config[index..dollar_index]);
        let next_char = config[dollar_index + 1..].chars().next();

        match next_char {
            Some(c @ ('$' | '!')) => {
                append_literal(&mut parts, &c.to_string());
                index = dollar_index + 2;
            }
            Some('{') => {
                let Some(end_offset) = config[dollar_index + 2..].find('}') else {
                    append_literal(&mut parts, "$");
                    index = dollar_index + 1;
                    continue;
                };
                let end_index = dollar_index + 2 + end_offset;
                let name = &config[dollar_index + 2..end_index];
                if is_env_var_name(name) {
                    parts.push(TemplatePart::Env(name.to_owned()));
                } else {
                    append_literal(&mut parts, &config[dollar_index..=end_index]);
                }
                index = end_index + 1;
            }
            _ => {
                // Spec: `ENV_VAR_NAME_PREFIX_RE` — the longest leading
                // env-var-name run after the `$`.
                let rest = &config[dollar_index + 1..];
                let name_len = rest
                    .char_indices()
                    .take_while(|&(i, c)| is_env_var_name_char(c, i == 0))
                    .count();
                if name_len > 0 {
                    parts.push(TemplatePart::Env(rest[..name_len].to_owned()));
                    index = dollar_index + 1 + name_len;
                } else {
                    append_literal(&mut parts, "$");
                    index = dollar_index + 1;
                }
            }
        }
    }

    parts
}

/// Spec: `parseConfigValueReference`.
fn parse_config_value_reference(config: &str) -> ConfigValueReference {
    if config.starts_with('!') {
        return ConfigValueReference::Command(config.to_owned());
    }
    ConfigValueReference::Template(parse_config_value_template(config))
}

/// Spec: `resolveEnvConfigValue` — empty values count as unset.
fn resolve_env_config_value(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|v| !v.is_empty())
}

fn template_env_var_names(parts: &[TemplatePart]) -> Vec<String> {
    let mut names: Vec<String> = Vec::new();
    for part in parts {
        if let TemplatePart::Env(name) = part
            && !names.contains(name)
        {
            names.push(name.clone());
        }
    }
    names
}

fn resolve_template(parts: &[TemplatePart]) -> Option<String> {
    let mut resolved = String::new();
    for part in parts {
        match part {
            TemplatePart::Literal(value) => resolved.push_str(value),
            TemplatePart::Env(name) => resolved.push_str(&resolve_env_config_value(name)?),
        }
    }
    Some(resolved)
}

/// Spec: `getConfigValueEnvVarNames`.
pub fn get_config_value_env_var_names(config: &str) -> Vec<String> {
    match parse_config_value_reference(config) {
        ConfigValueReference::Template(parts) => template_env_var_names(&parts),
        ConfigValueReference::Command(_) => Vec::new(),
    }
}

/// Spec: `getMissingConfigValueEnvVarNames`.
pub fn get_missing_config_value_env_var_names(config: &str) -> Vec<String> {
    get_config_value_env_var_names(config)
        .into_iter()
        .filter(|name| resolve_env_config_value(name).is_none())
        .collect()
}

/// Spec: `isCommandConfigValue`.
pub fn is_command_config_value(config: &str) -> bool {
    matches!(
        parse_config_value_reference(config),
        ConfigValueReference::Command(_)
    )
}

/// Spec: `isConfigValueConfigured`.
pub fn is_config_value_configured(config: &str) -> bool {
    get_missing_config_value_env_var_names(config).is_empty()
}

/// Spec: `executeWithDefaultShell` — `sh -c <command>`, 10s timeout,
/// trimmed stdout, failure → `None`.
fn execute_with_default_shell(command: &str) -> Option<String> {
    let mut child = Command::new("sh")
        .arg("-c")
        .arg(command)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;

    let mut stdout = child.stdout.take()?;
    let reader = std::thread::spawn(move || {
        let mut buffer = String::new();
        let _ = stdout.read_to_string(&mut buffer);
        buffer
    });

    let deadline = Instant::now() + Duration::from_millis(10_000);
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => {
                if Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    let _ = reader.join();
                    return None;
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            Err(_) => return None,
        }
    };

    let output = reader.join().ok()?;
    if !status.success() {
        return None;
    }
    let trimmed = output.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_owned())
    }
}

/// Spec: `executeCommandUncached` (the `!` prefix is stripped here).
fn execute_command_uncached(command_config: &str) -> Option<String> {
    execute_with_default_shell(&command_config[1..])
}

/// Spec: `executeCommand` — process-lifetime cache.
fn execute_command(command_config: &str) -> Option<String> {
    let mut cache = COMMAND_RESULT_CACHE
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    if let Some(result) = cache.get(command_config) {
        return result.clone();
    }
    let result = execute_command_uncached(command_config);
    cache.insert(command_config.to_owned(), result.clone());
    result
}

/// Spec: `resolveConfigValue`.
pub fn resolve_config_value(config: &str) -> Option<String> {
    match parse_config_value_reference(config) {
        ConfigValueReference::Command(command) => execute_command(&command),
        ConfigValueReference::Template(parts) => resolve_template(&parts),
    }
}

/// Spec: `resolveConfigValueUncached`.
pub fn resolve_config_value_uncached(config: &str) -> Option<String> {
    match parse_config_value_reference(config) {
        ConfigValueReference::Command(command) => execute_command_uncached(&command),
        ConfigValueReference::Template(parts) => resolve_template(&parts),
    }
}

/// Spec: `resolveConfigValueOrThrow`.
pub fn resolve_config_value_or_throw(
    config: &str,
    description: &str,
) -> Result<String, ConfigValueError> {
    if let Some(resolved) = resolve_config_value_uncached(config) {
        return Ok(resolved);
    }

    match parse_config_value_reference(config) {
        ConfigValueReference::Command(command) => Err(ConfigValueError::Command {
            description: description.to_owned(),
            command: command[1..].to_owned(),
        }),
        ConfigValueReference::Template(_) => {
            let missing = get_missing_config_value_env_var_names(config);
            match missing.len() {
                1 => Err(ConfigValueError::EnvVar {
                    description: description.to_owned(),
                    name: missing[0].clone(),
                }),
                n if n > 1 => Err(ConfigValueError::EnvVars {
                    description: description.to_owned(),
                    names: missing.join(", "),
                }),
                _ => Err(ConfigValueError::Other {
                    description: description.to_owned(),
                }),
            }
        }
    }
}

/// Spec: `clearConfigValueCache` — exported for testing.
pub fn clear_config_value_cache() {
    COMMAND_RESULT_CACHE
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clear();
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    #[test]
    fn literals_pass_through() {
        assert_eq!(resolve_config_value("sk-plain"), Some("sk-plain".into()));
    }

    #[test]
    fn dollar_escapes() {
        assert_eq!(resolve_config_value("a$$b"), Some("a$b".into()));
        assert_eq!(resolve_config_value("a$!b"), Some("a!b".into()));
    }

    #[test]
    fn env_template_resolves_and_missing_is_none() {
        // SAFETY: test process env; tests in this file are the only readers.
        unsafe { std::env::set_var("PI_RS_RCV_TEST_VAR", "hello") };
        assert_eq!(
            resolve_config_value("pre-$PI_RS_RCV_TEST_VAR-post"),
            Some("pre-hello-post".into())
        );
        assert_eq!(
            resolve_config_value("${PI_RS_RCV_TEST_VAR}!"),
            Some("hello!".into())
        );
        assert_eq!(resolve_config_value("$PI_RS_RCV_TEST_MISSING"), None);
        assert_eq!(
            get_missing_config_value_env_var_names("$PI_RS_RCV_TEST_MISSING"),
            vec!["PI_RS_RCV_TEST_MISSING".to_owned()]
        );
    }

    #[test]
    fn unterminated_brace_is_literal_dollar() {
        assert_eq!(resolve_config_value("a${b"), Some("a${b".into()));
    }

    #[test]
    fn invalid_brace_name_is_literal() {
        assert_eq!(
            resolve_config_value("${not valid}"),
            Some("${not valid}".into())
        );
    }

    #[test]
    fn command_values_execute_and_cache() {
        clear_config_value_cache();
        assert!(is_command_config_value("!echo hi"));
        assert_eq!(resolve_config_value("!echo hi"), Some("hi".into()));
        // Failed commands resolve to None.
        assert_eq!(resolve_config_value("!false"), None);
    }

    #[test]
    fn or_throw_error_messages_match_spec() {
        let err = resolve_config_value_or_throw("!false", "API key for provider \"x\"")
            .unwrap_err()
            .to_string();
        assert_eq!(
            err,
            "Failed to resolve API key for provider \"x\" from shell command: false"
        );
        let err = resolve_config_value_or_throw("$PI_RS_RCV_TEST_MISSING2", "API key")
            .unwrap_err()
            .to_string();
        assert_eq!(
            err,
            "Failed to resolve API key from environment variable: PI_RS_RCV_TEST_MISSING2"
        );
    }
}
