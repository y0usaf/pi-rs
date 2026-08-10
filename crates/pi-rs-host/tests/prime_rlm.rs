#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use pi_rs_host::{Host, HostConfig};

#[test]
fn prime_rlm_package_loads_through_public_loader() {
    let host = Host::new(HostConfig { dispatch_timeout_ms: 90_000, cwd: None, project_trusted: true }).expect("host");
    host.load("prime/rlm.lua", include_str!("../../../prime/rlm.lua")).expect("prime RLM package loads");
}
