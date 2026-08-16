#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

//! Exerciser for the additive `pi.kernel` surface (docs/pi-kernel-surface.md,
//! Stage 0). Loads the file-backed `examples/extensions/kernel-demo.lua`
//! through the public Host path and asserts the whole surface: mount applies
//! a port's declarative effects, get/has/set/remove work on the VM-resident
//! write context, a component's on_change fires exactly for a declared read
//! key, and unmount replays the effect inverse so the context returns to its
//! pre-mount state (no residue).

use pi_rs_host::{Host, HostConfig};

fn host(cwd: &std::path::Path) -> Host {
    Host::new(HostConfig {
        cwd: Some(cwd.to_string_lossy().into_owned()),
        ..HostConfig::default()
    })
    .unwrap()
}

fn demo_path() -> &'static str {
    concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../examples/extensions/kernel-demo.lua"
    )
}

#[test]
fn kernel_lua_surface_mount_get_set_has_remove_on_change_unmount() {
    let root = tempfile::tempdir().unwrap();
    let host = host(root.path());

    // The file-backed consumer loads through the public Host path.
    host.load_file(demo_path())
        .expect("kernel-demo.lua loads through the public host path");

    let result = host
        .call_command("kernel-surface-demo", "")
        .expect("kernel-surface-demo runs")
        .expect("kernel-surface-demo result");

    // get/has/set/remove on the committed write path.
    assert_eq!(result["probeGet"], "hello", "set+get round-trip");
    assert_eq!(result["probeHas"], true);
    assert_eq!(result["missingHas"], false, "has for an absent key");
    assert_eq!(result["removedHas"], false, "remove clears the key");

    // mount applies its effects: the composed key is now present.
    assert_eq!(result["mountedEditor"], "idle", "mount effect applied");
    assert_eq!(result["editorHas"], true);

    // spatial on_change fires exactly for a declared read key match...
    assert_eq!(result["onChangeFired"], true, "on_change did not fire");
    assert_eq!(result["changedKey"], "theme", "on_change got the wrong key");
    // ...and is untouched by a set on a non-declared key.
    assert_eq!(result["noChangeOnUndeclared"], true, "on_change leaked scope");

    // unmount replays the effect inverse in reverse: residue-empty.
    assert_eq!(result["editorGone"], true, "editor key survives unmount");
    assert_eq!(result["editorGoneGet"], serde_json::Value::Null);
    // pre-mount committed values are untouched by mount/unmount.
    assert_eq!(result["themeStillSet"], true);
    assert_eq!(result["baselineOneStill"], true);
    assert_eq!(result["baselineTwoStill"], true);
    assert_eq!(result["residueFree"], true, "mount/unmount left residue");
}