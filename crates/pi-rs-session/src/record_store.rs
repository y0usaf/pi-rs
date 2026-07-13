//! Versioned append-only storage for arbitrary JSON values.
//!
//! The format deliberately carries only framing metadata. Record values are
//! opaque to this crate: no field, role, relationship, or product concept is
//! interpreted. Callers supply every destination directory explicitly.

use std::fs::{File, OpenOptions};
use std::io::{self, BufRead, BufReader, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use serde_json::Value;
use thiserror::Error;

use crate::uuid::random_uuid;

/// On-disk format version. A version mismatch is rejected rather than guessed.
pub const FORMAT_VERSION: u32 = 1;
/// File suffix used by [`RecordStore::list`].
pub const STORE_EXTENSION: &str = "jsonl";
const FORMAT_NAME: &str = "pi-rs-records";
const HEADER: &[u8] = b"{\"format\":\"pi-rs-records\",\"version\":1}\n";
const FRAME_OVERHEAD_LIMIT: usize = 1_024;

/// Resource bounds applied while encoding and reading a store.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct StoreLimits {
    /// Largest serialized JSON value accepted as one record.
    pub max_record_bytes: usize,
    /// Largest record count accepted in one cursor window.
    pub max_window_records: usize,
    /// Largest encoded byte count accepted in one cursor window.
    pub max_window_bytes: usize,
}

impl Default for StoreLimits {
    fn default() -> Self {
        Self {
            max_record_bytes: 1024 * 1024,
            max_window_records: 256,
            max_window_bytes: 4 * 1024 * 1024,
        }
    }
}

impl StoreLimits {
    fn validate(self) -> Result<Self, StoreError> {
        if self.max_record_bytes == 0 || self.max_window_records == 0 || self.max_window_bytes == 0
        {
            return Err(StoreError::InvalidLimits);
        }
        Ok(self)
    }

    fn max_frame_bytes(self) -> usize {
        self.max_record_bytes.saturating_add(FRAME_OVERHEAD_LIMIT)
    }
}

/// Cooperative cancellation shared across threads and store operations.
#[derive(Clone, Debug, Default)]
pub struct CancellationToken(Arc<AtomicBool>);

impl CancellationToken {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn cancel(&self) {
        self.0.store(true, Ordering::Release);
    }

    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }

    fn check(&self) -> Result<(), StoreError> {
        if self.is_cancelled() {
            Err(StoreError::Cancelled)
        } else {
            Ok(())
        }
    }
}

/// Durable-store failures include the exact path and record boundary involved.
#[derive(Debug, Error)]
pub enum StoreError {
    #[error("record-store operation cancelled")]
    Cancelled,
    #[error("store limits must all be greater than zero")]
    InvalidLimits,
    #[error("invalid store name {0:?}; use ASCII letters, digits, '.', '-', or '_'")]
    InvalidName(String),
    #[error("record store already exists: {0}")]
    AlreadyExists(PathBuf),
    #[error("record store is locked by another open handle: {0}")]
    Locked(PathBuf),
    #[error("record-store {operation} failed for {path}: {source}")]
    Io {
        operation: &'static str,
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("invalid record-store header at byte {offset} in {path}: {reason}")]
    CorruptHeader {
        path: PathBuf,
        offset: u64,
        reason: String,
    },
    #[error(
        "unsupported record-store format version {found} in {path}; supported version is {supported}"
    )]
    UnsupportedVersion {
        path: PathBuf,
        found: u32,
        supported: u32,
    },
    #[error("partial record write at byte {offset} in {path} ({bytes} unterminated bytes)")]
    PartialWrite {
        path: PathBuf,
        offset: u64,
        bytes: usize,
    },
    #[error("corrupt record {sequence} at byte {offset} in {path}: {reason}")]
    CorruptRecord {
        path: PathBuf,
        sequence: u64,
        offset: u64,
        reason: String,
    },
    #[error("record is {bytes} bytes, above the configured {limit}-byte bound")]
    RecordTooLarge { bytes: usize, limit: usize },
    #[error(
        "cursor window ({records} records, {bytes} bytes) exceeds configured bounds ({record_limit} records, {byte_limit} bytes)"
    )]
    WindowLimit {
        records: usize,
        bytes: usize,
        record_limit: usize,
        byte_limit: usize,
    },
    #[error("cursor window needs at least {required} bytes for its next record")]
    WindowTooSmall { required: usize },
    #[error("copy requested {requested} records but source contains {available}")]
    CopyOutOfRange { requested: u64, available: u64 },
    #[error("record store changed outside its locked handle: {0}")]
    ConcurrentMutation(PathBuf),
}

