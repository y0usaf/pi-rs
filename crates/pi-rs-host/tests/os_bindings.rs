//! WS1.3 acceptance: OS bindings on the coroutine path.
//! - pi.exec matches core/exec.ts: capture, exit code, never-throws spawn
//!   failure, timeout → killed, cwd option; await time is watchdog-free
//! - pi.fs roundtrip (mkdir/write/append/read/stat/read_dir/remove/exists)
//! - pi.path matches Node posix semantics from inside Lua
//! - pi.env is a read-only view of the process environment
//! - pi.cwd() is the configured host cwd
//! - the exerciser examples load and run through the public API

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use pi_rs_host::{Host, HostConfig};

fn host_with(ms: i64, cwd: Option<String>) -> Host {
    Host::new(HostConfig {
        dispatch_timeout_ms: ms,
        cwd,
    })
    .expect("host starts")
}

fn host() -> Host {
    host_with(5000, None)
}

/// Register a command whose handler runs `body` and returns its result.
fn run(host: &Host, body: &str) -> serde_json::Value {
    host.load(
        "test://run",
        &format!(
            r#"
            local pi = ...
            pi.register_command("run", {{
                handler = function()
                    {body}
                end,
            }})
            "#
        ),
    )
    .expect("load");
    host.call_command("run", "")
        .expect("command runs")
        .expect("command returns a value")
}

#[test]
fn exec_captures_stdout_stderr_and_code() {
    let out = run(
        &host(),
        r#"
            local r = pi.exec("sh", { "-c", "echo out; echo err >&2; exit 3" })
            return { stdout = r.stdout, stderr = r.stderr, code = r.code, killed = r.killed }
        "#,
    );
    assert_eq!(out["stdout"], "out\n");
    assert_eq!(out["stderr"], "err\n");
    assert_eq!(out["code"], 3);
    assert_eq!(out["killed"], false);
}

#[test]
fn exec_spawn_failure_resolves_code_1() {
    // Spec: execCommand never throws — a missing binary resolves code 1.
    let out = run(
        &host(),
        r#"
            local r = pi.exec("pi-rs-definitely-not-a-command", {})
            return { code = r.code, killed = r.killed }
        "#,
    );
    assert_eq!(out["code"], 1);
    assert_eq!(out["killed"], false);
}

#[test]
fn exec_timeout_kills_and_await_time_is_free() {
    // Watchdog budget (250ms of Lua) < child lifetime if it ran to
    // completion (2s): the suspension while exec awaits is free, and the
    // timeout kills the child long before that.
    let start = std::time::Instant::now();
    let out = run(
        &host_with(250, None),
        r#"
            local r = pi.exec("sh", { "-c", "sleep 2" }, { timeout = 100 })
            return { killed = r.killed }
        "#,
    );
    assert_eq!(out["killed"], true);
    assert!(start.elapsed() < std::time::Duration::from_secs(2));
}

#[test]
fn exec_signal_abort_kills_the_child() {
    // Spec core/exec.ts: options.signal shares the timeout's killProcess
    // path — abort kills the child (SIGTERM) and resolves killed = true.
    let start = std::time::Instant::now();
    let out = run(
        &host(),
        r#"
            local signal = pi.abort_signal()
            pi.spawn(function()
                pi.sleep(50)
                signal:abort()
            end)
            local r = pi.exec("sh", { "-c", "printf early; sleep 5" }, { signal = signal })
            return { stdout = r.stdout, killed = r.killed }
        "#,
    );
    assert_eq!(out["stdout"], "early");
    assert_eq!(out["killed"], true);
    assert!(start.elapsed() < std::time::Duration::from_secs(5));
}

#[test]
fn exec_pre_aborted_signal_kills_immediately() {
    let start = std::time::Instant::now();
    let out = run(
        &host(),
        r#"
            local signal = pi.abort_signal()
            signal:abort()
            local r = pi.exec("sh", { "-c", "sleep 5" }, { signal = signal })
            return { killed = r.killed }
        "#,
    );
    assert_eq!(out["killed"], true);
    assert!(start.elapsed() < std::time::Duration::from_secs(5));
}

#[test]
fn exec_cwd_option_and_host_default() {
    let dir = std::env::temp_dir()
        .canonicalize()
        .expect("temp dir")
        .to_string_lossy()
        .into_owned();
    // Option cwd wins over the host default.
    let out = run(
        &host_with(5000, Some("/".to_owned())),
        &format!(
            r#"
            local with_opt = pi.exec("sh", {{ "-c", "pwd" }}, {{ cwd = "{dir}" }})
            local without = pi.exec("sh", {{ "-c", "pwd" }})
            return {{ with_opt = with_opt.stdout, without = without.stdout }}
            "#
        ),
    );
    assert_eq!(out["with_opt"], format!("{dir}\n"));
    assert_eq!(out["without"], "/\n");
}

