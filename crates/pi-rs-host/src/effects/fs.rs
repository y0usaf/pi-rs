use std::io::Write as _;
use std::time::UNIX_EPOCH;

use super::{EffectError, FsRequest};

#[derive(Debug, Clone)]
pub enum FsResult {
    Bytes(Vec<u8>),
    Bool(bool),
    Names(Vec<String>),
    Stat(FileStat),
    Path(String),
    Unit,
}

#[derive(Debug, Clone)]
pub struct FileStat {
    pub kind: &'static str,
    pub size: u64,
    pub modified_ms: i64,
}

fn io_error(operation: &str, path: &str, error: std::io::Error) -> EffectError {
    EffectError::Io(format!("{operation} '{path}': {error}"))
}

pub async fn execute(request: FsRequest) -> Result<FsResult, EffectError> {
    match request {
        FsRequest::Read { path, .. } => tokio::fs::read(&path)
            .await
            .map(FsResult::Bytes)
            .map_err(|error| io_error("read_file", &path, error)),
        FsRequest::Write { path, contents } => tokio::fs::write(&path, contents)
            .await
            .map(|()| FsResult::Unit)
            .map_err(|error| io_error("write_file", &path, error)),
        FsRequest::Append { path, contents } => tokio::task::spawn_blocking({
            let display = path.clone();
            move || {
                let mut file = std::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(&path)
                    .map_err(|error| io_error("append_file", &display, error))?;
                file.write_all(&contents)
                    .map_err(|error| io_error("append_file", &display, error))?;
                Ok(FsResult::Unit)
            }
        })
        .await
        .map_err(|error| EffectError::Io(format!("append_file task failed: {error}")))?,
        FsRequest::Exists { path } => Ok(FsResult::Bool(
            tokio::fs::try_exists(path).await.unwrap_or(false),
        )),
        FsRequest::ReadDir { path } => {
            let mut directory = tokio::fs::read_dir(&path)
                .await
                .map_err(|error| io_error("read_dir", &path, error))?;
            let mut names = Vec::new();
            while let Some(entry) = directory
                .next_entry()
                .await
                .map_err(|error| io_error("read_dir", &path, error))?
            {
                names.push(entry.file_name().to_string_lossy().into_owned());
            }
            Ok(FsResult::Names(names))
        }
        FsRequest::Stat { path } => {
            let metadata = tokio::fs::metadata(&path)
                .await
                .map_err(|error| io_error("stat", &path, error))?;
            let kind = if metadata.is_dir() {
                "dir"
            } else if metadata.is_file() {
                "file"
            } else {
                "other"
            };
            let modified_ms = metadata
                .modified()
                .ok()
                .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
                .map(|duration| i64::try_from(duration.as_millis()).unwrap_or(i64::MAX))
                .unwrap_or(0);
            Ok(FsResult::Stat(FileStat {
                kind,
                size: metadata.len(),
                modified_ms,
            }))
        }
        FsRequest::Mkdir { path } => tokio::fs::create_dir_all(&path)
            .await
            .map(|()| FsResult::Unit)
            .map_err(|error| io_error("mkdir", &path, error)),
        FsRequest::Realpath { path } => tokio::fs::canonicalize(&path)
            .await
            .map(|path| FsResult::Path(path.to_string_lossy().into_owned()))
            .map_err(|error| io_error("realpath", &path, error)),
        FsRequest::RemoveFile { path } => tokio::fs::remove_file(&path)
            .await
            .map(|()| FsResult::Unit)
            .map_err(|error| io_error("remove_file", &path, error)),
        FsRequest::CreateTempFile { prefix, contents } => tokio::task::spawn_blocking(move || {
            let safe_prefix: String = prefix
                .chars()
                .map(|character| {
                    if character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.') {
                        character
                    } else {
                        '-'
                    }
                })
                .collect();
            let mut file = tempfile::Builder::new()
                .prefix(&safe_prefix)
                .suffix(".log")
                .tempfile()
                .map_err(|error| io_error("create_temp_file", &safe_prefix, error))?;
            file.write_all(&contents)
                .map_err(|error| io_error("create_temp_file", &safe_prefix, error))?;
            let (_file, path) = file
                .keep()
                .map_err(|error| io_error("create_temp_file", &safe_prefix, error.error))?;
            Ok(FsResult::Path(path.to_string_lossy().into_owned()))
        })
        .await
        .map_err(|error| EffectError::Io(format!("create_temp_file task failed: {error}")))?,
    }
}
