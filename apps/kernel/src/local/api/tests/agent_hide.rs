use super::*;

fn hide_request(session_id: &str, agent_id: &str, hidden: bool) -> LocalDaemonRequest {
    serde_json::from_value(serde_json::json!({"HideAgent": {
        "session_id": session_id, "agent_id": agent_id, "hidden": hidden
    }}))
    .expect("HideAgent must be part of the shared protocol")
}

fn hidden_in_snapshot(h: &LocalRouterTestHarness, room: &str, agent_id: &str) -> bool {
    let snapshot: LocalDaemonRequest =
        serde_json::from_value(serde_json::json!({"GetSessionState": {"session_id": room}}))
            .unwrap();
    let value = serde_json::to_value(h.dispatch(snapshot).unwrap()).unwrap();
    value["SessionState"]["session"]["agents"]
        .as_array()
        .unwrap()
        .iter()
        .find(|agent| agent["id"] == agent_id)
        .is_some_and(|agent| agent["hidden"] == true)
}

#[test]
fn hide_agent_never_stops_its_run_or_queue_and_comes_back_when_the_work_is_done() {
    let worktree = crate::test_support::TestWorktree::new("agent-hide-work");
    let h = LocalRouterTestHarness::new();
    let LocalDaemonResponse::SessionCreated { session, .. } = h
        .dispatch(LocalDaemonRequest::CreateSession(
            worktree.session_request(),
        ))
        .unwrap()
    else {
        panic!("session")
    };
    let agent = h.spawn_workflow_test_agent(session.id(), "hide-me");
    h.launch_workflow_test_provider(session.id(), agent.id());
    let run_id = h.with_app(|app| {
        app.providers()
            .get_run_for_agent(session.id(), agent.id())
            .unwrap()
            .id()
            .to_string()
    });
    let LocalDaemonResponse::SessionAttached { attachment } = h
        .dispatch(LocalDaemonRequest::AttachToSession(
            AttachToSessionRequest {
                session_id: session.id().into(),
                client_id: "hide-test".into(),
                capability_level: ClientCapabilityLevel::FullTerminal,
            },
        ))
        .unwrap()
    else {
        panic!("attachment")
    };
    for text in ["first hidden work", "queued hidden work"] {
        h.dispatch(LocalDaemonRequest::SubmitPrompt(SubmitPromptRequest {
            session_id: session.id().into(),
            attachment_id: attachment.id().into(),
            target_agent_id: Some(agent.id().into()),
            prompt: text.into(),
            attachments: vec![],
        }))
        .unwrap();
    }

    let hidden = serde_json::to_value(
        h.dispatch(hide_request(session.id(), agent.id(), true))
            .expect("a working agent can be hidden"),
    )
    .unwrap();
    assert_eq!(hidden["AgentHidden"]["agent"]["hidden"], true);
    assert_eq!(hidden["AgentHidden"]["agent"]["archived"], false);
    assert_ne!(
        h.with_app(|app| app.providers().get_run(&run_id).unwrap().state()),
        crate::provider::ProviderRunState::Ended,
        "hiding must not end the run"
    );
    assert!(
        h.with_app(|app| {
            let room = app.sessions().get_session(session.id()).unwrap();
            room.active_prompt_for_agent(agent.id()).is_some()
                && room
                    .queued_prompts_for_agent(agent.id())
                    .is_some_and(|queue| queue.len() == 1)
        }),
        "hiding must keep the active and the queued prompt"
    );
    assert!(hidden_in_snapshot(&h, session.id(), agent.id()));

    h.dispatch(LocalDaemonRequest::CompletePrompt(CompletePromptRequest {
        session_id: session.id().into(),
    }))
    .unwrap();
    assert!(
        hidden_in_snapshot(&h, session.id(), agent.id()),
        "the queued prompt is still work, so the agent stays hidden"
    );
    h.dispatch(LocalDaemonRequest::CompletePrompt(CompletePromptRequest {
        session_id: session.id().into(),
    }))
    .unwrap();
    assert!(
        !hidden_in_snapshot(&h, session.id(), agent.id()),
        "the last prompt finished, so the agent is back in every client's Room"
    );
}

#[test]
fn hide_agent_on_an_idle_agent_stays_until_it_is_unhidden_and_checks_ownership() {
    let worktree = crate::test_support::TestWorktree::new("agent-hide-idle");
    let h = LocalRouterTestHarness::new();
    let LocalDaemonResponse::SessionCreated { session, agent, .. } = h
        .dispatch(LocalDaemonRequest::CreateSession(
            worktree.session_request(),
        ))
        .unwrap()
    else {
        panic!("session")
    };
    assert!(h
        .dispatch(hide_request("wrong-room", agent.id(), true))
        .is_err());
    assert!(h
        .dispatch_as_user("other-user", hide_request(session.id(), agent.id(), true))
        .is_err());
    assert!(!hidden_in_snapshot(&h, session.id(), agent.id()));
    h.dispatch(hide_request(session.id(), agent.id(), true))
        .unwrap();
    assert!(hidden_in_snapshot(&h, session.id(), agent.id()));
    let restored = serde_json::to_value(
        h.dispatch(hide_request(session.id(), agent.id(), false))
            .unwrap(),
    )
    .unwrap();
    assert!(restored["AgentHidden"]["agent"].get("hidden").is_none());
    assert!(!hidden_in_snapshot(&h, session.id(), agent.id()));
}

#[test]
fn hide_agent_protocol_shape_and_hash() {
    let value = serde_json::to_value(hide_request("room", "agent", true)).unwrap();
    assert_eq!(
        value,
        serde_json::json!({"HideAgent": {"session_id":"room", "agent_id":"agent", "hidden":true}})
    );
}

#[test]
fn hidden_agent_is_flagged_in_the_public_snapshot() {
    let worktree = crate::test_support::TestWorktree::new("agent-hide-public");
    let h = LocalRouterTestHarness::new();
    let LocalDaemonResponse::SessionCreated { session, agent, .. } = h
        .dispatch(LocalDaemonRequest::CreateSession(
            worktree.session_request(),
        ))
        .unwrap()
    else {
        panic!("session")
    };
    let flagged = |h: &LocalRouterTestHarness| {
        let LocalDaemonResponse::WaitingRoomPublicSnapshot { snapshot } = h
            .dispatch(LocalDaemonRequest::GetWaitingRoomPublicSnapshot(
                GetWaitingRoomPublicSnapshotRequest,
            ))
            .unwrap()
        else {
            panic!("expected snapshot")
        };
        serde_json::to_value(snapshot).unwrap()["sessions"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|room| room["id"] == session.id())
            .flat_map(|room| room["agents"].as_array().unwrap().clone())
            .find(|each| each["id"] == agent.id())
            .map(|each| each["hidden"] == true)
            .unwrap_or(false)
    };
    assert!(!flagged(&h));
    h.dispatch(hide_request(session.id(), agent.id(), true))
        .unwrap();
    assert!(
        flagged(&h),
        "the Navigator's snapshot names the hidden agent"
    );
}
