//! Push-model settings list: `pi.register_setting_item(def)` /
//! `pi.registered_setting_items()` — the host registry dogfood tool-management
//! uses to add custom rows (e.g. an active-tools filter) to the `/settings`
//! dialog. First registration per id wins across extensions; the read side
//! returns one entry per id in insertion order. Exercised unprivileged from
//! two file-backed extensions.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use pi_rs_host::{Host, HostConfig};

fn host() -> Host {
    Host::new(HostConfig::default()).unwrap()
}

#[test]
fn push_model_setting_items_first_registration_wins_in_order() {
    let host = host();
    // Probe command reads the resolved settings items.
    host.load(
        "<probe>",
        r#"
            local pi = ...
            pi.register_command("settings-items", {
                handler = function()
                    local out = {}
                    for _, item in ipairs(pi.registered_setting_items()) do
                        out[#out + 1] = { id = item.id, label = item.label, kind = item.type,
                                          settings_key = item.settings_key, value = item.default }
                    end
                    return out
                end,
            })
        "#,
    )
    .unwrap();

    // First extension registers the active-tools filter row.
    host.load(
        "<tool-management>",
        r#"
            local pi = ...
            pi.register_setting_item({
                id = "activeToolsFilter",
                label = "Filter active tools",
                type = "text",
                settings_key = "toolManagement.activeFilter",
                default = "",
                source = "tool-management",
            })
        "#,
    )
    .unwrap();
    // Second extension tries to claim the same id (first-registration wins);
    // it also adds a distinct row.
    host.load(
        "<other>",
        r#"
            local pi = ...
            pi.register_setting_item({
                id = "activeToolsFilter",
                label = "overridden label",
                type = "toggle",
                settings_key = "other.filter",
            })
            pi.register_setting_item({
                id = "modelCompaction",
                label = "Auto-compact model context",
                type = "toggle",
                settings_key = "compaction.enabled",
                default = true,
            })
        "#,
    )
    .unwrap();

    let items = host
        .call_command("settings-items", "")
        .expect("settings-items")
        .unwrap();
    let items = items.as_array().unwrap();
    assert_eq!(items.len(), 2, "{items:?}");
    // First registration wins for the colliding id (label stays tool-management's).
    assert_eq!(items[0]["id"], "activeToolsFilter");
    assert_eq!(items[0]["label"], "Filter active tools");
    assert_eq!(items[0]["kind"], "text");
    assert_eq!(items[0]["settings_key"], "toolManagement.activeFilter");
    assert_eq!(items[0]["value"], "");
    // The second extension's distinct row is appended in registration order.
    assert_eq!(items[1]["id"], "modelCompaction");
    assert_eq!(items[1]["label"], "Auto-compact model context");
    assert_eq!(items[1]["kind"], "toggle");
    assert_eq!(items[1]["value"], true);
}
