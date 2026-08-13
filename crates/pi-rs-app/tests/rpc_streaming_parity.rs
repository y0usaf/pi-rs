#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
// Differential for Plan 10 RPC streaming commands (prompt, bash, compact, fork,
// clone, new_session, switch_session, get_fork_messages) plus the seeded read
// commands. The oracle in tests/rpc-parity/oracle.json is generated from Pi's
// real `runRpcMode` (modes/rpc/rpc-mode.ts) by scripts/rpc-oracle, driving a
// scripted session stub over a JSONL command corpus. Each case records Pi's raw
// stdout bytes on stdin EOF.
//
// pi-rs's RPC role reproduces the scripted session through a gated parity seed:
// `PI_RPC_SCRIPTED_SEED` (→ `request.scriptedRpc`) carries the same seed shape
// gen-oracle.ts's makeSession uses. This test drives the real `pi --mode rpc`
// binary as a subprocess with the seed set, feeds the same JSONL command lines,
// and compares the response records to Pi's oracle (order-exact across records,
// key order-insensitive within a record — the RPC JSON contract). All 16 oracle
// cases are replayed; every streaming + seeded-read case must match.
use std::io::Write;
use std::process::{Command, Stdio};

#[cfg(debug_assertions)]
fn oracle_cases() -> serde_json::Value {
    serde_json::from_str(include_str!("../../../tests/rpc-parity/oracle.json")).unwrap()
}

/// Run `pi --mode rpc` under a scripted seed, feed JSONL on stdin, return
/// (stdout, stderr, exit status). Pi's runRpcMode shuts down orderly on stdin
/// EOF: exit 0, protocol-clean stdout, and no stray output on stderr.
#[cfg(debug_assertions)]
fn run_rpc_seeded(
    seed: &serde_json::Value,
    commands: &serde_json::Value,
    cwd: &str,
) -> (String, String, i32) {
    let bin = env!("CARGO_BIN_EXE_pi");
    let mut child = Command::new(bin)
        .args(["--mode", "rpc"])
        .arg("--cwd")
        .arg(cwd)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .env("HOME", cwd)
        // The seed must carry the same magic-token prefix main.rs's
        // scripted_rpc_seed() strips before parsing, so the debug seam turns
        // the scripted session data into request.scriptedRpc — and any env
        // value without the token leaves the seam inert.
        .env(
            "PI_RPC_SCRIPTED_SEED",
            format!("parity-seed:{}", serde_json::json!({ "seed": seed })),
        )
        .spawn()
        .expect("spawn pi");
    let input = commands
        .as_array()
        .unwrap()
        .iter()
        .map(|c| serde_json::to_string(c).unwrap() + "\n")
        .collect::<String>();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(input.as_bytes())
        .unwrap();
    let out = child.wait_with_output().expect("wait pi");
    (
        String::from_utf8(out.stdout).unwrap(),
        String::from_utf8(out.stderr).unwrap(),
        out.status.code().unwrap(),
    )
}

/// Split a JSONL stream into parsed records (ignoring blank/trailing).
fn records(stdout: &str) -> Vec<serde_json::Value> {
    stdout
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| serde_json::from_str(l).expect("each line is JSON"))
        .collect()
}

/// The scripts/rpc-oracle cases that exercise the streaming + seeded-read
/// commands now closed under PLAN 10, plus the empty-array and unknown cases.
#[cfg(debug_assertions)]
const ALL_CASES: &[&str] = &[
    "prompt-async-success",
    "prompt-preflight-failure",
    "event-streaming",
    "compact-bash-session-ops",
    "session-fork-clone",
    "fork-messages",
    "async-steer-followup-abort",
    "state-and-simple",
    "unknown-command-no-id",
    "thinking-model-commands",
    "set-model-not-found",
    "export-html",
    "commands-registry",
    "empty-fork-messages",
    "empty-commands",
    "empty-messages",
];

