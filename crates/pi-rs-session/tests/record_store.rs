#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

use std::fs::OpenOptions;
use std::io::Write;

use pi_rs_session::{CancellationToken, RecordStore, STORE_EXTENSION, StoreError, StoreLimits};
use serde_json::json;

fn token() -> CancellationToken {
    CancellationToken::new()
}

fn values(store: &RecordStore) -> Vec<serde_json::Value> {
    let mut cursor = store.cursor().expect("cursor");
    let mut values = Vec::new();
    loop {
        let window = cursor
            .next_window(2, 64 * 1024, &token())
            .expect("bounded window");
        values.extend(window.records);
        if window.done {
            return values;
        }
    }
}

#[test]
fn durable_append_reopens_only_at_the_explicit_xdg_destination() {
    let temporary = tempfile::tempdir().unwrap();
    let xdg_state = temporary.path().join("xdg-state");
    let destination = xdg_state.join("pi").join("records");
    let path;
    {
        let mut store =
            RecordStore::create(&destination, "arbitrary", StoreLimits::default(), &token())
                .unwrap();
        assert_eq!(
            store
                .append(&json!({"schema": 9, "anything": [1, true]}), &token())
                .unwrap(),
            0
        );
        assert_eq!(store.append(&json!("opaque"), &token()).unwrap(), 1);
        path = store.path().to_path_buf();
        assert!(path.starts_with(&xdg_state));
        assert_eq!(path.extension().unwrap(), STORE_EXTENSION);
    }

    let reopened = RecordStore::open(&path, StoreLimits::default(), &token()).unwrap();
    assert_eq!(
        values(&reopened),
        [json!({"schema": 9, "anything": [1, true]}), json!("opaque")]
    );
    assert!(!temporary.path().join(".pi").exists());
}

#[test]
fn cancelled_append_changes_no_atomic_commit_boundary() {
    let temporary = tempfile::tempdir().unwrap();
    let mut store = RecordStore::create(
        temporary.path(),
        "cancelled",
        StoreLimits::default(),
        &token(),
    )
    .unwrap();
    store.append(&json!({"kept": true}), &token()).unwrap();
    let committed = store.committed_len();
    let cancellation = token();
    cancellation.cancel();
    assert!(matches!(
        store.append(&json!({"ghost": true}), &cancellation),
        Err(StoreError::Cancelled)
    ));
    assert_eq!(store.committed_len(), committed);
    assert_eq!(std::fs::metadata(store.path()).unwrap().len(), committed);
}

#[test]
fn crash_style_partial_tail_is_diagnosed_at_its_exact_boundary() {
    let temporary = tempfile::tempdir().unwrap();
    let path;
    let committed;
    {
        let mut store = RecordStore::create(
            temporary.path(),
            "partial",
            StoreLimits::default(),
            &token(),
        )
        .unwrap();
        store.append(&json!({"complete": true}), &token()).unwrap();
        committed = store.committed_len();
        path = store.path().to_path_buf();
    }
    let mut file = OpenOptions::new().append(true).open(&path).unwrap();
    file.write_all(b"{\"version\":1").unwrap();
    file.sync_all().unwrap();
    drop(file);

    match RecordStore::open(&path, StoreLimits::default(), &token()) {
        Err(StoreError::PartialWrite { offset, bytes, .. }) => {
            assert_eq!(offset, committed);
            assert_eq!(bytes, 12);
        }
        Err(other) => panic!("expected partial-write diagnostic, got {other}"),
        Ok(_) => panic!("expected partial-write diagnostic, store opened"),
    }
}

#[test]
fn checksum_detects_valid_json_corruption_without_interpreting_the_value() {
    let temporary = tempfile::tempdir().unwrap();
    let path;
    {
        let mut store = RecordStore::create(
            temporary.path(),
            "corrupt",
            StoreLimits::default(),
            &token(),
        )
        .unwrap();
        store
            .append(&json!({"payload": "alpha"}), &token())
            .unwrap();
        path = store.path().to_path_buf();
    }
    let bytes = std::fs::read(&path).unwrap();
    let source = String::from_utf8(bytes).unwrap();
    let changed = source.replace("alpha", "omega");
    assert_eq!(source.len(), changed.len());
    std::fs::write(&path, changed).unwrap();

    match RecordStore::open(&path, StoreLimits::default(), &token()) {
        Err(StoreError::CorruptRecord {
            sequence, reason, ..
        }) => {
            assert_eq!(sequence, 0);
            assert!(reason.contains("checksum mismatch"));
        }
        Err(other) => panic!("expected corruption diagnostic, got {other}"),
        Ok(_) => panic!("expected corruption diagnostic, store opened"),
    }
}

