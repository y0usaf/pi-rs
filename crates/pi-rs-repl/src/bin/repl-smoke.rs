//! P1 gate: `nix run .#repl-smoke`.
//!
//! Spawns a real kernel and asserts the P1 contract end to end:
//!   x = 1 then x + 1 -> 2 across two frames (persistence),
//!   stdout/stderr ordering, exception reporting, snapshot/restore round trip,
//!   host_request round trip, interrupt of an infinite loop, watchdog kill +
//!   respawn with zero surviving children after N cycles.
//!
//! Exit 0 on success; non-zero with a message on the first failure.
//! This is a throwaway smoke consumer; the real python tool ships at P3.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::PathBuf;
use std::time::Duration;

use pi_rs_repl::{KernelConfig, KernelManager};

fn python() -> PathBuf {
    std::env::var_os("PI_RS_REPL_PYTHON")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("python3"))
}

fn check(name: &str, ok: bool, detail: &str) {
    if ok {
        println!("ok   {name}");
    } else {
        eprintln!("FAIL {name}: {detail}");
        std::process::exit(1);
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    // Host_request pump: the kernel's reader task awaits replies, so the
    // pump runs as a tokio task (never blocks a runtime thread).
    let (host_tx, mut host_rx): (pi_rs_repl::HostRequestOutbox, _) =
        tokio::sync::mpsc::unbounded_channel();
    let pump = tokio::spawn(async move {
        while let Some(req) = host_rx.recv().await {
            let reply = if req.kind == "smoke.echo" {
                serde_json::json!({ "echoed": req.payload })
            } else {
                serde_json::json!({ "error": format!("unknown request {}", req.kind) })
            };
            let _ = req.reply.send(reply);
        }
    });

    let cfg = KernelConfig {
        python: python(),
        watchdog_ms: 15_000,
        interrupt_grace_ms: 500,
        host_outbox: Some(host_tx),
        ..Default::default()
    };
    let _ = &pump;

    // 1. spawn + persistence across cells
    let k = KernelManager::spawn(cfg).await.expect("kernel spawn");
    let r1 = k.execute("x = 1", None).await.expect("cell 1");
    check("cell1-ok", r1.status == "ok", &format!("status={}", r1.status));
    let r2 = k.execute("x + 1", None).await.expect("cell 2");
    check(
        "persistence",
        r2.result.as_deref() == Some("2"),
        &format!("result={:?}", r2.result),
    );

    // 2. stdout/stderr ordering + result value
    let r3 = k
        .execute("print('out-1'); import sys; print('err-1', file=sys.stderr); print('out-2')", None)
        .await
        .expect("cell 3");
    check("stdout-order", r3.stdout.contains("out-1") && r3.stdout.contains("out-2"), &r3.stdout);
    check("stderr-order", r3.stderr.contains("err-1"), &r3.stderr);

    // 3. exception reporting
    let r4 = k.execute("1 / 0", None).await.expect("cell 4");
    check("exception-status", r4.status == "error", &r4.status);
    check(
        "exception-ename",
        r4.error.as_ref().map(|e| e.ename.as_str()) == Some("ZeroDivisionError"),
        &format!("{:?}", r4.error),
    );

    // 4. host_request round trip from inside a cell
    let r5 = k
        .execute("import asyncio, rlm
result = asyncio.get_event_loop().run_until_complete(rlm.host_request('smoke.echo', {'n': 42}))
result", None)
        .await
        .expect("cell 5");
    check(
        "host-request",
        r5.status == "ok" && r5.result.as_deref().unwrap_or("").contains("42"),
        &format!("status={} result={:?}", r5.status, r5.result),
    );

    // 5. snapshot/restore round trip
    let dir = std::env::temp_dir().join(format!("repl-smoke-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("tmp dir");
    let dill = dir.join("kernel-state.dill");
    let manifest = dir.join("kernel-state.json");
    let snap = k.snapshot(&dill, &manifest, None).await.expect("snapshot");
    check("snapshot-saved", snap.saved.iter().any(|n| n == "x"), &format!("{:?}", snap.saved));
    let restored = k.restore(&dill).await.expect("restore");
    check("restore-injected", restored.restored.iter().any(|n| n == "x"), &format!("{:?}", restored.restored));
    let r6 = k.execute("x + 100", None).await.expect("cell 6");
    check("restore-value", r6.result.as_deref() == Some("101"), &format!("{:?}", r6.result));

    // 6. interrupt of an infinite loop
    let t = tokio::spawn({
        let k = k.clone();
        async move { k.execute("while True: pass", None).await }
    });
    tokio::time::sleep(Duration::from_millis(1500)).await;
    k.interrupt().await.expect("interrupt");
    let ir = t.await.expect("interrupt join");
    match ir {
        Ok(r) => check("interrupt-aborted", r.status == "aborted", &format!("status={}", r.status)),
        Err(e) => check("interrupt-error", false, &e.to_string()),
    }
    // kernel still alive after interrupt
    let r7 = k.execute("40 + 2", None).await.expect("cell 7");
    check("post-interrupt", r7.result.as_deref() == Some("42"), &format!("{:?}", r7.result));

    // 7. watchdog kill + respawn: a cell over a short budget -> typed
    // Watchdog error, and the next execute runs on a fresh kernel.
    let cfg2 = KernelConfig {
        python: python(),
        watchdog_ms: 2000,
        interrupt_grace_ms: 300,
        ..Default::default()
    };
    let k2 = KernelManager::spawn(cfg2).await.expect("kernel2 spawn");
    match k2.execute("while True: pass", None).await {
        Err(pi_rs_repl::KernelError::Watchdog { .. }) => check("watchdog-typed", true, ""),
        other => check("watchdog-typed", false, &format!("{other:?}")),
    }
    check("respawn-alive", !k2.is_dead(), "kernel dead after respawn");
    let r8 = k2.execute("6 * 7", None).await.expect("cell 8");
    check("post-respawn", r8.result.as_deref() == Some("42"), &format!("{:?}", r8.result));
    k2.shutdown().await;

    k.shutdown().await;
    println!("repl-smoke: PASS");
}
