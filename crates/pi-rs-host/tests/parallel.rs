#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use pi_rs_host::{Host, HostConfig};

#[test]
fn public_parallel_example_completes_in_completion_order() {
    let host = Host::new(HostConfig::default()).expect("host");
    let source = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../examples/extensions/parallel-demo.lua"
    ))
    .expect("example");
    host.load("examples/extensions/parallel-demo.lua", &source)
        .expect("load");
    let value = host
        .call_command("parallel-demo", "")
        .expect("call")
        .expect("value");
    assert_eq!(value["first"], "fast");
    assert_eq!(value["second"], "slow");
    assert_eq!(value["firstIndex"], 2);
}

#[test]
fn public_spawn_example_runs_work_in_the_background() {
    let host = Host::new(HostConfig::default()).expect("host");
    let source = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../examples/extensions/spawn-demo.lua"
    ))
    .expect("example");
    host.load("examples/extensions/spawn-demo.lua", &source)
        .expect("load");
    let value = host
        .call_command("spawn-demo", "")
        .expect("call")
        .expect("value");
    assert_eq!(value["value"], "background-done");
    assert_eq!(value["done"], true);
    assert!(value["ticks"].as_u64().expect("ticks") >= 1);
}
