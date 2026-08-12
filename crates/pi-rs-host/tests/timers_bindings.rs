#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use pi_rs_host::{Host, HostConfig};

#[test]
fn timers_fire_and_clear() {
    let host = Host::new(HostConfig::default()).unwrap();
    host.load(
        "timers-demo.lua",
        include_str!("../../../examples/extensions/timers-demo.lua"),
    )
    .unwrap();
    let out = host.call_command("timers-demo", "").unwrap().unwrap();

    let order = out["order"].as_array().unwrap();
    // tick fires 3 times (interval self-clears on the 3rd call).
    let tick_count = order.iter().filter(|v| v.as_str() == Some("tick")).count();
    // timeout fired (the 20ms one).
    let timeout_count = order
        .iter()
        .filter(|v| v.as_str() == Some("timeout"))
        .count();
    // late-timeout and the cleared one must NOT have fired within 120ms.
    let late_count = order
        .iter()
        .filter(|v| v.as_str() == Some("late-timeout"))
        .count();
    let cleared_count = order
        .iter()
        .filter(|v| v.as_str() == Some("should-never-fire"))
        .count();

    assert_eq!(tick_count, 3);
    assert_eq!(timeout_count, 1);
    assert_eq!(late_count, 0);
    assert_eq!(cleared_count, 0);
    assert_eq!(out["ticks"], 3);
}