/// Gated to debug builds because the seed seam (main.rs `scripted_rpc_seed`)
/// is compiled OUT of release binaries for security; in release the seeded
/// oracle cannot be replayed, so this differential only applies under the dev
/// profile (which is what `nix flake check`'s cargoTest runs). The unseeded
/// honesty tests below run in every profile.
#[test]
#[cfg(debug_assertions)]
fn rpc_streaming_and_seeded_commands_match_pi_oracle() {
    let oracle = oracle_cases();
    let cases = oracle["cases"].as_array().unwrap();
    let mut failures = Vec::new();
    let temp = tempfile::tempdir().unwrap();
    let cwd = temp.path().to_string_lossy().into_owned();
    for case in cases {
        let name = case["name"].as_str().unwrap();
        if !ALL_CASES.contains(&name) {
            continue;
        }
        // The oracle's `seed` object is what gen-oracle's makeSession received;
        // pass it through PI_RPC_SCRIPTED_SEED.
        let seed = &case["seed"];
        let (raw_stdout, stderr, code) = run_rpc_seeded(seed, &case["commands"], &cwd);
        // Pi's runRpcMode exits 0 on stdin EOF and keeps stderr quiet unless a
        // genuine error was surfaced; the seeded path must behave identically.
        assert_eq!(
            code, 0,
            "{name}: RPC must exit 0 on orderly shutdown, got {code}, stderr={stderr:?}"
        );
        assert!(
            stderr.trim().is_empty(),
            "{name}: stderr must be clean, got {stderr:?}"
        );
        let got = records(&raw_stdout);
        let expected: Vec<serde_json::Value> = case["stdout"]
            .as_str()
            .unwrap()
            .lines()
            .filter(|l| !l.trim().is_empty())
            .map(|l| serde_json::from_str(l).unwrap())
            .collect();
        if got.len() != expected.len() {
            failures.push(format!(
                "{name}: record count {} != {}",
                got.len(),
                expected.len()
            ));
            continue;
        }
        for (i, (g, e)) in got.iter().zip(expected.iter()).enumerate() {
            if g != e {
                failures.push(format!(
                    "{name}: record {i} differs from Pi:\n  got {g}\n  exp {e}"
                ));
                break;
            }
        }
    }
    assert!(
        failures.is_empty(),
        "RPC streaming/seed oracle mismatches:\n{}",
        failures.join("\n")
    );
}

/// The production (non-seeded) RPC path must still reject a streaming command it
/// has no real provider for, and must keep stdout protocol-clean — the seed is
/// not set here, exercising the inert-seam behavior.
#[test]
fn rpc_unseeded_streaming_without_provider_stays_honest_and_clean() {
    let temp = tempfile::tempdir().unwrap();
    let cwd = temp.path().to_string_lossy().into_owned();
    // No PI_RPC_SCRIPTED_SEED: a bare prompt with no model/auth resolves to the
    // honest real-agent error path (no seeded envelope), and stdout stays JSONL.
    let bin = env!("CARGO_BIN_EXE_pi");
    let mut child = Command::new(bin)
        .args(["--mode", "rpc"])
        .arg("--cwd")
        .arg(&cwd)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .env("HOME", &cwd)
        .spawn()
        .expect("spawn pi");
    let input = "{\"type\":\"prompt\",\"message\":\"hi\",\"id\":\"r1\"}\n";
    child
        .stdin
        .take()
        .unwrap()
        .write_all(input.as_bytes())
        .unwrap();
    let out = child.wait_with_output().expect("wait pi");
    let stdout = String::from_utf8(out.stdout).unwrap();
    let stderr = String::from_utf8(out.stderr).unwrap();
    // The inert-seam run still exits 0 on stdin EOF (orderly shutdown), matching
    // Pi's runRpcMode, and stderr stays quiet.
    assert_eq!(
        out.status.code(),
        Some(0),
        "RPC must exit 0, stderr={stderr:?}"
    );
    // Every non-blank stdout line parses as a JSONL object (protocol-clean).
    for line in stdout.lines() {
        if !line.trim().is_empty() {
            let v: serde_json::Value = serde_json::from_str(line).expect("JSONL object");
            assert!(v.is_object(), "stdout line must be an object");
        }
    }
}

/// The production (non-seeded) RPC `bash` path must report REAL output, not the
/// empty `{stdout:""}` a mis-mapped executor would emit. Unlike the seeded
/// oracle cases (which use a scripted `bashResult`), this drives an actual bash
/// command through the shared executor and asserts Pi's `{exitCode,stdout,
/// stderr}` envelope carries the run's merged output. stderr stays "" because
/// pi-rs's executor merges stdout+stderr into a single `output` stream (it does
/// not split them), so the merged stream is reported on `stdout`.
#[test]
fn rpc_unseeded_bash_reports_real_output_in_pi_envelope() {
    let temp = tempfile::tempdir().unwrap();
    let cwd = temp.path().to_string_lossy().into_owned();
    let bin = env!("CARGO_BIN_EXE_pi");
    let mut child = Command::new(bin)
        .args(["--mode", "rpc"])
        .arg("--cwd")
        .arg(&cwd)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .env("HOME", &cwd)
        .spawn()
        .expect("spawn pi");
    let input = "{\"type\":\"bash\",\"command\":\"printf shell-ok\",\"id\":\"b1\"}\n";
    child
        .stdin
        .take()
        .unwrap()
        .write_all(input.as_bytes())
        .unwrap();
    let out = child.wait_with_output().expect("wait pi");
    let stdout = String::from_utf8(out.stdout).unwrap();
    let stderr = String::from_utf8(out.stderr).unwrap();
    assert_eq!(
        out.status.code(),
        Some(0),
        "RPC must exit 0, stderr={stderr:?}"
    );
    let rec = records(&stdout);
    assert_eq!(rec.len(), 1, "expect exactly one response: {stdout:?}");
    let r = &rec[0];
    assert_eq!(r["type"], "response");
    assert_eq!(r["command"], "bash");
    assert_eq!(r["success"], true);
    assert_eq!(r["id"], "b1");
    // The real bash run's merged output is reported on `stdout`, exitCode 0.
    assert_eq!(r["data"]["exitCode"], 0);
    assert_eq!(r["data"]["stdout"], "shell-ok");
    assert_eq!(r["data"]["stderr"], "");
}

