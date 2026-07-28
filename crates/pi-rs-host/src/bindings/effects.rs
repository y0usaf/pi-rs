//! Versioned bounded filesystem/process/environment effects and explicit
//! cancellation.

const DEFAULT_FILE_BYTES: usize = 1024 * 1024;
const MAX_FILE_BYTES: usize = 8 * 1024 * 1024;
const DEFAULT_LIST_ENTRIES: usize = 1_024;
const MAX_LIST_ENTRIES: usize = 16_384;
const DEFAULT_PROCESS_BYTES: usize = 1024 * 1024;
const MAX_PROCESS_BYTES: usize = crate::effects::DEFAULT_MAX_OUTPUT_BYTES;
const DEFAULT_PROCESS_TIMEOUT_MS: u64 = 30_000;
const MAX_PROCESS_TIMEOUT_MS: u64 = 300_000;

pub(crate) fn install(
    lua: &mlua::Lua,
    pi: &mlua::Table,
    cwd: &str,
    hub: crate::effects::EffectHub,
    environment: std::collections::BTreeMap<String, String>,
) -> mlua::Result<()> {
    let bridge = lua.create_table()?;
    crate::os::install(lua, &bridge, cwd, hub.clone())?;
    crate::exec::install(lua, &bridge, cwd, hub.clone())?;
    crate::effects::install(lua, &bridge, hub)?;
    let fs: mlua::Table = bridge.get("fs")?;
    let stat: mlua::Function = fs.get("stat")?;
    let stat_entry: mlua::Function = fs.get("stat")?;
    let exists: mlua::Function = fs.get("exists")?;
    let read_dir: mlua::Function = fs.get("read_dir")?;
    let mkdir: mlua::Function = fs.get("mkdir")?;
    let remove_file: mlua::Function = fs.get("remove_file")?;
    let read_file: mlua::Function = fs.get("read_file")?;
    let write_file: mlua::Function = fs.get("write_file")?;
    let exec: mlua::Function = bridge.get("exec")?;

    let filesystem = lua.create_table()?;
    filesystem.set("default_max_bytes", DEFAULT_FILE_BYTES)?;
    filesystem.set("max_bytes", MAX_FILE_BYTES)?;
    filesystem.set(
        "read",
        lua.create_async_function(move |_, (path, limit): (String, Option<usize>)| {
            let stat = stat.clone();
            let read_file = read_file.clone();
            async move {
                let limit = limit.unwrap_or(DEFAULT_FILE_BYTES);
                if !(1..=MAX_FILE_BYTES).contains(&limit) {
                    return Err(mlua::Error::runtime(format!(
                        "effects.v1.fs.read limit must be in 1..={MAX_FILE_BYTES}"
                    )));
                }
                let metadata: mlua::Table = stat.call_async(path.clone()).await?;
                let size = metadata.get::<u64>("size")?;
                if size > limit as u64 {
                    return Err(mlua::Error::runtime(format!(
                        "filesystem read exceeds {limit} bytes"
                    )));
                }
                let contents: String = read_file.call_async(path).await?;
                if contents.len() > limit {
                    return Err(mlua::Error::runtime(format!(
                        "filesystem read exceeds {limit} bytes"
                    )));
                }
                Ok(contents)
            }
        })?,
    )?;
    filesystem.set(
        "write",
        lua.create_async_function(move |_, (path, contents): (String, mlua::String)| {
            let write_file = write_file.clone();
            async move {
                if contents.as_bytes().len() > MAX_FILE_BYTES {
                    return Err(mlua::Error::runtime(format!(
                        "filesystem write exceeds {MAX_FILE_BYTES} bytes"
                    )));
                }
                write_file.call_async::<()>((path, contents)).await
            }
        })?,
    )?;
    // Metadata operations a file-backed configuration/resource policy needs:
    // probe, describe, enumerate, create, and remove paths it chose itself.
    // Rust never names a directory, a precedence, or a fallback.
    filesystem.set("default_max_entries", DEFAULT_LIST_ENTRIES)?;
    filesystem.set("max_entries", MAX_LIST_ENTRIES)?;
    filesystem.set("exists", exists)?;
    filesystem.set("stat", stat_entry)?;
    filesystem.set(
        "list",
        lua.create_async_function(move |_, (path, limit): (String, Option<usize>)| {
            let read_dir = read_dir.clone();
            async move {
                let limit = limit.unwrap_or(DEFAULT_LIST_ENTRIES);
                if !(1..=MAX_LIST_ENTRIES).contains(&limit) {
                    return Err(mlua::Error::runtime(format!(
                        "effects.v1.fs.list limit must be in 1..={MAX_LIST_ENTRIES}"
                    )));
                }
                let names: mlua::Table = read_dir.call_async(path).await?;
                if names.raw_len() > limit {
                    return Err(mlua::Error::runtime(format!(
                        "directory listing exceeds {limit} entries"
                    )));
                }
                Ok(names)
            }
        })?,
    )?;
    filesystem.set("make_directory", mkdir)?;
    filesystem.set("remove_file", remove_file)?;

    let process = lua.create_table()?;
    process.set("default_timeout_ms", DEFAULT_PROCESS_TIMEOUT_MS)?;
    process.set("max_timeout_ms", MAX_PROCESS_TIMEOUT_MS)?;
    process.set("default_max_output_bytes", DEFAULT_PROCESS_BYTES)?;
    process.set("max_output_bytes", MAX_PROCESS_BYTES)?;
    process.set(
        "run",
        lua.create_async_function(
            move |lua,
                  (command, arguments, options): (
                String,
                Option<mlua::Table>,
                Option<mlua::Table>,
            )| {
                let exec = exec.clone();
                async move {
                    let bounded = lua.create_table()?;
                    if let Some(options) = options {
                        for pair in options.pairs::<mlua::Value, mlua::Value>() {
                            let (key, value) = pair?;
                            bounded.set(key, value)?;
                        }
                    }
                    let timeout = bounded
                        .get::<Option<u64>>("timeout_ms")?
                        .or(bounded.get::<Option<u64>>("timeout")?)
                        .unwrap_or(DEFAULT_PROCESS_TIMEOUT_MS);
                    if !(1..=MAX_PROCESS_TIMEOUT_MS).contains(&timeout) {
                        return Err(mlua::Error::runtime(format!(
                            "effects.v1.process.run timeout_ms must be in 1..={MAX_PROCESS_TIMEOUT_MS}"
                        )));
                    }
                    let max_output = bounded
                        .get::<Option<usize>>("max_output_bytes")?
                        .unwrap_or(DEFAULT_PROCESS_BYTES);
                    if !(1..=MAX_PROCESS_BYTES).contains(&max_output) {
                        return Err(mlua::Error::runtime(format!(
                            "effects.v1.process.run max_output_bytes must be in 1..={MAX_PROCESS_BYTES}"
                        )));
                    }
                    bounded.set("timeout", timeout)?;
                    bounded.set("timeout_ms", mlua::Value::Nil)?;
                    bounded.set("max_output_bytes", max_output)?;
                    exec.call_async::<mlua::Table>((command, arguments, bounded))
                        .await
                }
            },
        )?,
    )?;

    let timer = lua.create_table()?;
    timer.set("sleep", bridge.get::<mlua::Function>("sleep")?)?;

    let cancellation = lua.create_table()?;
    cancellation.set(
        "new",
        lua.create_function(|lua, ()| {
            crate::ai::signal_userdata(lua, pi_rs_ai::transport::AbortSignal::new())
        })?,
    )?;

    // Pure path arithmetic beside the filesystem effects it addresses: Lua
    // composes its own resource paths instead of asking Rust for a location.
    let bridge_path: mlua::Table = bridge.get("path")?;
    let path = lua.create_table()?;
    for member in [
        "join",
        "normalize",
        "dirname",
        "basename",
        "extname",
        "is_absolute",
        "resolve",
        "relative",
    ] {
        path.set(member, bridge_path.get::<mlua::Function>(member)?)?;
    }
    path.set("separator", bridge_path.get::<String>("sep")?)?;

    // One immutable startup snapshot, read by name. Values never cross in
    // bulk, the table cannot be written, and nothing here knows what any
    // variable means.
    let names: Vec<String> = environment.keys().cloned().collect();
    let env = lua.create_table()?;
    env.set(
        "get",
        lua.create_function(move |_, name: String| Ok(environment.get(&name).cloned()))?,
    )?;
    env.set(
        "names",
        lua.create_function(move |lua, ()| {
            let result = lua.create_table_with_capacity(names.len(), 0)?;
            for name in &names {
                result.push(name.as_str())?;
            }
            Ok(result)
        })?,
    )?;

    let v1 = lua.create_table()?;
    v1.set("api_version", 1_u32)?;
    v1.set("fs", filesystem)?;
    v1.set("path", path)?;
    v1.set("env", env)?;
    v1.set("process", process)?;
    v1.set("timer", timer)?;
    v1.set("cancellation", cancellation)?;
    let effects = lua.create_table()?;
    effects.set("v1", v1)?;
    pi.set("effects", effects)
}