/// Valid metadata returned from directory listing.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StoreInfo {
    pub name: String,
    pub path: PathBuf,
    pub format_version: u32,
    pub record_count: u64,
    pub bytes: u64,
}

/// One malformed, partial, or locked candidate found during listing.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StoreDiagnostic {
    pub path: PathBuf,
    pub kind: &'static str,
    pub message: String,
}

/// Listing never hides a bad candidate behind the valid rows.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct StoreListing {
    pub stores: Vec<StoreInfo>,
    pub diagnostics: Vec<StoreDiagnostic>,
}

/// One bounded cursor result. `done` refers to the file snapshot taken when
/// the cursor was created, not records appended afterward.
#[derive(Clone, Debug, PartialEq)]
pub struct CursorWindow {
    pub records: Vec<Value>,
    pub start_sequence: u64,
    pub next_sequence: u64,
    pub encoded_bytes: usize,
    pub done: bool,
}

/// Exclusive writer/open handle. The sidecar lock is held until drop.
pub struct RecordStore {
    path: PathBuf,
    file: File,
    _lock: File,
    limits: StoreLimits,
    header_end: u64,
    record_count: u64,
    committed_len: u64,
}

/// Streaming read cursor over a fixed durable-length snapshot.
pub struct RecordCursor {
    path: PathBuf,
    reader: BufReader<File>,
    end_offset: u64,
    next_sequence: u64,
    limits: StoreLimits,
    pending: Option<(Value, usize)>,
}

struct Scan {
    header_end: u64,
    record_count: u64,
    bytes: u64,
}

impl RecordStore {
    /// Atomically creates an empty store in an explicit caller-owned directory.
    pub fn create(
        directory: impl AsRef<Path>,
        name: &str,
        limits: StoreLimits,
        cancellation: &CancellationToken,
    ) -> Result<Self, StoreError> {
        let directory = directory.as_ref();
        let limits = limits.validate()?;
        validate_name(name)?;
        cancellation.check()?;
        std::fs::create_dir_all(directory)
            .map_err(|source| io_error("create directory", directory, source))?;
        let path = store_path(directory, name);
        let lock = acquire_lock(&path, LockMode::Exclusive, true)?;
        if path.exists() {
            return Err(StoreError::AlreadyExists(path));
        }

        let temporary = temporary_path(directory, name);
        let creation = (|| {
            let mut file = create_new_file(&temporary)?;
            file.write_all(HEADER)
                .map_err(|source| io_error("write header", &temporary, source))?;
            file.sync_all()
                .map_err(|source| io_error("sync new store", &temporary, source))?;
            cancellation.check()?;
            std::fs::rename(&temporary, &path)
                .map_err(|source| io_error("publish new store", &path, source))?;
            sync_directory(directory)?;
            open_data_file(&path)
        })();
        if creation.is_err() {
            let _ = std::fs::remove_file(&temporary);
        }
        let file = creation?;
        Ok(Self {
            path,
            file,
            _lock: lock,
            limits,
            header_end: HEADER.len() as u64,
            record_count: 0,
            committed_len: HEADER.len() as u64,
        })
    }

    /// Opens and fully validates a store while taking its deterministic
    /// non-blocking exclusive lock.
    pub fn open(
        path: impl AsRef<Path>,
        limits: StoreLimits,
        cancellation: &CancellationToken,
    ) -> Result<Self, StoreError> {
        let path = path.as_ref().to_path_buf();
        let limits = limits.validate()?;
        cancellation.check()?;
        let lock = acquire_lock(&path, LockMode::Exclusive, true)?;
        let file = open_data_file(&path)?;
        let scan = scan_file(&file, &path, limits, cancellation)?;
        Ok(Self {
            path,
            file,
            _lock: lock,
            limits,
            header_end: scan.header_end,
            record_count: scan.record_count,
            committed_len: scan.bytes,
        })
    }