#[test]
fn concurrent_open_is_nonblocking_deterministic_and_released_on_drop() {
    let temporary = tempfile::tempdir().unwrap();
    let store =
        RecordStore::create(temporary.path(), "locked", StoreLimits::default(), &token()).unwrap();
    let path = store.path().to_path_buf();
    assert!(matches!(
        RecordStore::open(&path, StoreLimits::default(), &token()),
        Err(StoreError::Locked(locked)) if locked == path
    ));
    drop(store);
    RecordStore::open(&path, StoreLimits::default(), &token()).unwrap();
}

#[test]
fn cursor_windows_are_bounded_and_snapshot_later_appends() {
    let temporary = tempfile::tempdir().unwrap();
    let mut store = RecordStore::create(
        temporary.path(),
        "cursor",
        StoreLimits {
            max_window_records: 2,
            ..StoreLimits::default()
        },
        &token(),
    )
    .unwrap();
    for value in 0..5 {
        store.append(&json!({"value": value}), &token()).unwrap();
    }
    let mut cursor = store.cursor().unwrap();
    store.append(&json!({"value": "later"}), &token()).unwrap();

    let first = cursor.next_window(2, 4096, &token()).unwrap();
    assert_eq!(first.start_sequence, 0);
    assert_eq!(first.next_sequence, 2);
    assert_eq!(first.records.len(), 2);
    assert!(!first.done);
    let second = cursor.next_window(2, 4096, &token()).unwrap();
    let third = cursor.next_window(2, 4096, &token()).unwrap();
    assert_eq!(second.records.len(), 2);
    assert_eq!(third.records, [json!({"value": 4})]);
    assert!(third.done);
    assert!(matches!(
        cursor.next_window(3, 4096, &token()),
        Err(StoreError::WindowLimit { .. })
    ));
}

#[test]
fn prefix_copy_is_atomic_and_contains_exactly_the_requested_records() {
    let temporary = tempfile::tempdir().unwrap();
    let source_directory = temporary.path().join("source");
    let destination = temporary.path().join("destination");
    let mut source = RecordStore::create(
        &source_directory,
        "source",
        StoreLimits::default(),
        &token(),
    )
    .unwrap();
    for value in [json!(null), json!([1, 2]), json!({"third": true})] {
        source.append(&value, &token()).unwrap();
    }
    let copied = source
        .copy_prefix(&destination, "prefix", Some(2), &token())
        .unwrap();
    assert_eq!(copied.record_count(), 2);
    assert_eq!(values(&copied), [json!(null), json!([1, 2])]);
    assert!(destination.join("prefix.jsonl").exists());
    assert!(std::fs::read_dir(&destination).unwrap().all(|entry| {
        !entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .ends_with(".tmp")
    }));
}

#[test]
fn cancelled_copy_publishes_no_destination_or_temporary_file() {
    let temporary = tempfile::tempdir().unwrap();
    let mut source = RecordStore::create(
        temporary.path().join("source"),
        "source",
        StoreLimits::default(),
        &token(),
    )
    .unwrap();
    source.append(&json!({"record": 1}), &token()).unwrap();
    let destination = temporary.path().join("destination");
    let cancellation = token();
    cancellation.cancel();
    assert!(matches!(
        source.copy_prefix(&destination, "copy", None, &cancellation),
        Err(StoreError::Cancelled)
    ));
    assert!(!destination.join("copy.jsonl").exists());
    if destination.exists() {
        assert!(std::fs::read_dir(destination).unwrap().next().is_none());
    }
}

#[test]
fn listing_is_sorted_and_surfaces_locked_and_malformed_candidates() {
    let temporary = tempfile::tempdir().unwrap();
    let directory = temporary.path();
    drop(RecordStore::create(directory, "alpha", StoreLimits::default(), &token()).unwrap());
    let locked = RecordStore::create(directory, "beta", StoreLimits::default(), &token()).unwrap();
    let broken =
        RecordStore::create(directory, "broken", StoreLimits::default(), &token()).unwrap();
    let broken_path = broken.path().to_path_buf();
    drop(broken);
    std::fs::write(broken_path, b"not a header\n").unwrap();

    let listing = RecordStore::list(directory, StoreLimits::default(), &token()).unwrap();
    assert_eq!(listing.stores.len(), 1);
    assert_eq!(listing.stores[0].name, "alpha");
    assert_eq!(
        listing
            .diagnostics
            .iter()
            .map(|diagnostic| (
                diagnostic
                    .path
                    .file_name()
                    .unwrap()
                    .to_string_lossy()
                    .into_owned(),
                diagnostic.kind
            ))
            .collect::<Vec<_>>(),
        [
            ("beta.jsonl".to_owned(), "locked"),
            ("broken.jsonl".to_owned(), "header")
        ]
    );
    drop(locked);
}
