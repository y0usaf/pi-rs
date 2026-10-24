//! `--fork <path|id>` (`cli/args.ts`, `main.ts`, `core/session-manager.ts` `forkFrom`): a copy of
//! another session, under a new id, in this project's session directory.

use std::path::Path;

use pi_durable::storage::sqlite;

/// The session file `arg` names: a path when it is one, else `<dir>/<id>.db`, else the one file in
/// `dir` whose id starts with `arg`.
pub fn resolve(dir: &str, arg: &str) -> Option<String> {
    let direct = Path::new(arg);
    if direct.is_file() {
        return Some(direct.to_string_lossy().into_owned());
    }
    let exact = format!("{dir}/{arg}.db");
    if Path::new(&exact).is_file() {
        return Some(exact);
    }
    let mut matches: Vec<String> = std::fs::read_dir(dir)
        .ok()?
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "db"))
        .filter(|path| {
            path.file_stem()
                .and_then(|stem| stem.to_str())
                .is_some_and(|stem| stem.starts_with(arg))
        })
        .map(|path| path.to_string_lossy().into_owned())
        .collect();
    matches.sort();
    match matches.len() {
        1 => matches.pop(),
        _ => None,
    }
}

/// Copy `source` into `dir` under a new id and return the new file. A session whose run is still in
/// flight is refused: its copy would resume that run.
pub fn copy(source: &str, dir: &str, id: Option<&str>) -> Result<String, String> {
    let path = Path::new(source);
    if !path.is_file() {
        return Err(format!("Cannot fork: source session is missing: {source}"));
    }
    if sqlite::in_flight(path).map_err(|e| format!("Cannot fork: {e}"))? {
        return Err(format!(
            "Cannot fork: {source} has a run in flight, and its copy would resume it"
        ));
    }
    std::fs::create_dir_all(dir).map_err(|e| format!("Error: {e}"))?;
    let target = match id {
        Some(id) => format!("{dir}/{id}.db"),
        None => new_path(dir),
    };
    if Path::new(&target).exists() {
        return Err(format!(
            "Session already exists with id '{}'",
            id.unwrap_or_default()
        ));
    }
    sqlite::copy_database(path, Path::new(&target)).map_err(|e| format!("Cannot fork: {e}"))?;
    Ok(target)
}

fn new_path(dir: &str) -> String {
    let millis = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    let id =
        pi_durable::ai::utils::uuid::uuidv7(Some(millis)).unwrap_or_else(|_| millis.to_string());
    format!("{dir}/{id}.db")
}