    /// Lists `*.jsonl` stores in deterministic path order. Invalid and locked
    /// files are returned as diagnostics rather than silently omitted.
    pub fn list(
        directory: impl AsRef<Path>,
        limits: StoreLimits,
        cancellation: &CancellationToken,
    ) -> Result<StoreListing, StoreError> {
        let directory = directory.as_ref();
        let limits = limits.validate()?;
        cancellation.check()?;
        let entries = std::fs::read_dir(directory)
            .map_err(|source| io_error("list directory", directory, source))?;
        let mut paths = Vec::new();
        for entry in entries {
            cancellation.check()?;
            let entry =
                entry.map_err(|source| io_error("read directory entry", directory, source))?;
            let path = entry.path();
            if path
                .extension()
                .is_some_and(|extension| extension == STORE_EXTENSION)
                && entry
                    .file_type()
                    .map_err(|source| io_error("inspect directory entry", &path, source))?
                    .is_file()
            {
                paths.push(path);
            }
        }
        paths.sort();

        let mut listing = StoreListing::default();
        for path in paths {
            cancellation.check()?;
            let inspected = (|| {
                let _lock = acquire_lock(&path, LockMode::Shared, false)?;
                let file = File::open(&path)
                    .map_err(|source| io_error("open for listing", &path, source))?;
                scan_file(&file, &path, limits, cancellation)
            })();
            match inspected {
                Ok(scan) => listing.stores.push(StoreInfo {
                    name: path
                        .file_stem()
                        .and_then(|name| name.to_str())
                        .unwrap_or_default()
                        .to_owned(),
                    path,
                    format_version: FORMAT_VERSION,
                    record_count: scan.record_count,
                    bytes: scan.bytes,
                }),
                Err(error) => listing.diagnostics.push(StoreDiagnostic {
                    path,
                    kind: diagnostic_kind(&error),
                    message: error.to_string(),
                }),
            }
        }
        Ok(listing)
    }

    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    #[must_use]
    pub fn record_count(&self) -> u64 {
        self.record_count
    }

    #[must_use]
    pub fn committed_len(&self) -> u64 {
        self.committed_len
    }

    /// Appends one opaque value. Cancellation is checked before the commit
    /// starts. Once bytes are written, the method always completes `sync_data`
    /// (or reports an I/O failure), avoiding an ambiguous cancelled commit.
    pub fn append(
        &mut self,
        value: &Value,
        cancellation: &CancellationToken,
    ) -> Result<u64, StoreError> {
        let encoded_value =
            serde_json::to_vec(value).map_err(|source| StoreError::CorruptRecord {
                path: self.path.clone(),
                sequence: self.record_count,
                offset: self.committed_len,
                reason: source.to_string(),
            })?;
        if encoded_value.len() > self.limits.max_record_bytes {
            return Err(StoreError::RecordTooLarge {
                bytes: encoded_value.len(),
                limit: self.limits.max_record_bytes,
            });
        }
        let sequence = self.record_count;
        let frame = encode_frame(sequence, &encoded_value);

        let actual_len = self
            .file
            .metadata()
            .map_err(|source| io_error("inspect before append", &self.path, source))?
            .len();
        if actual_len != self.committed_len {
            return Err(StoreError::ConcurrentMutation(self.path.clone()));
        }
        cancellation.check()?;
        if let Err(source) = self.file.write_all(&frame) {
            self.rollback_append();
            return Err(io_error("append record", &self.path, source));
        }
        if let Err(source) = self.file.sync_data() {
            self.rollback_append();
            return Err(io_error("sync appended record", &self.path, source));
        }
        self.committed_len = self.committed_len.saturating_add(frame.len() as u64);
        self.record_count = self.record_count.saturating_add(1);
        Ok(sequence)
    }

