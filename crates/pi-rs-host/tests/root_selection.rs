//! Explicit root selection: naming a registration instead of outbidding it.
//!
//! Without a selection the host resolves a root kind by the highest priority
//! among the active registrations and fails on a tie, so replacing a root
//! meant registering at a higher number than whatever shipped it. That is a
//! bidding war, not a declaration. `pi.roots.v1.list`/`select` let an ordinary
//! package read the live registry and name one row per kind.
//!
//! Every journey here runs through the public Lua surface: registration,
//! listing, selection, conflict, disposal. Rust decides nothing about which
//! root wins beyond honoring the named one.

#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

use pi_rs_host::kernel::{DispatchRequest, RootKind};
use pi_rs_host::{Host, HostConfig, HostError, PackageSource};

fn host() -> Host {
    Host::new(HostConfig {
        dispatch_timeout_ms: 5_000,
        ..HostConfig::default()
    })
    .expect("host")
}

fn request(root: RootKind) -> DispatchRequest {
    DispatchRequest::new(
        root,
        serde_json::json!({"kind":"probe"}),
        serde_json::json!({}),
    )
}

/// Two competing frontend roots: `shipped` outbids `replacement` on priority.
const COMPETITORS: &str = r#"
local roots = (...).roots.v1
local kernel = (...).kernel.v1

for _, entry in ipairs({ { id = "shipped", priority = 10 }, { id = "replacement", priority = 0 } }) do
  roots.register({
    kind = "frontend",
    id = entry.id,
    active = true,
    priority = entry.priority,
    dispatch = function()
      kernel.action("answered", { by = entry.id })
    end,
  })
end
"#;

fn answered_by(host: &Host, root: RootKind) -> String {
    let batch = host.dispatch(request(root)).expect("dispatch");
    batch.actions[0].payload["by"]
        .as_str()
        .expect("by")
        .to_owned()
}

#[test]
fn a_named_root_outranks_a_higher_priority_registration() {
    let host = host();
    host.load("memory://competitors", COMPETITORS)
        .expect("competitors");
    assert_eq!(
        answered_by(&host, RootKind::Frontend),
        "shipped",
        "priority alone resolves the kind before anything is selected"
    );

    host.load(
        "memory://selector",
        r#"
local roots = (...).roots.v1
roots.select("frontend", "replacement")
"#,
    )
    .expect("selector");
    assert_eq!(
        answered_by(&host, RootKind::Frontend),
        "replacement",
        "an explicit selection beats the higher priority registration"
    );
}

