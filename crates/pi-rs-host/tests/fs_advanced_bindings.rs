#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use pi_rs_host::{Host, HostConfig};
use std::path::PathBuf;

fn host() -> Host {
    Host::new(HostConfig::default()).unwrap()
}

fn scratch_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("pi-rs-{name}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn load_demo(host: &Host) {
    host.load(
        "fs-advanced.lua",
        include_str!("../../../examples/extensions/fs-advanced.lua"),
    )
    .unwrap();
}

#[test]
fn fs_symlink_metadata_atomic_rename() {
    let dir = scratch_dir("fs-adv");
    let host = host();
    load_demo(&host);
    let out = host
        .call_command("fs-advanced", &dir.to_string_lossy())
        .unwrap()
        .unwrap();
    assert_eq!(out["link_target"], "data.txt");
    assert_eq!(out["lstat_is_symlink"], true);
    assert_eq!(out["stat_follows_to_file"], true);
    assert_eq!(out["mode"], 384); // 0600
    assert_eq!(out["content"], "atomic content\n");
    assert_eq!(out["tmp_exists"], true);
    assert_eq!(out["can_access"], true);
    assert_eq!(out["moved_exists"], true);
    // rmdir / remove_dir / remove_dir_all (rmSync).
    assert_eq!(out["single_removed"], true);
    assert_eq!(out["nested_removed"], true);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn fs_watch_polls_and_closes() {
    let dir = scratch_dir("fs-watch");
    let file = dir.join("watched.txt");
    let host = host();
    load_demo(&host);
    let out = host
        .call_command("fs-watch", &file.to_string_lossy())
        .unwrap()
        .unwrap();
    assert_eq!(out["before"], false);
    assert_eq!(out["fired"], true);
    assert_eq!(out["kinds"], serde_json::json!(["change"]));
    assert_eq!(out["closed"], true);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn fs_stat_reports_rich_metadata() {
    let dir = scratch_dir("fs-stat");
    let file = dir.join("meta.txt");
    std::fs::write(&file, "hello").unwrap();
    let host = host();
    host.load(
        "test://stat",
        &format!(
            r#"
            local pi = ...
            pi.register_command("stat", {{
                handler = function()
                    local s = pi.fs.stat("{}")
                    return {{ size = s.size, kind = s.type, nlink = s.nlink,
                        has_uid = s.uid > 0, has_mode = s.mode > 0,
                        modified_positive = s.modified_ms > 0 }}
                end,
            }})
            "#,
            file.display()
        ),
    )
    .unwrap();
    let out = host.call_command("stat", "").unwrap().unwrap();
    assert_eq!(out["size"], 5);
    assert_eq!(out["kind"], "file");
    assert_eq!(out["nlink"], 1);
    assert_eq!(out["has_uid"], true);
    assert_eq!(out["has_mode"], true);
    assert_eq!(out["modified_positive"], true);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn fs_chmod_rejects_setuid_and_applies_octal_mode() {
    let dir = scratch_dir("fs-chmod");
    let file = dir.join("mode.txt");
    std::fs::write(&file, "x").unwrap();
    let host = host();
    host.load(
        "test://chmod",
        r#"
        local pi = ...
        pi.register_command("chmod", {
            handler = function(arg)
                local ok_setuid, err = pcall(pi.fs.chmod, arg, "4755")
                -- A sticky write bit is fine; a readable-and-writable file.
                pi.fs.chmod(arg, "0644")
                local mode = pi.fs.lstat(arg).mode
                local ok_sticky, sticky_err = pcall(pi.fs.chmod, arg, "1644")
                return { setuid_rejected = not ok_setuid,
                         setuid_error = ok_setuid and "" or tostring(err),
                         mode = mode,
                         sticky_ok = ok_sticky }
            end,
        })
        "#,
    )
    .unwrap();
    let out = host
        .call_command("chmod", &file.to_string_lossy())
        .unwrap()
        .unwrap();
    assert_eq!(out["setuid_rejected"], true);
    assert!(out["setuid_error"].as_str().unwrap().contains("setuid"));
    assert_eq!(out["mode"], 0o644);
    assert_eq!(out["sticky_ok"], true);
    let _ = std::fs::remove_dir_all(&dir);
}
