// MP-08 / MP-11, A01 focused policy regressions; same tests run against base.

fn agent(id: &str, creator: Option<&str>, controller: Option<&str>) -> crate::agent::AgentInstance {
    let mut a = crate::agent::AgentInstance::new(
        id,
        format!("ref-{id}"),
        "room",
        Some(id.into()),
        "dev-stub",
        None,
        None,
        None,
        crate::agent::GridPosition::new(0, 0, 1, 1),
    );
    a.set_controlled_by_metaagent_id(controller.map(str::to_string));
    let mut json = serde_json::to_value(a).unwrap();
    json["spawned_by_agent_id"] = serde_json::json!(creator);
    serde_json::from_value(json).unwrap()
}

fn session() -> crate::session::RuntimeSession {
    crate::session::RuntimeSession::new("room", None, "workspace", "worktree", "machine", "kernel")
}

#[test]
fn room_admission_direct_child_without_legacy_controller() {
    let a = agent("a", None, None);
    let b = agent("b", Some("a"), None);
    let result = super::request::meta_agent_request(
        &session(),
        &a,
        &["alias".into(), "b".into(), "new-name".into()],
        &[b],
        true,
    );
    assert!(
        result.is_ok(),
        "A01 direct creator must authorize independently of legacy controller"
    );
}

#[test]
fn room_admission_grandchild_controller_cannot_mint_rights() {
    let a = agent("a", None, None);
    let d = agent("d", Some("b"), Some("a"));
    let result = super::request::meta_agent_request(
        &session(),
        &a,
        &["delete".into(), "d".into()],
        &[d],
        true,
    );
    assert!(
        result.is_err(),
        "A01 legacy controller must not authorize grandchild deletion"
    );
}

#[test]
fn room_admission_peer_workflow_node_binding_needs_no_controller() {
    let a = agent("a", None, None);
    let p = agent("peer", None, None);
    let result = super::request::meta_workflow_request(
        &session(),
        &a,
        &["node".into(), "add".into(), "flow".into(), "peer".into()],
        &[p],
        true,
    );
    assert!(
        result.is_ok(),
        "A01 regular room peer may be bound to a workflow"
    );
}