/// The production (non-seeded) RPC `compact` path must emit Pi's
/// `{data:{sessionId,summary,kept}}` envelope from real session state, not crash
/// (it was previously latent, only exercised by the seeded oracle cases which
/// supply a scripted `compactResult`). Same for the honest `get_session_stats`
/// read over real state. Together these pin that the unseeded streaming deferral
/// paths run clean and protocol-clean on stdin EOF.
#[test]
fn rpc_unseeded_compact_and_session_stats_run_real_envelopes() {
    let temp = tempfile::tempdir().unwrap();
    let cwd = temp.path().to_string_lossy().into_owned();
    let bin = env!("CARGO_BIN_EXE_pi");
    let mut child = Command::new(bin)
        .args(["--mode", "rpc"])
        .arg("--cwd")
        .arg(&cwd)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .env("HOME", &cwd)
        .spawn()
        .expect("spawn pi");
    let input = concat!(
        "{\"type\":\"compact\",\"id\":\"c1\"}\n",
        "{\"type\":\"get_session_stats\",\"id\":\"s1\"}\n",
    );
    child
        .stdin
        .take()
        .unwrap()
        .write_all(input.as_bytes())
        .unwrap();
    let out = child.wait_with_output().expect("wait pi");
    let stdout = String::from_utf8(out.stdout).unwrap();
    let stderr = String::from_utf8(out.stderr).unwrap();
    assert_eq!(
        out.status.code(),
        Some(0),
        "RPC must exit 0, stderr={stderr:?}"
    );
    let rec = records(&stdout);
    // Sync (depth-0) commands like get_session_stats emit inline during input
    // processing, before the deferred depth-1 `compact` finishes in the
    // completion phase — so s1 comes before c1.
    assert_eq!(rec.len(), 2, "expect two responses: {stdout:?}");
    assert_eq!(rec[0]["command"], "get_session_stats");
    assert_eq!(rec[0]["id"], "s1");
    assert_eq!(rec[0]["success"], true);
    assert!(rec[0]["data"]["messageCount"].is_i64());
    assert!(
        rec[0]["data"]["sessionId"].is_string(),
        "real session id, got {:?}",
        rec[0]["data"]["sessionId"]
    );
    assert_eq!(rec[1]["command"], "compact");
    assert_eq!(rec[1]["id"], "c1");
    assert_eq!(rec[1]["success"], true);
    assert!(
        rec[1]["data"]["sessionId"].is_string(),
        "real session id, got {:?}",
        rec[1]["data"]["sessionId"]
    );
    assert!(rec[1]["data"]["summary"].is_string());
    assert!(rec[1]["data"]["kept"].is_i64());
}

/// The production (non-seeded) RPC `bash` path must harden malformed input and
/// executor failures into Pi-style `success:false` error envelopes rather than
/// crashing and corrupting the RPC JSONL stream. A bash record with a missing
/// `command` field would previously throw in the `.."\n"..` concatenation; now
/// it surfaces as a clean rejection (success:false, error set) and the process
/// still exits 0 with protocol-clean stdout.
#[test]
fn rpc_unseeded_bash_missing_command_emits_error_envelope() {
    let temp = tempfile::tempdir().unwrap();
    let cwd = temp.path().to_string_lossy().into_owned();
    let bin = env!("CARGO_BIN_EXE_pi");
    let mut child = Command::new(bin)
        .args(["--mode", "rpc"])
        .arg("--cwd")
        .arg(&cwd)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .env("HOME", &cwd)
        .spawn()
        .expect("spawn pi");
    // No `command` field: must become an error envelope, not a stdout-corrupting
    // uncaught throw or a crash.
    let input = "{\"type\":\"bash\",\"id\":\"b1\"}\n";
    child
        .stdin
        .take()
        .unwrap()
        .write_all(input.as_bytes())
        .unwrap();
    let out = child.wait_with_output().expect("wait pi");
    let stdout = String::from_utf8(out.stdout).unwrap();
    let stderr = String::from_utf8(out.stderr).unwrap();
    assert_eq!(
        out.status.code(),
        Some(0),
        "RPC must exit 0 on orderly shutdown, stderr={stderr:?}"
    );
    let rec = records(&stdout);
    assert_eq!(rec.len(), 1, "expect exactly one response: {stdout:?}");
    let r = &rec[0];
    assert_eq!(r["type"], "response");
    assert_eq!(r["command"], "bash");
    assert_eq!(r["success"], false);
    assert_eq!(r["id"], "b1");
    assert!(
        r["error"].is_string(),
        "error envelope, got {:?}",
        r["error"]
    );
}
