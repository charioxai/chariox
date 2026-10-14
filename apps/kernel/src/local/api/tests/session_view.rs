//! MP-08 / MP-10 / MP-11: bundled view retains ordinary admission and attach semantics.
use super::*;

fn view_request(session_id: &str, preferred: Option<&str>) -> LocalDaemonRequest {
    LocalDaemonRequest::AttachSessionView(crate::local::AttachSessionViewRequest {
        session_id: session_id.into(),
        client_id: "initial-view-client".into(),
        capability_level: ClientCapabilityLevel::FullTerminal,
        preferred_agent_id: preferred.map(str::to_string),
    })
}

#[test]
fn mp08_mp10_initial_session_view_reuses_state_history_and_attachment_paths() {
    let harness = LocalRouterTestHarness::new();
    let request = harness.fixture_session_request("initial-session-view");
    let session = match harness
        .dispatch(LocalDaemonRequest::CreateSession(request))
        .unwrap()
    {
        LocalDaemonResponse::SessionCreated { session, .. } => session,
        _ => panic!("expected session"),
    };
    let response = harness.dispatch(view_request(session.id(), None)).unwrap();
    let encoded = serde_json::to_value(&response).unwrap();
    assert_eq!(
        encoded["SessionViewAttached"]
            .as_object()
            .unwrap()
            .keys()
            .collect::<Vec<_>>(),
        ["attachment", "history", "state"]
    );
    assert!(encoded["SessionViewAttached"]["state"]
        .get("SessionState")
        .is_some());
    assert!(encoded["SessionViewAttached"]["history"]
        .get("SessionHistoryOutline")
        .is_some());
    let LocalDaemonResponse::SessionViewAttached {
        attachment,
        state,
        history,
    } = response
    else {
        panic!("expected initial view");
    };
    assert_eq!(attachment.session_id(), session.id());
    let LocalDaemonResponse::SessionState {
        session: projected, ..
    } = *state
    else {
        panic!("expected ordinary state");
    };
    assert_eq!(projected.id(), session.id());
    let LocalDaemonResponse::SessionHistoryOutline { agents } = *history else {
        panic!("expected ordinary outline");
    };
    assert_eq!(agents.len(), projected.agents().len());
    assert!(agents
        .iter()
        .all(|agent| agent.prefetched_blobs.as_ref().is_some_and(Vec::is_empty)));
    // Same-client reattach still uses the ordinary replacement binding.
    let replacement = harness.dispatch(view_request(session.id(), None)).unwrap();
    let LocalDaemonResponse::SessionViewAttached {
        attachment: replacement,
        ..
    } = replacement
    else {
        panic!("expected replacement view");
    };
    assert_ne!(replacement.id(), attachment.id());
    assert!(harness
        .dispatch(LocalDaemonRequest::DetachFromSession(
            DetachFromSessionRequest {
                attachment_id: replacement.id().into(),
            }
        ))
        .is_ok());
}

#[test]
fn mp08_mp10_mp11_initial_session_view_denies_nonmember_before_attachment() {
    let harness = LocalRouterTestHarness::new();
    let request = harness.fixture_session_request("initial-session-view-admission");
    let session = match harness
        .dispatch(LocalDaemonRequest::CreateSession(request))
        .unwrap()
    {
        LocalDaemonResponse::SessionCreated { session, .. } => session,
        _ => panic!("expected session"),
    };
    assert!(harness
        .dispatch_as_user("foreign-user", view_request(session.id(), None))
        .is_err());
    let before = serde_json::to_value(
        harness
            .dispatch(LocalDaemonRequest::GetSessionState(
                GetSessionStateRequest {
                    session_id: session.id().into(),
                },
            ))
            .unwrap(),
    )
    .unwrap();
    assert!(harness
        .dispatch(view_request("missing-session", None))
        .is_err());
    let after = serde_json::to_value(
        harness
            .dispatch(LocalDaemonRequest::GetSessionState(
                GetSessionStateRequest {
                    session_id: session.id().into(),
                },
            ))
            .unwrap(),
    )
    .unwrap();
    assert_eq!(before, after);
}

#[test]
fn mp08_mp10_initial_session_view_selects_the_preferred_six_agent_group() {
    let harness = LocalRouterTestHarness::new();
    let request = harness.fixture_session_request("initial-session-view-desk");
    let session = match harness
        .dispatch(LocalDaemonRequest::CreateSession(request))
        .unwrap()
    {
        LocalDaemonResponse::SessionCreated { session, .. } => session,
        _ => panic!("expected session"),
    };
    for index in 1..8 {
        harness
            .dispatch(LocalDaemonRequest::SpawnAgent(SpawnAgentRequest {
                session_id: session.id().into(),
                account_profile: None,
                alias: Some(format!("view-agent-{index}")),
                provider: Some("slow-structured".into()),
                model: Some("default".into()),
                effort: None,
                execution_mode: None,
                permission_level: None,
                worktree_id: None,
                kernel_ref: None,
                slice_ref: None,
                worktree_placement: None,
                metaagent: false,
            }))
            .unwrap();
    }
    let LocalDaemonResponse::SessionState { session, .. } = harness
        .dispatch(LocalDaemonRequest::GetSessionState(
            GetSessionStateRequest {
                session_id: session.id().into(),
            },
        ))
        .unwrap()
    else {
        panic!("expected state")
    };
    let preferred = session.agents()[7].id();
    let LocalDaemonResponse::SessionViewAttached { history, .. } = harness
        .dispatch(view_request(session.id(), Some(preferred)))
        .unwrap()
    else {
        panic!("expected view");
    };
    let LocalDaemonResponse::SessionHistoryOutline { agents } = *history else {
        panic!("expected outline")
    };
    let mut expected = session.agents()[6..]
        .iter()
        .map(|agent| agent.id().to_string())
        .collect::<Vec<_>>();
    expected.sort();
    assert_eq!(
        agents
            .iter()
            .map(|agent| agent.agent_id.clone())
            .collect::<Vec<_>>(),
        expected
    );
}
