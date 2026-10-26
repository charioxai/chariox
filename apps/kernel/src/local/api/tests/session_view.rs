//! MP-08 / MP-10 / MP-11: bundled view retains ordinary admission and attach semantics.
use super::*;

fn view_request(session_id: &str, preferred: Option<&str>) -> LocalDaemonRequest {
    LocalDaemonRequest::AttachSessionView(crate::local::AttachSessionViewRequest {
        session_id: session_id.into(),
        client_id: "initial-view-client".into(),
        capability_level: ClientCapabilityLevel::FullTerminal,
        preferred_agent_id: preferred.map(str::to_string),
        latest_prompt_count: None,
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
    assert!(agents.iter().all(|agent| agent.prefetched_blobs.is_none()));
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

#[test]
fn mp08_mp10_mp11_initial_view_is_a_bounded_latest_turn_not_a_blob_download() {
    use crate::history::{HistoryEventTurnContext, SessionHistoryEntry};
    use crate::terminal::TerminalOutputKind;
    let harness = LocalRouterTestHarness::new();
    let request = harness.fixture_session_request("initial-view-medium-history");
    let LocalDaemonResponse::SessionCreated { session, .. } = harness
        .dispatch(LocalDaemonRequest::CreateSession(request))
        .unwrap()
    else {
        panic!("session")
    };
    for index in 1..6 {
        harness
            .dispatch(LocalDaemonRequest::SpawnAgent(SpawnAgentRequest {
                session_id: session.id().into(),
                account_profile: None,
                alias: Some(format!("preview-{index}")),
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
        panic!("state")
    };
    let store = harness.with_app(|app| app.operational_history_store());
    let tool_body = "source line\n".repeat(8_192);
    for agent in session.agents() {
        for turn in 0..5 {
            let context = HistoryEventTurnContext {
                session_id: Some(session.id().into()),
                agent_id: Some(agent.id().into()),
                provider_run_id: Some("preview-run".into()),
                prompt_id: Some(format!("prompt-{}-{turn}", agent.id())),
                turn_id: Some(format!("turn-{}-{turn}", agent.id())),
                ..Default::default()
            };
            store
                .append_transcript(
                    &SessionHistoryEntry::user_prompt(
                        session.id(),
                        "preview-client",
                        agent.id(),
                        format!("Review source part {turn}"),
                    ),
                    context.clone(),
                )
                .unwrap();
            store.append_transcript(&SessionHistoryEntry::provider_output(
                session.id(), "preview-run", Some(agent.id()), TerminalOutputKind::ProviderTool,
                Some(format!("read-{}-{turn}", agent.id())),
                serde_json::json!({"id": format!("read-{}-{turn}", agent.id()), "tool": "read",
                    "status": "completed", "path": "src/lib.rs", "output": tool_body}).to_string()), context.clone()).unwrap();
            store
                .append_transcript(
                    &SessionHistoryEntry::provider_output(
                        session.id(),
                        "preview-run",
                        Some(agent.id()),
                        TerminalOutputKind::ProviderOutput,
                        Some(format!("answer-{}-{turn}", agent.id())),
                        "Source review conclusion.\n".repeat(80),
                    ),
                    context,
                )
                .unwrap();
        }
    }
    let response = harness.dispatch(view_request(session.id(), None)).unwrap();
    let bytes = serde_json::to_vec(&response).unwrap().len();
    eprintln!("MP-08 / MP-10 / MP-11 initial-view six-agent five-turn bytes={bytes}");
    assert!(
        bytes <= 64 * 1024,
        "initial view carries full blob bodies: {bytes} bytes"
    );
    let LocalDaemonResponse::SessionViewAttached { history, .. } = response else {
        panic!("view")
    };
    let LocalDaemonResponse::SessionHistoryOutline { agents } = *history else {
        panic!("outline")
    };
    assert_eq!(agents.len(), 6);
    for agent in agents {
        assert_eq!(agent.turns.len(), 1);
        assert!(agent.prefetched_blobs.is_none());
        assert!(agent.turns[0].user_prompt.entry.text.ends_with("part 4"));
        let tool = agent.turns[0]
            .blobs
            .iter()
            .find(|blob| blob.kind == crate::history::SessionHistoryEntryKind::ProviderTool)
            .unwrap();
        assert!(tool
            .preview
            .as_ref()
            .is_some_and(|preview| preview.contains("src/lib.rs")));
        let LocalDaemonResponse::SessionHistoryBlobContents { blobs } = harness
            .dispatch(LocalDaemonRequest::GetSessionHistoryBlobContents(
                crate::local::GetSessionHistoryBlobContentsRequest {
                    session_id: session.id().into(),
                    agent_id: agent.agent_id.clone(),
                    blob_ids: vec![tool.blob_id.clone()],
                },
            ))
            .unwrap()
        else {
            panic!("details")
        };
        assert!(blobs[0]
            .entries
            .iter()
            .any(
                |entry| serde_json::from_str::<serde_json::Value>(&entry.entry.text)
                    .ok()
                    .and_then(|value| value
                        .get("output")
                        .and_then(|output| output.as_str())
                        .map(str::to_owned))
                    .as_deref()
                    == Some(tool_body.as_str())
            ));
        let LocalDaemonResponse::SessionHistoryOutline { agents: older } = harness
            .dispatch(LocalDaemonRequest::GetSessionHistoryOutline(
                crate::local::GetSessionHistoryOutlineRequest {
                    session_id: session.id().into(),
                    agent_ids: Some(vec![agent.agent_id]),
                    latest_prompt_count: Some(4),
                    cursor: agent.next_cursor,
                    include_prefetched_content: false,
                },
            ))
            .unwrap()
        else {
            panic!("older outline")
        };
        assert_eq!(
            older[0].turns.len(),
            4,
            "initial preview preserves ordinary older-history paging"
        );
    }
}
