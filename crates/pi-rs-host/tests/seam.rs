//! WS1.1 acceptance: the coroutine async seam.
//! - a handler awaits a host future and returns a value
//! - a busy-loop handler is killed by the watchdog
//! - await time is free: a sleep longer than the budget survives
//! - the exerciser example loads and runs through the public API

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use pi_rs_host::{Host, HostConfig, Outcome};

fn host_with_budget(ms: i64) -> Host {
    Host::new(HostConfig {
        dispatch_timeout_ms: ms,
        ..HostConfig::default()
    })
    .expect("host starts")
}

#[test]
fn handler_awaits_host_future_and_returns() {
    let host = host_with_budget(5000);
    host.load(
        "test://await",
        r#"
            local pi = ...
            pi.on("ping", function(event)
                pi.sleep(10)
                return { echoed = event.value, awaited = true }
            end)
        "#,
    )
    .expect("load");

    let outcomes = host
        .emit("ping", &serde_json::json!({ "value": 42 }))
        .expect("emit");
    assert_eq!(outcomes.len(), 1);
    let result = outcomes[0].result.as_ref().expect("handler ok");
    assert_eq!(
        result.as_ref().expect("has value"),
        &serde_json::json!({ "echoed": 42, "awaited": true })
    );
}

#[test]
fn busy_loop_handler_is_killed() {
    let host = host_with_budget(100);
    host.load(
        "test://hang",
        r#"
            local pi = ...
            pi.on("hang", function() while true do end end)
            pi.on("hang", function() return "survivor" end)
        "#,
    )
    .expect("load");

    let outcomes = host.emit("hang", &serde_json::json!({})).expect("emit");
    assert_eq!(outcomes.len(), 2);
    let err = outcomes[0].result.as_ref().expect_err("watchdog kill");
    assert!(err.contains("watchdog"), "unexpected error: {err}");
    // One hung handler doesn't stop the rest.
    let ok = outcomes[1].result.as_ref().expect("second handler ran");
    assert_eq!(
        ok.as_ref().expect("has value"),
        &serde_json::json!({ "message": "survivor" })
    );
}

#[test]
fn await_time_does_not_burn_watchdog_budget() {
    // Budget 100ms of Lua execution; the handler awaits 300ms of wall
    // clock. If await time counted, the first Lua instruction after the
    // resume would trip the hook.
    let host = host_with_budget(100);
    host.load(
        "test://slow-await",
        r#"
            local pi = ...
            pi.on("slow", function()
                pi.sleep(300)
                return { survived = true }
            end)
        "#,
    )
    .expect("load");

    let outcomes = host.emit("slow", &serde_json::json!({})).expect("emit");
    assert_eq!(outcomes.len(), 1);
    let result = outcomes[0].result.as_ref().expect("await time is free");
    assert_eq!(
        result.as_ref().expect("has value"),
        &serde_json::json!({ "survived": true })
    );
}

#[test]
fn awaits_reset_the_continuous_execution_window() {
    // Budget 150ms of continuous Lua execution. The handler burns ~40ms
    // of Lua per slice, awaiting between slices, for ~400ms of total Lua
    // time. Cumulative metering would kill it; per-window metering (the
    // long-running agent loop shape) must not.
    let host = host_with_budget(150);
    host.load(
        "test://sliced",
        r#"
            local pi = ...
            pi.on("sliced", function()
                for _ = 1, 10 do
                    local deadline = pi.monotonic_ms() + 40
                    while pi.monotonic_ms() < deadline do end
                    pi.sleep(1)
                end
                return { survived = true }
            end)
        "#,
    )
    .expect("load");

    let outcomes = host.emit("sliced", &serde_json::json!({})).expect("emit");
    assert_eq!(outcomes.len(), 1);
    let result = outcomes[0]
        .result
        .as_ref()
        .expect("total Lua time beyond budget survives when sliced by awaits");
    assert_eq!(
        result.as_ref().expect("has value"),
        &serde_json::json!({ "survived": true })
    );
}

#[test]
fn spawned_task_interleaves_with_the_dispatching_handler() {
    // pi.spawn starts a background coroutine on the dispatch's task set:
    // it runs while the handler awaits, and join() returns its value.
    // The interactive frontend runs agent turns this way while its event
    // loop keeps rendering.
    let host = host_with_budget(5000);
    host.load(
        "test://spawn",
        r#"
            local pi = ...
            pi.on("spawned", function()
                local order = {}
                local task = pi.spawn(function()
                    order[#order + 1] = "task-start"
                    pi.sleep(1)
                    order[#order + 1] = "task-end"
                    return "task-value"
                end)
                -- The spawned coroutine has not run yet (it starts at the
                -- next await point), and the handler keeps executing.
                order[#order + 1] = "handler"
                local started = task:done()
                pi.sleep(5)
                order[#order + 1] = "handler-awoke"
                local value = task:join()
                return { order = order, value = value, started = started,
                         done = task:done() }
            end)
        "#,
    )
    .expect("load");

    let outcomes = host.emit("spawned", &serde_json::json!({})).expect("emit");
    assert_eq!(outcomes.len(), 1);
    let result = outcomes[0].result.as_ref().expect("handler ok");
    assert_eq!(
        result.as_ref().expect("has value"),
        &serde_json::json!({
            "order": ["handler", "task-start", "task-end", "handler-awoke"],
            "value": "task-value",
            "started": false,
            "done": true,
        })
    );
}

#[test]
fn handler_error_is_attributed_not_fatal() {
    let host = host_with_budget(5000);
    host.load(
        "test://boom",
        r#"
            local pi = ...
            pi.on("go", function() error("boom") end)
        "#,
    )
    .expect("load");

    let outcomes = host.emit("go", &serde_json::json!({})).expect("emit");
    assert_eq!(outcomes.len(), 1);
    assert_eq!(outcomes[0].source, "test://boom");
    let err = outcomes[0].result.as_ref().expect_err("handler errored");
    assert!(err.contains("boom"), "unexpected error: {err}");
}

#[test]
fn top_level_await_in_chunk_load() {
    let host = host_with_budget(100);
    host.load(
        "test://top-level",
        r#"
            local pi = ...
            pi.sleep(150) -- longer than the budget: await time is free at load too
            pi.on("check", function() return "loaded" end)
        "#,
    )
    .expect("top-level await");
    let outcomes = host.emit("check", &serde_json::json!({})).expect("emit");
    assert!(outcomes[0].result.is_ok());
}

#[test]
fn emit_with_no_handlers_is_empty() {
    let host = host_with_budget(5000);
    let outcomes = host
        .emit("unheard", &serde_json::json!({}))
        .expect("emit succeeds");
    assert!(outcomes.is_empty());
}

#[test]
fn exerciser_example_runs() {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../examples/extensions/await-demo.lua"
    );
    let source = std::fs::read_to_string(path).expect("exerciser example exists");
    let host = host_with_budget(5000);
    host.load("examples/extensions/await-demo.lua", &source)
        .expect("example loads");

    let outcomes = host
        .emit("session_start", &serde_json::json!({}))
        .expect("emit");
    assert_eq!(outcomes.len(), 1);
    let Outcome { source, result } = &outcomes[0];
    assert_eq!(source, "examples/extensions/await-demo.lua");
    assert_eq!(
        result.as_ref().expect("ok").as_ref().expect("value"),
        &serde_json::json!({ "message": "hello from await-demo (awaited 50ms)" })
    );
}