    /// Creates a bounded streaming cursor over the store's current durable
    /// length. Later appends are intentionally outside this cursor snapshot.
    pub fn cursor(&self) -> Result<RecordCursor, StoreError> {
        let file = File::open(&self.path)
            .map_err(|source| io_error("open cursor file", &self.path, source))?;
        let mut reader = BufReader::new(file);
        reader
            .seek(SeekFrom::Start(self.header_end))
            .map_err(|source| io_error("seek cursor", &self.path, source))?;
        Ok(RecordCursor {
            path: self.path.clone(),
            reader,
            end_offset: self.committed_len,
            next_sequence: 0,
            limits: self.limits,
            pending: None,
        })
    }

    /// Atomically copies the first `record_count` records to a new explicit
    /// destination. `None` copies the full snapshot. The destination is not
    /// published until its complete file and directory entry are synced.
    pub fn copy_prefix(
        &self,
        directory: impl AsRef<Path>,
        name: &str,
        record_count: Option<u64>,
        cancellation: &CancellationToken,
    ) -> Result<Self, StoreError> {
        let directory = directory.as_ref();
        validate_name(name)?;
        cancellation.check()?;
        let count = record_count.unwrap_or(self.record_count);
        if count > self.record_count {
            return Err(StoreError::CopyOutOfRange {
                requested: count,
                available: self.record_count,
            });
        }
        std::fs::create_dir_all(directory)
            .map_err(|source| io_error("create copy directory", directory, source))?;
        let path = store_path(directory, name);
        let lock = acquire_lock(&path, LockMode::Exclusive, true)?;
        if path.exists() {
            return Err(StoreError::AlreadyExists(path));
        }
        let temporary = temporary_path(directory, name);

        let copied = (|| {
            let source = File::open(&self.path)
                .map_err(|error| io_error("open copy source", &self.path, error))?;
            let mut reader = BufReader::new(source);
            reader
                .seek(SeekFrom::Start(self.header_end))
                .map_err(|error| io_error("seek copy source", &self.path, error))?;
            let mut destination = create_new_file(&temporary)?;
            destination
                .write_all(HEADER)
                .map_err(|error| io_error("write copy header", &temporary, error))?;
            for sequence in 0..count {
                cancellation.check()?;
                let (value, _) = read_frame(
                    &mut reader,
                    &self.path,
                    sequence,
                    self.committed_len,
                    self.limits,
                )?;
                let encoded_value =
                    serde_json::to_vec(&value).map_err(|error| StoreError::CorruptRecord {
                        path: self.path.clone(),
                        sequence,
                        offset: 0,
                        reason: error.to_string(),
                    })?;
                let frame = encode_frame(sequence, &encoded_value);
                destination
                    .write_all(&frame)
                    .map_err(|error| io_error("write copy record", &temporary, error))?;
            }
            destination
                .sync_all()
                .map_err(|error| io_error("sync copied store", &temporary, error))?;
            cancellation.check()?;
            std::fs::rename(&temporary, &path)
                .map_err(|error| io_error("publish copied store", &path, error))?;
            sync_directory(directory)?;
            open_data_file(&path)
        })();
        if copied.is_err() {
            let _ = std::fs::remove_file(&temporary);
        }
        let file = copied?;
        let committed_len = file
            .metadata()
            .map_err(|source| io_error("inspect copied store", &path, source))?
            .len();
        Ok(Self {
            path,
            file,
            _lock: lock,
            limits: self.limits,
            header_end: HEADER.len() as u64,
            record_count: count,
            committed_len,
        })
    }

    fn rollback_append(&mut self) {
        let _ = self.file.set_len(self.committed_len);
        let _ = self.file.sync_data();
    }
}

impl RecordCursor {
    #[must_use]
    pub fn next_sequence(&self) -> u64 {
        self.next_sequence
    }

