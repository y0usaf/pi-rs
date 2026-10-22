//! `core/resolve-config-value.ts`: API keys and headers in auth.json and models.json are a literal,
//! a `$NAME`/`${NAME}` environment template (`$$` and `$!` escape), or `!command` output.

use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Eq)]
enum Part {
    Literal(String),
    Env(String),
}

fn is_name_start(c: char) -> bool {
    c.is_ascii_alphabetic() || c == '_'
}

fn is_name(text: &str) -> bool {
    let mut chars = text.chars();
    chars.next().is_some_and(is_name_start) && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

fn literal(parts: &mut Vec<Part>, value: &str) {
    if value.is_empty() {
        return;
    }
    if let Some(Part::Literal(last)) = parts.last_mut() {
        last.push_str(value);
    } else {
        parts.push(Part::Literal(value.to_owned()));
    }
}

fn template(config: &str) -> Vec<Part> {
    let mut parts = Vec::new();
    let mut index = 0;
    while index < config.len() {
        let Some(offset) = config[index..].find('$') else {
            literal(&mut parts, &config[index..]);
            break;
        };
        let dollar = index + offset;
        literal(&mut parts, &config[index..dollar]);
        let rest = &config[dollar + 1..];
        match rest.chars().next() {
            Some(c @ ('$' | '!')) => {
                literal(&mut parts, &c.to_string());
                index = dollar + 2;
            }
            Some('{') => match rest[1..].find('}') {
                None => {
                    literal(&mut parts, "$");
                    index = dollar + 1;
                }
                Some(end) => {
                    let name = &rest[1..1 + end];
                    if is_name(name) {
                        parts.push(Part::Env(name.to_owned()));
                    } else {
                        literal(&mut parts, &config[dollar..dollar + 2 + end + 1]);
                    }
                    index = dollar + 2 + end + 1;
                }
            },
            Some(c) if is_name_start(c) => {
                let len = rest
                    .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
                    .unwrap_or(rest.len());
                parts.push(Part::Env(rest[..len].to_owned()));
                index = dollar + 1 + len;
            }
            _ => {
                literal(&mut parts, "$");
                index = dollar + 1;
            }
        }
    }
    parts
}

pub type Env = BTreeMap<String, String>;

fn env_value(name: &str, env: Option<&Env>) -> Option<String> {
    env.and_then(|e| e.get(name))
        .filter(|v| !v.is_empty())
        .cloned()
        .or_else(|| std::env::var(name).ok().filter(|v| !v.is_empty()))
}

pub fn is_command(config: &str) -> bool {
    config.starts_with('!')
}

/// The variables a template names, in order, once each.
pub fn env_var_names(config: &str) -> Vec<String> {
    if is_command(config) {
        return Vec::new();
    }
    let mut names: Vec<String> = Vec::new();
    for part in template(config) {
        if let Part::Env(name) = part
            && !names.contains(&name)
        {
            names.push(name);
        }
    }
    names
}

/// Runs `!command` with `sh -c` (Node's `execSync`), 10 s timeout; trimmed stdout or None.
fn run_command(command: &str) -> Option<String> {
    use std::process::{Command, Stdio};
    let mut child = Command::new("/bin/sh")
        .arg("-c")
        .arg(command)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if std::time::Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
            Ok(None) => std::thread::sleep(std::time::Duration::from_millis(5)),
            Err(_) => return None,
        }
    };
    let mut out = String::new();
    use std::io::Read;
    child.stdout.take()?.read_to_string(&mut out).ok()?;
    if !status.success() {
        return None;
    }
    let out = out.trim();
    (!out.is_empty()).then(|| out.to_owned())
}

/// `resolveConfigValueUncached`.
pub fn resolve(config: &str, env: Option<&Env>) -> Option<String> {
    if is_command(config) {
        return run_command(&config[1..]);
    }
    let mut out = String::new();
    for part in template(config) {
        match part {
            Part::Literal(s) => out.push_str(&s),
            Part::Env(name) => out.push_str(&env_value(&name, env)?),
        }
    }
    Some(out)
}

/// `resolveConfigValueOrThrow`.
pub fn resolve_or_err(
    config: &str,
    description: &str,
    env: Option<&Env>,
) -> Result<String, String> {
    if let Some(value) = resolve(config, env) {
        return Ok(value);
    }
    if is_command(config) {
        return Err(format!(
            "Failed to resolve {description} from shell command: {}",
            &config[1..]
        ));
    }
    let missing: Vec<String> = env_var_names(config)
        .into_iter()
        .filter(|n| env_value(n, env).is_none())
        .collect();
    match missing.len() {
        0 => Err(format!("Failed to resolve {description}")),
        1 => Err(format!(
            "Failed to resolve {description} from environment variable: {}",
            missing[0]
        )),
        _ => Err(format!(
            "Failed to resolve {description} from environment variables: {}",
            missing.join(", ")
        )),
    }
}

/// `resolveHeadersOrThrow`.
pub fn resolve_headers(
    headers: Option<&BTreeMap<String, String>>,
    description: &str,
    env: Option<&Env>,
) -> Result<Option<BTreeMap<String, String>>, String> {
    let Some(headers) = headers else {
        return Ok(None);
    };
    let mut out = BTreeMap::new();
    for (key, value) in headers {
        out.insert(
            key.clone(),
            resolve_or_err(value, &format!("{description} header \"{key}\""), env)?,
        );
    }
    Ok((!out.is_empty()).then_some(out))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn templates() {
        let env: Env = [("A".to_owned(), "x".to_owned())].into();
        assert_eq!(
            resolve("k-$A-${A}$$!$!", Some(&env)).as_deref(),
            Some("k-x-x$!!")
        );
        assert_eq!(
            resolve("${not a name}", None).as_deref(),
            Some("${not a name}")
        );
        assert_eq!(resolve("plain", None).as_deref(), Some("plain"));
        assert_eq!(resolve("!echo hi", None).as_deref(), Some("hi"));
        assert_eq!(
            resolve_or_err("$PI_RS_NOPE_1$PI_RS_NOPE_2", "API key", None).unwrap_err(),
            "Failed to resolve API key from environment variables: PI_RS_NOPE_1, PI_RS_NOPE_2"
        );
    }
}
