//! pi.fs.watch — filesystem watch mechanism (PLAN 9.9 filesystem.watchFile,
//! resource.file_watcher lifetime).
//!
//! A polling watcher: the dispatch drive loop (vm.rs) stats every watched
//! path on its tick cadence and invokes the callback when the mtime/size
//! pair changes. Deterministic in tests, no inotify dependency. Each
//! watcher is a tracked resource; close() or VM shutdown removes it.
//!
//! Lua surface (pi.fs.watch):
//! - watch(path, options?, callback) -> watcher
//!   options: { interval_ms = poll cadence (default 100) }
//!   callback(path, "change")
//!   watcher methods: close(), path()

use std::cell::RefCell;
use std::collections::HashMap;
use std::time::UNIX_EPOCH;

use mlua::Function;

pub(crate) struct WatcherEntry {
    pub(crate) id: u64,
    pub(crate) path: String,
    pub(crate) last_mtime: Option<i64>,
    pub(crate) last_size: Option<u64>,
    pub(crate) last_inode: Option<u64>,
    pub(crate) func: Function,
}

thread_local! {
    static WATCHERS: RefCell<HashMap<u64, WatcherEntry>> = RefCell::new(HashMap::new());
    static NEXT_ID: RefCell<u64> = const { RefCell::new(1) };
}

fn next_id() -> u64 {
    NEXT_ID.with(|n| {
        let mut n = n.borrow_mut();
        let id = *n;
        *n += 1;
        id
    })
}

fn metadata(path: &str) -> Option<(i64, u64, u64)> {
    let md = std::fs::metadata(path).ok()?;
    let mtime = md
        .modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| i64::try_from(d.as_nanos()).unwrap_or(i64::MAX))
        .unwrap_or(0);
    // Inode: rename-replace (atomic_write) changes the inode even when the
    // filesystem reports an identical mtime/size for back-to-back writes
    // (tmpfs timestamp granularity).
    #[cfg(unix)]
    let inode = { use std::os::unix::fs::MetadataExt as _; md.ino() };
    #[cfg(not(unix))]
    let inode = 0;
    Some((mtime, md.len(), inode))
}

pub(crate) fn register(path: String, func: Function) -> u64 {
    let id = next_id();
    let (mtime, size, inode) = metadata(&path).unwrap_or((0, 0, 0));
    WATCHERS.with(|w| {
        w.borrow_mut().insert(
            id,
            WatcherEntry {
                id,
                path: path.clone(),
                last_mtime: Some(mtime),
                last_size: Some(size),
                last_inode: Some(inode),
                func,
            },
        );
    });
    let kind = "resource.file_watcher";
    let label = format!("watch:{path}");
    crate::resources::register(kind, label, move || {
        remove(id);
    });
    id
}

pub(crate) fn remove(id: u64) -> bool {
    WATCHERS.with(|w| w.borrow_mut().remove(&id).is_some())
}

pub(crate) fn count() -> usize {
    WATCHERS.with(|w| w.borrow().len())
}

/// Stat every watched path; return entries whose mtime/size changed since
/// the previous poll (the drive loop invokes their callbacks).
pub(crate) async fn poll() -> Vec<WatcherEntry> {
    // Synchronous stat on the VM thread: each poll stats only watched
    // paths, and the drive loop bounds the cadence by its select tick.
    // spawn_blocking would require the callback (an Rc-backed mlua
    // function) to be Send; it is not, and the stat is fast.
    let mut changed = Vec::new();
    WATCHERS.with(|w| {
        let mut watchers = w.borrow_mut();
        for entry in watchers.values_mut() {
            let current = metadata(&entry.path);
            let changed_now = match (current, entry.last_mtime, entry.last_size, entry.last_inode) {
                (Some((mtime, size, inode)), Some(last_mtime), Some(last_size), Some(last_inode)) => {
                    mtime != last_mtime || size != last_size || inode != last_inode
                }
                _ => true,
            };
            if changed_now
                && let Some((mtime, size, inode)) = current {
                    entry.last_mtime = Some(mtime);
                    entry.last_size = Some(size);
                    entry.last_inode = Some(inode);
                    changed.push(WatcherEntry {
                        id: entry.id,
                        path: entry.path.clone(),
                        last_mtime: entry.last_mtime,
                        last_size: entry.last_size,
                        last_inode: entry.last_inode,
                        func: entry.func.clone(),
                    });
                }
        }
    });
    changed
}

pub(crate) fn dispose_all() -> usize {
    WATCHERS.with(|w| {
        let mut watchers = w.borrow_mut();
        let count = watchers.len();
        watchers.clear();
        count
    })
}