    /// Reads at most the requested and configured record/byte bounds.
    pub fn next_window(
        &mut self,
        max_records: usize,
        max_bytes: usize,
        cancellation: &CancellationToken,
    ) -> Result<CursorWindow, StoreError> {
        if max_records == 0
            || max_bytes == 0
            || max_records > self.limits.max_window_records
            || max_bytes > self.limits.max_window_bytes
        {
            return Err(StoreError::WindowLimit {
                records: max_records,
                bytes: max_bytes,
                record_limit: self.limits.max_window_records,
                byte_limit: self.limits.max_window_bytes,
            });
        }
        cancellation.check()?;
        let start_sequence = self.next_sequence;
        let mut records = Vec::new();
        let mut encoded_bytes = 0usize;

        while records.len() < max_records {
            cancellation.check()?;
            let next = if let Some(pending) = self.pending.take() {
                Some(pending)
            } else if self
                .reader
                .stream_position()
                .map_err(|source| io_error("read cursor position", &self.path, source))?
                >= self.end_offset
            {
                None
            } else {
                Some(read_frame(
                    &mut self.reader,
                    &self.path,
                    self.next_sequence,
                    self.end_offset,
                    self.limits,
                )?)
            };
            let Some((value, bytes)) = next else {
                break;
            };
            if bytes > max_bytes && records.is_empty() {
                self.pending = Some((value, bytes));
                return Err(StoreError::WindowTooSmall { required: bytes });
            }
            if encoded_bytes.saturating_add(bytes) > max_bytes {
                self.pending = Some((value, bytes));
                break;
            }
            encoded_bytes += bytes;
            records.push(value);
            self.next_sequence = self.next_sequence.saturating_add(1);
        }

        let done = self.pending.is_none()
            && self
                .reader
                .stream_position()
                .map_err(|source| io_error("read cursor position", &self.path, source))?
                >= self.end_offset;
        Ok(CursorWindow {
            records,
            start_sequence,
            next_sequence: self.next_sequence,
            encoded_bytes,
            done,
        })
    }
}

fn validate_name(name: &str) -> Result<(), StoreError> {
    let valid = !name.is_empty()
        && name != "."
        && name != ".."
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_'));
    if valid {
        Ok(())
    } else {
        Err(StoreError::InvalidName(name.to_owned()))
    }
}

fn store_path(directory: &Path, name: &str) -> PathBuf {
    directory.join(format!("{name}.{STORE_EXTENSION}"))
}

fn temporary_path(directory: &Path, name: &str) -> PathBuf {
    directory.join(format!(".{name}.{}.tmp", random_uuid()))
}

fn lock_path(path: &Path) -> PathBuf {
    let mut value = path.as_os_str().to_os_string();
    value.push(".lock");
    PathBuf::from(value)
}

fn create_new_file(path: &Path) -> Result<File, StoreError> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options
        .open(path)
        .map_err(|source| io_error("create temporary store", path, source))
}

fn open_data_file(path: &Path) -> Result<File, StoreError> {
    OpenOptions::new()
        .read(true)
        .append(true)
        .open(path)
        .map_err(|source| io_error("open store", path, source))
}

#[derive(Clone, Copy)]
enum LockMode {
    Shared,
    Exclusive,
}

fn acquire_lock(path: &Path, mode: LockMode, create: bool) -> Result<File, StoreError> {
    let lock_path = lock_path(path);
    let mut options = OpenOptions::new();
    options.read(true).write(true).create(create);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let lock = options
        .open(&lock_path)
        .map_err(|source| io_error("open lock", &lock_path, source))?;
    let acquired = match mode {
        LockMode::Shared => lock.try_lock_shared(),
        LockMode::Exclusive => lock.try_lock(),
    };
    if let Err(source) = acquired {
        let source: io::Error = source.into();
        if source.kind() == io::ErrorKind::WouldBlock {
            return Err(StoreError::Locked(path.to_path_buf()));
        }
        return Err(io_error("acquire lock", &lock_path, source));
    }
    Ok(lock)
}

fn sync_directory(path: &Path) -> Result<(), StoreError> {
    let directory = File::open(path).map_err(|source| io_error("open directory", path, source))?;
    directory
        .sync_all()
        .map_err(|source| io_error("sync directory", path, source))
}

