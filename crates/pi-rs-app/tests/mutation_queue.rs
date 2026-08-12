#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use pi_rs_app::builtins::TOOLS_PACK;
use pi_rs_host::{Host, HostConfig};

fn host() -> Host {
    let host = Host::new(HostConfig::default()).unwrap();
    let report = host.load_embedded(&[TOOLS_PACK]);
    assert!(
        report.errors.is_empty(),
        "tools pack loads: {:?}",
        report.errors
    );
    host
}

#[test]
fn mutation_queue_folder_leases_releases_and_reads_back() {
    let dir = std::env::temp_dir().join(format!("pi-rs-mutation-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("m.txt");
    let host = host();
    host.load(
        "mutation-demo.lua",
        &std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../examples/extensions/mutation-demo.lua"
        ))
        .unwrap(),
    )
    .unwrap();
    let out = host
        .call_command("mutation-demo", &file.to_string_lossy())
        .unwrap()
        .unwrap();
    assert_eq!(out["locked_inside"], true);
    assert_eq!(out["active_before_release"], 1);
    assert_eq!(out["content"], "mutated\n");
    assert_eq!(out["released_after_error"], true);
    assert_eq!(out["locks_after"], 0);
    let _ = std::fs::remove_file(&file);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn mutation_queue_is_reusable_by_file_backed_package() {
    let dir = std::env::temp_dir().join(format!("pi-rs-mutation2-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("r.txt");
    let host = host();
    // A file-backed package requires the same exact-version module the
    // builtin tools use, then runs a mutation through it.
    host.load(
        "file-backed-mutator.lua",
        &format!(
            r#"
            local pi = ...
            local mutation = pi.module.require("pi.tools.file-mutation", "1")
            pi.register_command("mutate", {{
                handler = function()
                    mutation.with_file_mutation_queue("{}", function()
                        pi.fs.write_file("{}", "via module")
                    end)
                    return mutation.active_lock_count()
                end,
            }})
            "#,
            file.display(),
            file.display()
        ),
    )
    .unwrap();
    let out = host.call_command("mutate", "").unwrap().unwrap();
    assert_eq!(out, 0);
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "via module");
    let _ = std::fs::remove_file(&file);
    let _ = std::fs::remove_dir_all(&dir);
}
