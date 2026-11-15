use super::*;

#[test]
fn agent_deadline_public_dispatch_sets_reads_and_rejects_wrong_room() {
    let worktree = crate::test_support::TestWorktree::new("deadline-public-dispatch");
    let harness = LocalRouterTestHarness::new();
    let create = || match harness
        .dispatch(LocalDaemonRequest::CreateSession(
            worktree.session_request(),
        ))
        .unwrap()
    {
        LocalDaemonResponse::SessionCreated { session, agent, .. } => (session, agent),
        _ => panic!("expected room"),
    };
    let (room, agent) = create();
    let deadline_ms = 4_102_444_800_000_u64;
    let request = |session: &str| {
        serde_json::from_value::<LocalDaemonRequest>(serde_json::json!({
        "SetAgentDeadline": {"session_id": session, "agent_id": agent.id(), "deadline_ms": deadline_ms}
    })).unwrap()
    };
    let armed = harness
        .dispatch(request(room.id()))
        .expect("public deadline dispatch must be supported");
    let value = serde_json::to_value(armed).unwrap();
    assert_eq!(
        value["AgentDeadline"]["deadline"]["deadline_ms"],
        deadline_ms
    );
    let get = serde_json::from_value::<LocalDaemonRequest>(serde_json::json!({
        "GetAgentDeadline": {"session_id": room.id(), "agent_id": agent.id()}
    }))
    .unwrap();
    let read = serde_json::to_value(harness.dispatch(get).unwrap()).unwrap();
    assert_eq!(
        read["AgentDeadline"]["deadline"]["deadline_ms"],
        deadline_ms
    );
    let (other, _) = create();
    assert!(matches!(
        harness.dispatch(request(other.id())),
        Err(DaemonError::AgentNotInSession { .. })
    ));
}

#[test]
fn agent_deadline_public_dispatch_extends_clears_and_shortens() {
    let worktree = crate::test_support::TestWorktree::new("deadline-edit-dispatch");
    let harness = LocalRouterTestHarness::new();
    let (room, agent) = match harness
        .dispatch(LocalDaemonRequest::CreateSession(
            worktree.session_request(),
        ))
        .unwrap()
    {
        LocalDaemonResponse::SessionCreated { session, agent, .. } => (session, agent),
        _ => panic!("expected room"),
    };
    let set = |deadline: serde_json::Value| {
        let request = serde_json::from_value::<LocalDaemonRequest>(serde_json::json!({
            "SetAgentDeadline": {"session_id": room.id(), "agent_id": agent.id(), "deadline_ms": deadline}
        })).expect("deadline edit must accept a time or null");
        serde_json::to_value(
            harness
                .dispatch(request)
                .expect("deadline edit must succeed"),
        )
        .unwrap()
    };
    let first = 4_102_444_800_000_u64;
    set(first.into());
    let extended = set((first + 60_000).into());
    assert_eq!(
        extended["AgentDeadline"]["deadline"]["deadline_ms"],
        first + 60_000
    );
    let cleared = set(serde_json::Value::Null);
    assert!(cleared["AgentDeadline"]["deadline"].is_null());
    let read = harness
        .dispatch(LocalDaemonRequest::GetAgentDeadline(
            crate::local::GetAgentDeadlineRequest {
                session_id: room.id().into(),
                agent_id: agent.id().into(),
            },
        ))
        .unwrap();
    assert!(matches!(
        read,
        LocalDaemonResponse::AgentDeadline { deadline: None }
    ));
    set(first.into());
    let shortened = set((first - 60_000).into());
    assert_eq!(
        shortened["AgentDeadline"]["deadline"]["deadline_ms"],
        first - 60_000
    );
}