fn scan_file(
    file: &File,
    path: &Path,
    limits: StoreLimits,
    cancellation: &CancellationToken,
) -> Result<Scan, StoreError> {
    let bytes = file
        .metadata()
        .map_err(|source| io_error("inspect store", path, source))?
        .len();
    let reader_file =
        File::open(path).map_err(|source| io_error("open store for validation", path, source))?;
    let mut reader = BufReader::new(reader_file);
    let header_end = read_header(&mut reader, path, bytes)?;
    let mut record_count = 0u64;
    loop {
        cancellation.check()?;
        let offset = reader
            .stream_position()
            .map_err(|source| io_error("read validation position", path, source))?;
        if offset >= bytes {
            break;
        }
        let _ = read_frame(&mut reader, path, record_count, bytes, limits)?;
        record_count = record_count.saturating_add(1);
    }
    Ok(Scan {
        header_end,
        record_count,
        bytes,
    })
}

fn read_header(
    reader: &mut BufReader<File>,
    path: &Path,
    file_len: u64,
) -> Result<u64, StoreError> {
    let offset = reader
        .stream_position()
        .map_err(|source| io_error("read header position", path, source))?;
    if offset >= file_len {
        return Err(StoreError::CorruptHeader {
            path: path.to_path_buf(),
            offset,
            reason: "missing header".to_owned(),
        });
    }
    let line =
        read_limited_line(reader, 256).map_err(|source| io_error("read header", path, source))?;
    if line.last() != Some(&b'\n') {
        return Err(StoreError::PartialWrite {
            path: path.to_path_buf(),
            offset,
            bytes: line.len(),
        });
    }
    let header: Value = serde_json::from_slice(&line[..line.len() - 1]).map_err(|source| {
        StoreError::CorruptHeader {
            path: path.to_path_buf(),
            offset,
            reason: source.to_string(),
        }
    })?;
    let format = header
        .get("format")
        .and_then(Value::as_str)
        .ok_or_else(|| StoreError::CorruptHeader {
            path: path.to_path_buf(),
            offset,
            reason: "missing string format".to_owned(),
        })?;
    if format != FORMAT_NAME {
        return Err(StoreError::CorruptHeader {
            path: path.to_path_buf(),
            offset,
            reason: format!("unexpected format {format:?}"),
        });
    }
    let version = header
        .get("version")
        .and_then(Value::as_u64)
        .and_then(|value| u32::try_from(value).ok())
        .ok_or_else(|| StoreError::CorruptHeader {
            path: path.to_path_buf(),
            offset,
            reason: "missing integer version".to_owned(),
        })?;
    if version != FORMAT_VERSION {
        return Err(StoreError::UnsupportedVersion {
            path: path.to_path_buf(),
            found: version,
            supported: FORMAT_VERSION,
        });
    }
    reader
        .stream_position()
        .map_err(|source| io_error("read header position", path, source))
}

