#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
// Differential for Plan 10 (`@file` argument processing + piped stdin +
// initial-message building). The oracle in
// tests/file-processor-parity/oracle.json is generated from Pi's real
// `processFileArguments` (cli/file-processor.ts) and `buildInitialMessage`
// (cli/initial-message.ts) by scripts/file-processor-oracle. The text path is
// captured byte-for-byte (paths normalized to a `@FIXDIR@` sentinel so the
// checked oracle is portable); this test drives the same fixture files through
// the Rust port and compares.
use pi_rs_app::cli::file_processor::{
    FileProcessorError, build_initial_message, process_file_arguments,
};

const FIX_SENTINEL: &str = "@FIXDIR@";
const ORACLE: &str = include_str!("../../../tests/file-processor-parity/oracle.json");

fn fixdir() -> std::path::PathBuf {
    let here = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    // Normalize the ../.. so the substituted paths match the processor's
    // resolve_read_path output exactly.
    std::fs::canonicalize(here.join("../../tests/file-processor-parity/fixtures")).unwrap()
}

fn substitute(path: &str) -> String {
    // Replace the machine-local fixture dir with the sentinel, matching the
    // generator's normalization. Mirrors `path.split(fixDir).join(FIX_SENTINEL)`.
    path.replace(FIX_SENTINEL, &fixdir().to_string_lossy())
}

#[test]
fn text_file_processing_matches_pi_byte_for_byte() {
    let oracle: serde_json::Value = serde_json::from_str(ORACLE).unwrap();
    let cases = oracle["cases"].as_array().unwrap();
    for case in cases.iter().filter(|c| c["kind"] == "files") {
        let name = case["name"].as_str().unwrap();
        // Reconstruct the real file list by substituting the sentinel back
        // into absolute fixture paths.
        let files: Vec<String> = case["files"]
            .as_array()
            .unwrap()
            .iter()
            .map(|f| substitute(f.as_str().unwrap()))
            .collect();

        // Image cases are generated with autoResizeImages:false (deterministic
        // base64); text cases use the default true. Match the generator so the
        // comparison is byte-for-byte.
        let expect_images = case["images"].as_array().is_some() && !case["images"].as_array().unwrap().is_empty();
        let auto_resize = !expect_images;
        let expected = substitute(case["text"].as_str().unwrap());

        // Build the base_dir as the fixtures parent (so the relative file
        // path resolves identically to the generator's absolute paths). We
        // pass base_dir = fixture dir and absolute file paths, mirroring the
        // generator (which passed absolute paths into processFileArguments).
        let base = fixdir().parent().unwrap().to_path_buf();
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let result = rt
            .block_on(process_file_arguments(&files, &base, auto_resize))
            .unwrap_or_else(|e| panic!("case {name}: processor returned error {e:?}"));

        assert_eq!(
            result.text, expected,
            "case {name}: text mismatch (expected oracle's normalized text)"
        );
        assert_eq!(
            result.images.len(),
            case["imagesLen"].as_u64().unwrap() as usize,
            "case {name}: image count mismatch"
        );
        // For image cases, compare the mime type and base64 payload against
        // Pi's recorded attachment byte-for-byte.
        if let Some(imgs) = case["images"].as_array() {
            for (i, img) in imgs.iter().enumerate() {
                let got = &result.images[i];
                assert_eq!(
                    got.mime_type,
                    img["mimeType"].as_str().unwrap(),
                    "case {name}: image[{i}] mime mismatch"
                );
                assert_eq!(
                    got.data,
                    img["data"].as_str().unwrap(),
                    "case {name}: image[{i}] base64 payload mismatch"
                );
            }
        }
    }
}

#[test]
fn missing_file_yields_not_found_error() {
    let base = fixdir().parent().unwrap().to_path_buf();
    let missing = base.join("does-not-exist.txt");
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let err = rt
        .block_on(process_file_arguments(
            &[missing.to_string_lossy().into_owned()],
            &base,
            true,
        ))
        .unwrap_err();
    match err {
        FileProcessorError::NotFound(p) => assert_eq!(&p, &missing),
        other => panic!("expected NotFound, got {other:?}"),
    }
}

#[test]
fn build_initial_message_matches_pi_oracle() {
    let oracle: serde_json::Value = serde_json::from_str(ORACLE).unwrap();
    let cases = oracle["cases"].as_array().unwrap();
    for case in cases.iter().filter(|c| c["kind"] == "init") {
        let name = case["name"].as_str().unwrap();
        let parsed = &case["input"]["parsed"];
        let mut messages: Vec<String> = parsed["messages"]
            .as_array()
            .map(|a| a.iter().map(|v| v.as_str().unwrap().to_owned()).collect())
            .unwrap_or_default();
        let file_text = case["input"]["fileText"].as_str().unwrap_or_default();
        let stdin = case["input"]["stdinContent"].as_str().map(str::to_owned);
        // Only init cases with no images feed through the images-free path;
        // the oracle's init cases carry no fileImages.
        let out = build_initial_message(&mut messages, file_text, vec![], stdin);
        let expected = &case["result"]["initialMessage"];
        match expected {
            serde_json::Value::Null => {
                assert!(out.initial_message.is_none(), "case {name}: expected none");
            }
            v => {
                assert_eq!(
                    out.initial_message.as_deref(),
                    Some(v.as_str().unwrap()),
                    "case {name}: message mismatch"
                );
            }
        }
    }
}
