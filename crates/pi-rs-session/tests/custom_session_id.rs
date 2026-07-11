//! Port of `test/session-manager/custom-session-id.test.ts`.
#![allow(clippy::unwrap_used, clippy::panic)]

mod common;

use std::path::Path;

use common::{is_session_file_name, is_uuid_v7};
use pi_rs_session::{NewSessionOptions, SessionError, SessionManager};
use serde_json::json;

fn with_id(id: &str) -> Option<NewSessionOptions> {
    Some(NewSessionOptions {
        id: Some(id.to_owned()),
        parent_session: None,
    })
}

#[test]
fn uses_the_provided_id() {
    let mut session = SessionManager::in_memory();
    session.new_session(with_id("my-custom-id")).unwrap();
    assert_eq!(session.get_session_id(), "my-custom-id");
}

#[test]
fn allows_alphanumeric_ids_with_interior_punctuation() {
    let mut session = SessionManager::in_memory();
    session.new_session(with_id("abc-123_def.456")).unwrap();
    assert_eq!(session.get_session_id(), "abc-123_def.456");
}

#[test]
fn rejects_invalid_custom_session_ids() {
    let invalid = [
        "", "-abc", "abc-", "_abc", "abc_", ".abc", "abc.", "abc/def", "abc\\def", "abc def",
    ];
    for id in invalid {
        let mut session = SessionManager::in_memory();
        let err = session.new_session(with_id(id)).unwrap_err();
        assert!(matches!(err, SessionError::InvalidSessionId), "id {id:?}");
        assert!(
            err.to_string()
                .starts_with("Session id must be non-empty, contain only alphanumeric characters")
        );
    }
}

#[test]
fn generates_uuid_v7_when_no_id_provided() {
    let mut session = SessionManager::in_memory();
    session.new_session(None).unwrap();
    let id = session.get_session_id().to_owned();
    assert!(!id.is_empty());
    assert!(is_uuid_v7(&id), "{id}");
}

#[test]
fn generates_uuid_v7_when_options_without_id() {
    let mut session = SessionManager::in_memory();
    session
        .new_session(Some(NewSessionOptions {
            id: None,
            parent_session: Some("parent.jsonl".to_owned()),
        }))
        .unwrap();
    let id = session.get_session_id().to_owned();
    assert!(is_uuid_v7(&id), "{id}");
}

#[test]
fn includes_custom_id_in_session_header() {
    let mut session = SessionManager::in_memory();
    session.new_session(with_id("header-test-id")).unwrap();
    assert_eq!(session.get_header().unwrap()["id"], "header-test-id");
}

#[test]
fn generates_uuid_v7_when_constructed_without_explicit_id() {
    let session = SessionManager::in_memory();
    assert!(is_uuid_v7(session.get_session_id()));
    assert_eq!(
        session.get_header().unwrap()["id"],
        session.get_session_id()
    );
}

#[test]
fn uses_provided_id_when_creating_persisted_session() {
    let temp = tempfile::tempdir().unwrap();
    let dir = temp.path().to_string_lossy().into_owned();
    let session =
        SessionManager::create(&dir, Some(&dir), "", with_id("created-session-id")).unwrap();

    assert_eq!(session.get_session_id(), "created-session-id");
    assert_eq!(session.get_header().unwrap()["id"], "created-session-id");
    let session_file = session.get_session_file().unwrap();
    assert!(session_file.contains("created-session-id"));
    let basename = Path::new(session_file)
        .file_name()
        .unwrap()
        .to_string_lossy();
    assert!(
        is_session_file_name(&basename, "created-session-id"),
        "{basename}"
    );
    assert!(!Path::new(session_file).exists());
}