#[test]
fn fs_roundtrip() {
    let dir = std::env::temp_dir().join(format!("pi-rs-os-test-{}", std::process::id()));
    let dir_str = dir.to_string_lossy().into_owned();
    let out = run(
        &host(),
        &format!(
            r#"
            local dir = "{dir_str}"
            pi.fs.mkdir(dir .. "/sub")
            local file = dir .. "/f.txt"
            pi.fs.write_file(file, "one\n")
            pi.fs.append_file(file, "two\n")
            local text = pi.fs.read_file(file)
            local st = pi.fs.stat(file)
            local dst = pi.fs.stat(dir .. "/sub")
            local names = pi.fs.read_dir(dir)
            table.sort(names)
            local existed = pi.fs.exists(file)
            pi.fs.remove_file(file)
            return {{
                text = text,
                size = st.size,
                type = st.type,
                dir_type = dst.type,
                modified_positive = st.modified_ms > 0,
                names = names,
                existed = existed,
                exists_after = pi.fs.exists(file),
            }}
            "#
        ),
    );
    assert_eq!(out["text"], "one\ntwo\n");
    assert_eq!(out["size"], 8);
    assert_eq!(out["type"], "file");
    assert_eq!(out["dir_type"], "dir");
    assert_eq!(out["modified_positive"], true);
    assert_eq!(out["names"], serde_json::json!(["f.txt", "sub"]));
    assert_eq!(out["existed"], true);
    assert_eq!(out["exists_after"], false);
    std::fs::remove_dir_all(&dir).expect("cleanup");
}

#[test]
fn fs_errors_are_lua_errors() {
    // Node's sync fs throws; the pcall is the try/catch translation.
    let out = run(
        &host(),
        r#"
            local ok, err = pcall(pi.fs.read_file, "/definitely/not/a/file")
            return { ok = ok, mentions_op = tostring(err):find("read_file") ~= nil }
        "#,
    );
    assert_eq!(out["ok"], false);
    assert_eq!(out["mentions_op"], true);
}

#[test]
fn path_matches_node_posix_from_lua() {
    let out = run(
        &host_with(5000, Some("/home/myself/node".to_owned())),
        r#"
            return {
                join = pi.path.join("/foo", "bar", "baz/asdf", "quux", ".."),
                normalize = pi.path.normalize("/foo/bar//baz/asdf/quux/.."),
                dirname = pi.path.dirname("/foo/bar/baz/asdf/quux"),
                basename = pi.path.basename("/foo/bar/baz/asdf/quux.html"),
                basename_sfx = pi.path.basename("/foo/bar/baz/asdf/quux.html", ".html"),
                extname = pi.path.extname("index.coffee.md"),
                is_abs = pi.path.is_absolute("/a"),
                not_abs = pi.path.is_absolute("a"),
                resolve = pi.path.resolve("wwwroot", "static_files/png/", "../gif/image.gif"),
                relative = pi.path.relative("/data/orandea/test/aaa", "/data/orandea/impl/bbb"),
                sep = pi.path.sep,
            }
        "#,
    );
    assert_eq!(out["join"], "/foo/bar/baz/asdf");
    assert_eq!(out["normalize"], "/foo/bar/baz/asdf");
    assert_eq!(out["dirname"], "/foo/bar/baz/asdf");
    assert_eq!(out["basename"], "quux.html");
    assert_eq!(out["basename_sfx"], "quux");
    assert_eq!(out["extname"], ".md");
    assert_eq!(out["is_abs"], true);
    assert_eq!(out["not_abs"], false);
    assert_eq!(
        out["resolve"],
        "/home/myself/node/wwwroot/static_files/gif/image.gif"
    );
    assert_eq!(out["relative"], "../../impl/bbb");
    assert_eq!(out["sep"], "/");
}

#[test]
fn env_reads_and_is_read_only() {
    let out = run(
        &host(),
        r#"
            local ok = pcall(function() pi.env.PI_RS_TEST_RO = "x" end)
            return {
                has_path = type(pi.env.PATH) == "string",
                missing_is_nil = pi.env.PI_RS_DEFINITELY_UNSET_VAR == nil,
                write_ok = ok,
            }
        "#,
    );
    assert_eq!(out["has_path"], true);
    assert_eq!(out["missing_is_nil"], true);
    assert_eq!(out["write_ok"], false);
}