fn read_frame(
    reader: &mut BufReader<File>,
    path: &Path,
    expected_sequence: u64,
    end_offset: u64,
    limits: StoreLimits,
) -> Result<(Value, usize), StoreError> {
    let offset = reader
        .stream_position()
        .map_err(|source| io_error("read record position", path, source))?;
    let line = read_limited_line(reader, limits.max_frame_bytes())
        .map_err(|source| io_error("read record", path, source))?;
    if line.last() != Some(&b'\n') {
        if line.len() > limits.max_frame_bytes() {
            return Err(StoreError::CorruptRecord {
                path: path.to_path_buf(),
                sequence: expected_sequence,
                offset,
                reason: format!("frame exceeds {} bytes", limits.max_frame_bytes()),
            });
        }
        return Err(StoreError::PartialWrite {
            path: path.to_path_buf(),
            offset,
            bytes: line.len(),
        });
    }
    let position = reader
        .stream_position()
        .map_err(|source| io_error("read record position", path, source))?;
    if position > end_offset {
        return Err(StoreError::PartialWrite {
            path: path.to_path_buf(),
            offset,
            bytes: line.len(),
        });
    }
    let frame: Value = serde_json::from_slice(&line[..line.len() - 1]).map_err(|source| {
        StoreError::CorruptRecord {
            path: path.to_path_buf(),
            sequence: expected_sequence,
            offset,
            reason: source.to_string(),
        }
    })?;
    let field_error = |field: &str| StoreError::CorruptRecord {
        path: path.to_path_buf(),
        sequence: expected_sequence,
        offset,
        reason: format!("missing or invalid {field}"),
    };
    let version = frame
        .get("version")
        .and_then(Value::as_u64)
        .and_then(|value| u32::try_from(value).ok())
        .ok_or_else(|| field_error("version"))?;
    if version != FORMAT_VERSION {
        return Err(StoreError::CorruptRecord {
            path: path.to_path_buf(),
            sequence: expected_sequence,
            offset,
            reason: format!("record version {version} is unsupported"),
        });
    }
    let sequence = frame
        .get("sequence")
        .and_then(Value::as_u64)
        .ok_or_else(|| field_error("sequence"))?;
    if sequence != expected_sequence {
        return Err(StoreError::CorruptRecord {
            path: path.to_path_buf(),
            sequence: expected_sequence,
            offset,
            reason: format!("found sequence {sequence}"),
        });
    }
    let stored_checksum = frame
        .get("checksum")
        .and_then(Value::as_str)
        .ok_or_else(|| field_error("checksum"))?;
    let value = frame
        .get("value")
        .cloned()
        .ok_or_else(|| field_error("value"))?;
    let encoded = serde_json::to_vec(&value).map_err(|source| StoreError::CorruptRecord {
        path: path.to_path_buf(),
        sequence: expected_sequence,
        offset,
        reason: source.to_string(),
    })?;
    if encoded.len() > limits.max_record_bytes {
        return Err(StoreError::RecordTooLarge {
            bytes: encoded.len(),
            limit: limits.max_record_bytes,
        });
    }
    if stored_checksum != checksum(&encoded) {
        return Err(StoreError::CorruptRecord {
            path: path.to_path_buf(),
            sequence: expected_sequence,
            offset,
            reason: "checksum mismatch".to_owned(),
        });
    }
    Ok((value, line.len()))
}

fn read_limited_line(reader: &mut BufReader<File>, max_bytes: usize) -> io::Result<Vec<u8>> {
    let mut line = Vec::new();
    reader
        .take(max_bytes.saturating_add(1) as u64)
        .read_until(b'\n', &mut line)?;
    Ok(line)
}

fn encode_frame(sequence: u64, encoded_value: &[u8]) -> Vec<u8> {
    let mut frame = format!(
        "{{\"version\":{FORMAT_VERSION},\"sequence\":{sequence},\"checksum\":\"{}\",\"value\":",
        checksum(encoded_value)
    )
    .into_bytes();
    frame.extend_from_slice(encoded_value);
    frame.extend_from_slice(b"}\n");
    frame
}

fn checksum(encoded: &[u8]) -> String {
    // CRC-64/ECMA-182 framing checksum. It is for accidental corruption and
    // torn-write detection, not authentication.
    let mut checksum = 0u64;
    for byte in encoded {
        checksum ^= u64::from(*byte) << 56;
        for _ in 0..8 {
            checksum = if checksum & (1 << 63) == 0 {
                checksum << 1
            } else {
                (checksum << 1) ^ 0x42f0_e1eb_a9ea_3693
            };
        }
    }
    format!("{checksum:016x}")
}

fn diagnostic_kind(error: &StoreError) -> &'static str {
    match error {
        StoreError::Locked(_) => "locked",
        StoreError::PartialWrite { .. } => "partial-write",
        StoreError::CorruptHeader { .. } | StoreError::UnsupportedVersion { .. } => "header",
        StoreError::CorruptRecord { .. } | StoreError::RecordTooLarge { .. } => "corruption",
        StoreError::Cancelled => "cancelled",
        StoreError::Io { .. } => "io",
        _ => "invalid",
    }
}

fn io_error(operation: &'static str, path: impl AsRef<Path>, source: io::Error) -> StoreError {
    StoreError::Io {
        operation,
        path: path.as_ref().to_path_buf(),
        source,
    }
}
