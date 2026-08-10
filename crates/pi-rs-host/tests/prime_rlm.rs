#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use pi_rs_host::{Host, HostConfig};

fn host() -> Host {
    Host::new(HostConfig {
        dispatch_timeout_ms: 90_000,
        cwd: None,
        project_trusted: true,
    })
    .expect("host")
}

fn load_prime(host: &Host) {
    host.load("prime/rlm.lua", include_str!("../../../prime/rlm.lua"))
        .expect("prime RLM package loads");
}

#[test]
fn prime_rlm_package_loads_through_public_loader() {
    let host = host();
    load_prime(&host);
}

/// The `.#prime` app composes the RLM package through the public loader and
/// dispatches to the `prime-rlm` role it registers. This pins that the role is
/// reachable by its generic role name after an ordinary file-backed load — the
/// wiring path the launcher's `--role prime-rlm --package prime/rlm.lua` uses.
#[test]
fn prime_rlm_role_is_registered_through_public_loader() {
    let host = host();
    load_prime(&host);
    let roles = host.roles().expect("roles");
    let prime_rlm = roles
        .iter()
        .find(|role| role.role == "prime-rlm")
        .expect("prime-rlm role registered");
    assert!(prime_rlm.active, "prime-rlm role must be active");
    assert_eq!(prime_rlm.id, "prime-rlm");
    assert_eq!(prime_rlm.source, "prime/rlm.lua");
}