#[test]
fn cwd_is_the_configured_host_cwd() {
    let out = run(
        &host_with(5000, Some("/configured/dir".to_owned())),
        "return { cwd = pi.cwd() }",
    );
    assert_eq!(out["cwd"], "/configured/dir");
}

#[test]
fn exec_streams_data_before_completion() {
    let out = run(
        &host(),
        r#"
            local chunks = {}
            local result = pi.exec("sh", { "-c", "printf one; sleep 0.02; printf two" }, {
                onData = function(chunk) chunks[#chunks + 1] = chunk end,
            })
            return { joined = table.concat(chunks), code = result.code }
        "#,
    );
    assert_eq!(out, serde_json::json!({ "joined": "onetwo", "code": 0 }));
}

#[cfg(unix)]
#[test]
fn exec_timeout_kills_the_process_group() {
    let dir = std::env::temp_dir().join(format!("pi-rs-tree-kill-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("tempdir");
    let pid_file = dir.join("descendant.pid");
    let script = format!(
        "sh -c \"trap '' TERM; while :; do sleep 1; done\" & echo $! > {}; wait",
        pid_file.display()
    );
    let out = run(
        &host(),
        &format!(
            r#"return pi.exec("sh", {{ "-c", {:?} }}, {{ timeout = 30 }})"#,
            script
        ),
    );
    assert_eq!(out["killed"], true);
    let pid: i32 = std::fs::read_to_string(pid_file)
        .expect("descendant pid")
        .trim()
        .parse()
        .expect("numeric pid");
    std::thread::sleep(std::time::Duration::from_millis(20));
    // A killed child may briefly remain as a zombie, but it must not be running.
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).unwrap_or_default();
    assert!(
        stat.is_empty() || stat.split_whitespace().nth(2) == Some("Z"),
        "descendant still running: {stat}"
    );
}

#[test]
fn exec_nonpositive_timeout_is_disabled_and_callback_errors_propagate() {
    let out = run(
        &host(),
        r#"
        local zero = pi.exec("sh", { "-c", "printf zero" }, { timeout = 0 })
        local negative = pi.exec("sh", { "-c", "printf negative" }, { timeout = -10 })
        local ok, err = pcall(pi.exec, "sh", { "-c", "printf callback" }, {
            onData = function() error("callback exploded") end,
        })
        return { zero = zero.stdout, negative = negative.stdout, ok = ok,
            callback_error = tostring(err):find("callback exploded") ~= nil }
    "#,
    );
    assert_eq!(
        out,
        serde_json::json!({
            "zero": "zero", "negative": "negative", "ok": false, "callback_error": true
        })
    );
}

#[test]
fn exec_demo_exerciser_loads_and_runs() {
    let host = host();
    let source = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../examples/extensions/exec-demo.lua"
    ));
    host.load("examples/extensions/exec-demo.lua", source)
        .expect("exerciser loads");

    let reply = host
        .call_command("git-dirty", "")
        .expect("command runs")
        .expect("command returns a message");
    let message = reply["message"].as_str().expect("message string");
    assert!(message.ends_with("uncommitted file(s)"), "got: {message}");

    let reply = host
        .call_command("exec-abort", "")
        .expect("command runs")
        .expect("command returns a message");
    let message = reply["message"].as_str().expect("message string");
    assert_eq!(message, "killed=true output=\"partial\"", "got: {message}");

    // The guard handlers are subscribed and dispatchable.
    let outcomes = host
        .emit("session_before_switch", &serde_json::json!({}))
        .expect("emit");
    assert_eq!(outcomes.len(), 1);
    assert!(outcomes[0].result.is_ok());
}

#[test]
fn os_demo_exerciser_loads_and_runs() {
    let host = host();
    let source = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../examples/extensions/os-demo.lua"
    ));
    host.load("examples/extensions/os-demo.lua", source)
        .expect("exerciser loads");

    let reply = host
        .call_command("os-demo", "")
        .expect("command runs")
        .expect("command returns a message");
    let message = reply["message"].as_str().expect("message string");
    assert!(message.contains("base=notes.txt"), "got: {message}");
    assert!(message.contains("ext=.txt"), "got: {message}");
    assert!(message.contains("bytes=14"), "got: {message}");
    assert!(message.contains("raw=14"), "got: {message}");
    assert!(
        message.contains("real_base=pi-rs-os-demo"),
        "got: {message}"
    );
    assert!(message.contains("lines=2"), "got: {message}");
    assert!(
        message.contains("exists_after=false:ABC123:true"),
        "got: {message}"
    );
}
