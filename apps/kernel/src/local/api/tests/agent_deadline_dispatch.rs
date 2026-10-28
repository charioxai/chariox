use super::*;

#[test]
fn lane_deadline_public_dispatch_sets_reads_and_rejects_wrong_room() {
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