#[test]
fn generates_uuid_v7_when_creating_branched_session() {
    let mut session = SessionManager::in_memory();
    let first_id = session
        .append_message(json!({
            "role": "user",
            "content": [{ "type": "text", "text": "hello" }],
            "timestamp": pi_rs_session::time::now_ms(),
        }))
        .unwrap();

    session.create_branched_session(&first_id).unwrap();

    assert!(is_uuid_v7(session.get_session_id()));
    assert_eq!(
        session.get_header().unwrap()["id"],
        session.get_session_id()
    );
}

#[test]
fn generates_uuid_v7_when_forking_from_session_file() {
    let temp = tempfile::tempdir().unwrap();
    let dir = temp.path().to_string_lossy().into_owned();
    let source_path = temp.path().join("source.jsonl");
    std::fs::write(
        &source_path,
        format!(
            "{}\n{}\n",
            json!({
                "type": "session", "version": 3, "id": "legacy-session-id",
                "timestamp": pi_rs_session::time::now_iso(), "cwd": dir,
            }),
            json!({
                "type": "message", "id": "entry-1", "parentId": null,
                "timestamp": pi_rs_session::time::now_iso(),
                "message": {
                    "role": "assistant",
                    "content": [{ "type": "text", "text": "hello" }],
                    "api": "openai-responses", "provider": "openai", "model": "gpt-5.4",
                    "usage": {
                        "input": 0, "output": 0, "cacheRead": 0, "cacheWrite": 0,
                        "totalTokens": 0,
                        "cost": { "input": 0, "output": 0, "cacheRead": 0, "cacheWrite": 0, "total": 0 },
                    },
                    "stopReason": "stop",
                    "timestamp": pi_rs_session::time::now_ms(),
                },
            }),
        ),
    )
    .unwrap();

    let forked =
        SessionManager::fork_from(&source_path.to_string_lossy(), &dir, Some(&dir), "", None)
            .unwrap();
    let header = forked.get_header().unwrap();
    assert!(is_uuid_v7(header["id"].as_str().unwrap()));
    assert_eq!(
        header["parentSession"],
        source_path.to_string_lossy().as_ref()
    );
}

#[test]
fn uses_provided_id_when_forking_from_session_file() {
    let temp = tempfile::tempdir().unwrap();
    let dir = temp.path().to_string_lossy().into_owned();
    let source_path = temp.path().join("source.jsonl");
    std::fs::write(
        &source_path,
        format!(
            "{}\n",
            json!({
                "type": "session", "version": 3, "id": "source-session-id",
                "timestamp": pi_rs_session::time::now_iso(), "cwd": dir,
            }),
        ),
    )
    .unwrap();

    let forked = SessionManager::fork_from(
        &source_path.to_string_lossy(),
        &dir,
        Some(&dir),
        "",
        Some(NewSessionOptions {
            id: Some("forked-session-id".to_owned()),
            parent_session: None,
        }),
    )
    .unwrap();
    let header = forked.get_header().unwrap();
    assert_eq!(header["id"], "forked-session-id");
    assert_eq!(
        header["parentSession"],
        source_path.to_string_lossy().as_ref()
    );
    let session_file = forked.get_session_file().unwrap().to_owned();
    assert!(session_file.contains("forked-session-id"));
    let basename = Path::new(&session_file)
        .file_name()
        .unwrap()
        .to_string_lossy();
    assert!(
        is_session_file_name(&basename, "forked-session-id"),
        "{basename}"
    );
}

#[test]
fn fork_source_must_be_valid() {
    let temp = tempfile::tempdir().unwrap();
    let dir = temp.path().to_string_lossy().into_owned();
    let source_path = temp.path().join("empty.jsonl");
    std::fs::write(&source_path, "").unwrap();

    let err =
        match SessionManager::fork_from(&source_path.to_string_lossy(), &dir, Some(&dir), "", None)
        {
            Err(err) => err,
            Ok(_) => panic!("fork from empty source must fail"),
        };
    assert!(
        err.to_string()
            .starts_with("Cannot fork: source session file is empty or invalid:")
    );
}
