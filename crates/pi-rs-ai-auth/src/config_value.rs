//! Stored API-key value resolution.
//!
//! Literal values may interpolate environment variables; values beginning with
//! `!` execute through the configured shell with a hard timeout. Command output
//! is cached for the process lifetime and never included in errors or debug
//! output. The process-wide cache has a hard 64-entry bound and deterministic
//! first-in/first-out eviction: hits and updates do not refresh insertion order.

use std::collections::{HashMap, VecDeque};
use std::sync::{LazyLock, Mutex, PoisonError};
use std::time::Duration;

const COMMAND_CACHE_CAPACITY: usize = 64;

static COMMAND_CACHE: LazyLock<Mutex<CommandCache>> =
    LazyLock::new(|| Mutex::new(CommandCache::new(COMMAND_CACHE_CAPACITY)));

struct CommandCache {
    capacity: usize,
    values: HashMap<String, Option<String>>,
    insertion_order: VecDeque<String>,
}

impl CommandCache {
    fn new(capacity: usize) -> Self {
        assert!(capacity > 0, "command cache capacity must be non-zero");
        Self {
            capacity,
            values: HashMap::new(),
            insertion_order: VecDeque::new(),
        }
    }

    fn get(&self, key: &str) -> Option<Option<String>> {
        self.values.get(key).cloned()
    }

    fn insert(&mut self, key: String, value: Option<String>) {
        if let Some(cached) = self.values.get_mut(&key) {
            *cached = value;
            return;
        }
        while self.values.len() >= self.capacity {
            if let Some(oldest) = self.insertion_order.pop_front() {
                self.values.remove(&oldest);
            }
        }
        self.insertion_order.push_back(key.clone());
        self.values.insert(key, value);
    }
}

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

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Barrier, Mutex, PoisonError};

    use super::CommandCache;

    #[test]
    fn command_cache_evicts_fifo_without_refreshing_hits_or_updates() {
        let mut cache = CommandCache::new(2);
        cache.insert("first".into(), Some("one".into()));
        cache.insert("second".into(), Some("two".into()));
        assert_eq!(cache.get("first"), Some(Some("one".into())));
        cache.insert("first".into(), Some("updated".into()));
        cache.insert("third".into(), None);

        assert_eq!(cache.get("first"), None);
        assert_eq!(cache.get("second"), Some(Some("two".into())));
        assert_eq!(cache.get("third"), Some(None));
    }

    #[test]
    fn command_cache_remains_hard_bounded_under_concurrent_insertions() {
        const CAPACITY: usize = 8;
        const WORKERS: usize = 16;
        let cache = Arc::new(Mutex::new(CommandCache::new(CAPACITY)));
        let barrier = Arc::new(Barrier::new(WORKERS));
        let workers: Vec<_> = (0..WORKERS)
            .map(|worker| {
                let cache = Arc::clone(&cache);
                let barrier = Arc::clone(&barrier);
                std::thread::spawn(move || {
                    barrier.wait();
                    for sequence in 0..32 {
                        cache
                            .lock()
                            .unwrap_or_else(PoisonError::into_inner)
                            .insert(format!("{worker}-{sequence}"), Some(sequence.to_string()));
                    }
                })
            })
            .collect();
        for worker in workers {
            assert!(worker.join().is_ok(), "cache worker must finish");
        }

        let cache = cache.lock().unwrap_or_else(PoisonError::into_inner);
        assert_eq!(cache.values.len(), CAPACITY);
        assert_eq!(cache.insertion_order.len(), CAPACITY);
        assert!(
            cache
                .insertion_order
                .iter()
                .all(|key| cache.values.contains_key(key))
        );
    }
}