#[test]
fn listing_reports_resolution_inputs_without_the_dispatch_handle() {
    let host = host();
    host.load("memory://competitors", COMPETITORS)
        .expect("competitors");
    host.load(
        "memory://reporter",
        r#"
local pi = ...
local roots = pi.roots.v1
local kernel = pi.kernel.v1

roots.select("frontend", "replacement")

roots.register({
  kind = "application",
  id = "reporter",
  active = true,
  priority = 0,
  dispatch = function()
    local rows = {}
    local dispatchable = false
    for _, row in ipairs(roots.list("frontend")) do
      rows[#rows + 1] = row.id
        .. "/" .. row.kind
        .. "/" .. row.source
        .. "/" .. tostring(row.priority)
        .. "/" .. tostring(row.active)
        .. "/" .. tostring(row.selected)
      if row.dispatch ~= nil then dispatchable = true end
    end
    kernel.action("listed", { rows = rows, dispatchable = dispatchable })
  end,
})
"#,
    )
    .expect("reporter");

    let batch = host
        .dispatch(request(RootKind::Application))
        .expect("dispatch");
    let payload = &batch.actions[0].payload;
    assert_eq!(
        payload["rows"],
        serde_json::json!([
            "replacement/frontend/memory://competitors/0/true/true",
            "shipped/frontend/memory://competitors/10/true/false",
        ]),
        "rows are ordered by (kind, id) and carry the resolution inputs plus the live selection"
    );
    assert_eq!(
        payload["dispatchable"],
        serde_json::json!(false),
        "listing roots never hands out the dispatch function"
    );
}

#[test]
fn a_selection_naming_no_active_registration_fails_the_dispatch() {
    let host = host();
    host.load("memory://competitors", COMPETITORS)
        .expect("competitors");
    host.load(
        "memory://selector",
        r#"
local roots = (...).roots.v1
roots.select("frontend", "typo")
"#,
    )
    .expect("selector");

    let error = host
        .dispatch(request(RootKind::Frontend))
        .expect_err("a stale selection is not silently ignored");
    let message = error.to_string();
    assert!(
        message.contains("frontend root 'typo'")
            && message.contains("memory://selector")
            && message.contains("not registered and active"),
        "the diagnostic names the kind, the id, and who selected it: {message}"
    );
}

#[test]
fn an_inactive_registration_cannot_be_revived_by_selecting_it() {
    let host = host();
    host.load(
        "memory://roots",
        r#"
local roots = (...).roots.v1
local kernel = (...).kernel.v1

roots.register({ kind = "agent", id = "live", active = true, priority = 0,
  dispatch = function() kernel.action("answered", { by = "live" }) end })
roots.register({ kind = "agent", id = "retired", active = false, priority = 50,
  dispatch = function() kernel.action("answered", { by = "retired" }) end })
roots.select("agent", "retired")
"#,
    )
    .expect("roots");

    let error = host
        .dispatch(request(RootKind::Agent))
        .expect_err("an inactive root stays inactive");
    assert!(
        error.to_string().contains("agent root 'retired'"),
        "{error}"
    );
}

#[test]
fn a_second_source_cannot_select_a_kind_another_package_owns() {
    let host = host();
    host.load("memory://competitors", COMPETITORS)
        .expect("competitors");
    host.load(
        "memory://first",
        r#"(...).roots.v1.select("frontend", "replacement")"#,
    )
    .expect("first");

    let error = host
        .load(
            "memory://second",
            r#"(...).roots.v1.select("frontend", "shipped")"#,
        )
        .expect_err("two sources selecting one kind is a conflict");
    let message = error.to_string();
    assert!(
        message.contains("root selection") && message.contains("memory://first"),
        "the conflict names the mechanism and the owner: {message}"
    );
    assert_eq!(
        answered_by(&host, RootKind::Frontend),
        "replacement",
        "the refused selection changed nothing"
    );
}

#[test]
fn the_same_source_may_reselect_and_clear_its_own_kind() {
    let host = host();
    host.load("memory://competitors", COMPETITORS)
        .expect("competitors");
    host.load(
        "memory://selector",
        r#"
local roots = (...).roots.v1
local kernel = (...).kernel.v1

roots.select("frontend", "replacement")

roots.register({ kind = "application", id = "control", active = true, priority = 0,
  dispatch = function(snapshot)
    local command = snapshot.event.command
    if command == "clear" then
      roots.select("frontend")
    else
      roots.select("frontend", command)
    end
    kernel.action("selected", { command = command })
  end })
"#,
    )
    .expect("selector");

    let clear = DispatchRequest::new(
        RootKind::Application,
        serde_json::json!({"command":"clear"}),
        serde_json::json!({}),
    );
    host.dispatch(clear).expect("clear");
    assert_eq!(
        answered_by(&host, RootKind::Frontend),
        "shipped",
        "clearing the selection hands the kind back to priority resolution"
    );

    let reselect = DispatchRequest::new(
        RootKind::Application,
        serde_json::json!({"command":"replacement"}),
        serde_json::json!({}),
    );
    host.dispatch(reselect).expect("reselect");
    assert_eq!(
        answered_by(&host, RootKind::Frontend),
        "replacement",
        "the owning source may select the same kind again"
    );
}

#[test]
fn disposing_the_selecting_package_restores_priority_resolution() {
    let host = host();
    host.load("memory://competitors", COMPETITORS)
        .expect("competitors");
    let selector = host
        .load_package(PackageSource::Memory {
            key: "memory://selector",
            source: r#"(...).roots.v1.select("frontend", "replacement")"#,
        })
        .expect("selector");
    assert_eq!(answered_by(&host, RootKind::Frontend), "replacement");

    host.dispose_package(&selector).expect("dispose");
    assert_eq!(
        answered_by(&host, RootKind::Frontend),
        "shipped",
        "a selection is scope-owned: disposal cannot leave a dangling choice"
    );
}

#[test]
fn selection_and_listing_refuse_a_kind_no_package_can_register() {
    let host = host();
    let error = host
        .load(
            "memory://session-selector",
            r#"(...).roots.v1.select("session", "anything")"#,
        )
        .expect_err("an unregisterable kind cannot be selected either");
    assert!(
        error
            .to_string()
            .contains("roots.v1 supports application, agent, and frontend roots"),
        "{error}"
    );

    let listed = host.load(
        "memory://session-lister",
        r#"(...).roots.v1.list("session")"#,
    );
    assert!(
        listed
            .expect_err("listing agrees with registering")
            .to_string()
            .contains("roots.v1 supports application, agent, and frontend roots")
    );
}

#[test]
fn an_empty_selection_id_is_refused_rather_than_treated_as_clearing() {
    let host = host();
    let error = host
        .load(
            "memory://blank",
            r#"(...).roots.v1.select("frontend", "  ")"#,
        )
        .expect_err("a blank id is a typo, not a clear");
    assert!(
        error
            .to_string()
            .contains("root selection id must be a non-empty string"),
        "{error}"
    );
}

#[test]
fn an_unselected_kind_still_reports_a_tie_as_a_conflict() {
    let host = host();
    host.load(
        "memory://tied",
        r#"
local roots = (...).roots.v1
local kernel = (...).kernel.v1
for _, id in ipairs({ "left", "right" }) do
  roots.register({ kind = "frontend", id = id, active = true, priority = 0,
    dispatch = function() kernel.action("answered", { by = id }) end })
end
"#,
    )
    .expect("tied");

    assert!(matches!(
        host.dispatch(request(RootKind::Frontend)),
        Err(HostError::Conflict(_))
    ));

    host.load(
        "memory://tiebreaker",
        r#"(...).roots.v1.select("frontend", "right")"#,
    )
    .expect("tiebreaker");
    assert_eq!(
        answered_by(&host, RootKind::Frontend),
        "right",
        "selection is how a tie is decided without renumbering either side"
    );
}
