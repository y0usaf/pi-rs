#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

use std::process::Command;

#[test]
fn explicit_file_application_runs_the_public_spine_and_exits_cleanly() {
    let scratch = tempfile::tempdir().unwrap();
    let effect_path = scratch.path().join("effect.txt");
    let shutdown_path = scratch.path().join("shutdown.txt");
    let source = format!(
        r#"
local pi = ...
local roots = pi.roots.v1
local terminal = pi.terminal.v1
local models = pi.models.v1
local effects = pi.effects.v1

pi.kernel.v1.resource(function() effects.fs.write({shutdown}, "clean") end)
roots.register({{
  kind="application", id="launcher-spine", dispatch=function(snapshot)
    local input = terminal.input_buffer()
    local events = input:feed(snapshot.event.arguments[1])
    local text = ""
    for _, event in ipairs(events) do text = text .. event.data end
    local display = terminal.display()
    local frame = display:submit({{
      version=terminal.display_schema_version,
      viewport={{columns=20, rows=1}}, root=1,
      nodes={{{{id=1, rect={{x=0,y=0,width=20,height=1}},
               content={{kind="text", runs={{{{text=text}}}}}}}}}},
    }})
    effects.fs.write({effect}, text)
    local signal = effects.cancellation.new()
    local cancelled = effects.process.run("sh", {{"-c", "printf partial; sleep 10"}}, {{
      timeout_ms=5000, max_output_bytes=64, signal=signal,
      onData=function() signal:abort() end,
    }})
    local model = models.find("moonshotai", "kimi-k2.6")
    roots.action("launched", {{
      input=text, revision=frame.revision, cells=frame.painted_cells,
      file=effects.fs.read({effect}, 64),
      model={{provider=model.provider, id=model.id}},
      cancelled={{killed=cancelled.killed, stdout=cancelled.stdout}},
      manifest=snapshot.context.manifest,
    }})
    roots.action("shutdown", {{reason="complete"}})
  end,
}})
"#,
        effect = serde_json::to_string(&effect_path.to_string_lossy()).unwrap(),
        shutdown = serde_json::to_string(&shutdown_path.to_string_lossy()).unwrap(),
    );
    std::fs::write(scratch.path().join("application.lua"), source).unwrap();
    std::fs::write(
        scratch.path().join("packages.json"),
        r#"{"version":1,"packages":["application.lua"]}"#,
    )
    .unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_pi"))
        .current_dir(scratch.path())
        .env("HOME", scratch.path())
        .env_remove("PI_PACKAGE_MANIFEST")
        .env_remove("XDG_CONFIG_HOME")
        .env_remove("XDG_DATA_HOME")
        .env_remove("XDG_STATE_HOME")
        .env_remove("XDG_CACHE_HOME")
        .args(["--manifest", "packages.json", "--", "hello"])
        .output()
        .expect("run raw launcher");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let result: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(result["actions"].as_array().unwrap().len(), 2);
    let launched = &result["actions"][0];
    assert_eq!(launched["kind"], "launched");
    assert_eq!(launched["payload"]["input"], "hello");
    assert_eq!(launched["payload"]["revision"], 1);
    assert_eq!(launched["payload"]["cells"], 5);
    assert_eq!(launched["payload"]["file"], "hello");
    assert_eq!(
        launched["payload"]["model"],
        serde_json::json!({"provider":"moonshotai", "id":"kimi-k2.6"})
    );
    assert_eq!(
        launched["payload"]["cancelled"],
        serde_json::json!({"killed":true, "stdout":"partial"})
    );
    assert!(
        launched["payload"]["manifest"]
            .as_str()
            .unwrap()
            .ends_with("packages.json")
    );
    assert_eq!(result["actions"][1]["kind"], "shutdown");
    assert_eq!(std::fs::read_to_string(effect_path).unwrap(), "hello");
    assert_eq!(std::fs::read_to_string(shutdown_path).unwrap(), "clean");
}
