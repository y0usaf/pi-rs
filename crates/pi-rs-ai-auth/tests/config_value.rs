//! Stored literal/environment/command credential resolution.

#![allow(clippy::unwrap_used)]

use pi_rs_ai_auth::resolve_config_value;

#[tokio::test]
async fn resolves_literals_templates_escapes_and_bounded_commands() {
    let variable = format!("PI_RS_AUTH_TEST_{}", std::process::id());
    // SAFETY: this test binary has one environment-mutating test.
    unsafe { std::env::set_var(&variable, "environment-secret") };

    assert_eq!(
        resolve_config_value("literal-secret").await.as_deref(),
        Some("literal-secret")
    );
    assert_eq!(
        resolve_config_value(&format!("prefix-${{{variable}}}-$${variable}-$!")).await,
        Some(format!("prefix-environment-secret-${variable}-!"))
    );
    assert!(
        resolve_config_value("$PI_RS_AUTH_MISSING_VALUE")
            .await
            .is_none()
    );
    assert_eq!(
        resolve_config_value("!printf 'command-secret\\n'")
            .await
            .as_deref(),
        Some("command-secret")
    );
    assert!(resolve_config_value("!exit 7").await.is_none());

    unsafe { std::env::remove_var(&variable) };
}
