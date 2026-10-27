use super::*;
use crate::local::GetSessionHistoryOutlineRequest;

fn archive_request(session_id: &str, agent_id: &str, archived: bool) -> LocalDaemonRequest {
    serde_json::from_value(serde_json::json!({"ArchiveAgent": {
        "session_id": session_id, "agent_id": agent_id, "archived": archived
    }}))
    .expect("ArchiveAgent must be part of the shared protocol")
}

#[test]
fn archive_agent_retains_identity_and_repairs_focus_in_session_snapshot() {
    let worktree = crate::test_support::TestWorktree::new("agent-archive");
    let harness = LocalRouterTestHarness::new();
    let LocalDaemonResponse::SessionCreated {
        session,
        agent: default_agent,
        ..
    } = harness
        .dispatch(LocalDaemonRequest::CreateSession(
            worktree.session_request(),
        ))
        .unwrap()
    else {
        panic!("session not created")
    };
    let id = default_agent.id();
    let response = harness
        .dispatch(archive_request(session.id(), id, true))
        .expect("idle agent should archive");
    let value = serde_json::to_value(response).unwrap();
    assert_eq!(value["AgentArchived"]["agent"]["id"], id);
    assert_eq!(value["AgentArchived"]["agent"]["archived"], true);
    assert!(value["AgentArchived"]["session"]["focused_agent_id"].is_null());
    assert_eq!(
        value["AgentArchived"]["session"]["agents"][0]["archived"],
        true
    );
    let focus: LocalDaemonRequest = serde_json::from_value(serde_json::json!({"FocusAgent": {
        "session_id": session.id(), "agent_id": id
    }}))
    .unwrap();
    assert!(
        harness.dispatch(focus).is_err(),
        "archived agent cannot receive focus"
    );
    let restored = serde_json::to_value(
        harness
            .dispatch(archive_request(session.id(), id, false))
            .unwrap(),
    )
    .unwrap();
    assert_eq!(restored["AgentArchived"]["agent"]["id"], id);
    assert_eq!(restored["AgentArchived"]["agent"]["archived"], false);
    assert_eq!(restored["AgentArchived"]["session"]["focused_agent_id"], id);
}

#[test]
fn archive_agent_cancels_busy_work_and_preserves_history_and_provider_records() {
    let worktree = crate::test_support::TestWorktree::new("agent-archive-history");
    let h = LocalRouterTestHarness::new();
    let LocalDaemonResponse::SessionCreated { session, .. } = h
        .dispatch(LocalDaemonRequest::CreateSession(
            worktree.session_request(),
        ))
        .unwrap()
    else {
        panic!("session")
    };
    let agent = h.spawn_workflow_test_agent(session.id(), "archive-me");
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
                client_id: "archive-test".into(),
                capability_level: ClientCapabilityLevel::FullTerminal,
            },
        ))
        .unwrap()
    else {
        panic!("attachment")
    };
    for text in ["first archived history", "queued archived history"] {
        h.dispatch(LocalDaemonRequest::SubmitPrompt(SubmitPromptRequest {
            session_id: session.id().into(),
            attachment_id: attachment.id().into(),
            target_agent_id: Some(agent.id().into()),
            prompt: text.into(),
            attachments: vec![],
        }))
        .unwrap();
    }
    h.dispatch(archive_request(session.id(), agent.id(), true))
        .expect("running and queued work should stop when archived");
    assert_eq!(
        h.with_app(|app| app.providers().get_run(&run_id).unwrap().state()),
        crate::provider::ProviderRunState::Ended
    );
    let history = h
        .dispatch(LocalDaemonRequest::GetSessionHistoryOutline(
            GetSessionHistoryOutlineRequest {
                session_id: session.id().into(),
                agent_ids: Some(vec![agent.id().into()]),
                latest_prompt_count: Some(4),
                cursor: None,
                include_prefetched_content: true,
            },
        ))
        .expect("archived history readable");
    assert!(serde_json::to_string(&history)
        .unwrap()
        .contains("first archived history"));
    assert_eq!(
        h.with_app(|app| app
            .providers()
            .get_run(&run_id)
            .unwrap()
            .agent_instance_id()
            .map(str::to_string)),
        Some(agent.id().to_string())
    );
    let submit = LocalDaemonRequest::SubmitPrompt(SubmitPromptRequest {
        session_id: session.id().into(),
        attachment_id: attachment.id().into(),
        target_agent_id: Some(agent.id().into()),
        prompt: "must not run".into(),
        attachments: vec![],
    });
    assert!(h.dispatch(submit).is_err());
    h.dispatch(archive_request(session.id(), agent.id(), false))
        .unwrap();
    h.dispatch(LocalDaemonRequest::DestroyAgent(DestroyAgentRequest {
        session_id: session.id().into(),
        agent_id: agent.id().into(),
    }))
    .unwrap();
}

#[test]
fn archive_agent_checks_membership_and_ownership_without_mutation() {
    let worktree = crate::test_support::TestWorktree::new("agent-archive-owner");
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
        .dispatch(archive_request("wrong-room", agent.id(), true))
        .is_err());
    assert!(h
        .dispatch_as_user(
            "other-user",
            archive_request(session.id(), agent.id(), true)
        )
        .is_err());
    let result = serde_json::to_value(
        h.dispatch(archive_request(session.id(), agent.id(), false))
            .unwrap(),
    )
    .unwrap();
    assert_eq!(result["AgentArchived"]["agent"]["archived"], false);
}

#[test]
fn archive_agent_state_survives_kernel_restart() {
    let worktree = crate::test_support::TestWorktree::new("agent-archive-restart");
    let config = DaemonConfig::for_tests();
    let (room, id) = {
        let h = LocalRouterTestHarness::with_config(config.clone());
        let LocalDaemonResponse::SessionCreated { session, .. } = h
            .dispatch(LocalDaemonRequest::CreateSession(
                worktree.session_request(),
            ))
            .unwrap()
        else {
            panic!("session")
        };
        let agent = h.spawn_workflow_test_agent(session.id(), "archive-history");
        h.launch_workflow_test_provider(session.id(), agent.id());
        h.with_app(|app| {
            let updated = app
                .agents_mut()
                .set_agent_provider_resume_state(
                    agent.id(),
                    crate::provider::ProviderResumeState::from_codex_thread_id(
                        "archive-preserved-thread",
                    ),
                )
                .unwrap();
            app.durable_state_store()
                .append_event(
                    "agent.updated",
                    Some(agent.id().into()),
                    serde_json::json!({"agent":updated}),
                )
                .unwrap();
        });
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
                    client_id: "archive-restart".into(),
                    capability_level: ClientCapabilityLevel::FullTerminal,
                },
            ))
            .unwrap()
        else {
            panic!("attachment")
        };
        h.dispatch(LocalDaemonRequest::SubmitPrompt(SubmitPromptRequest {
            session_id: session.id().into(),
            attachment_id: attachment.id().into(),
            target_agent_id: Some(agent.id().into()),
            prompt: "retained archive history".into(),
            attachments: vec![],
        }))
        .unwrap();
        h.dispatch(LocalDaemonRequest::CompletePrompt(CompletePromptRequest {
            session_id: session.id().into(),
        }))
        .unwrap();
        h.dispatch(archive_request(session.id(), agent.id(), true))
            .unwrap();
        assert_eq!(
            h.with_app(|app| app
                .providers()
                .get_run(&run_id)
                .unwrap()
                .agent_instance_id()
                .map(str::to_string)),
            Some(agent.id().to_string())
        );
        (session.id().to_string(), agent.id().to_string())
    };
    let h = LocalRouterTestHarness::with_config(config);
    let snapshot: LocalDaemonRequest =
        serde_json::from_value(serde_json::json!({"GetSessionState": {"session_id": room}}))
            .unwrap();
    let value = serde_json::to_value(h.dispatch(snapshot).unwrap()).unwrap();
    assert!(value["SessionState"]["session"]["agents"]
        .as_array()
        .unwrap()
        .iter()
        .any(|agent| agent["id"] == id && agent["archived"] == true));
    let history = h
        .dispatch(LocalDaemonRequest::GetSessionHistoryOutline(
            GetSessionHistoryOutlineRequest {
                session_id: room.clone(),
                agent_ids: Some(vec![id.clone()]),
                latest_prompt_count: Some(4),
                cursor: None,
                include_prefetched_content: true,
            },
        ))
        .unwrap();
    assert!(serde_json::to_string(&history)
        .unwrap()
        .contains("retained archive history"));
    assert_eq!(
        h.with_app(|app| app
            .agents()
            .get_agent(&id)
            .unwrap()
            .provider_resume_state()
            .codex_thread_id()
            .map(str::to_owned)),
        Some("archive-preserved-thread".into())
    );
    let restored =
        serde_json::to_value(h.dispatch(archive_request(&room, &id, false)).unwrap()).unwrap();
    assert_eq!(restored["AgentArchived"]["agent"]["archived"], false);
}

#[test]
fn archive_agent_protocol_592_shape_and_hash() {
    let value = serde_json::to_value(archive_request("room", "agent", true)).unwrap();
    assert_eq!(
        value,
        serde_json::json!({"ArchiveAgent": {"session_id":"room", "agent_id":"agent", "archived":true}})
    );
    let digest = format!("{:x}", Sha256::digest(serde_json::to_vec(&value).unwrap()));
    assert_eq!(
        digest,
        "195f2bbf47c8f0a17aec7f8a3f65f54fc3c56bad25bfcf21cba3cbad508d1955"
    );
}

fn assert_archive_agent_capacity_reused(batch: bool) {
    let worktree = crate::test_support::TestWorktree::new("archive-capacity");
    let mut config = crate::DaemonConfig::for_tests();
    config.user_config.workflow.session_default_max_agents = Some(2);
    let h = LocalRouterTestHarness::with_config(config);
    let LocalDaemonResponse::SessionCreated { session, agent, .. } = h
        .dispatch(LocalDaemonRequest::CreateSession(
            worktree.session_request(),
        ))
        .unwrap()
    else {
        panic!("session")
    };
    for index in 1..session.max_agents() {
        h.spawn_workflow_test_agent(session.id(), &format!("capacity-{index}"));
    }
    h.dispatch(archive_request(session.id(), agent.id(), true))
        .unwrap();
    if batch {
        let request = serde_json::from_value(serde_json::json!({"SpawnAgents": {
            "session_id": session.id(), "agents": [{"alias": "replacement", "provider": "codex"}]
        }}))
        .unwrap();
        h.dispatch(request)
            .expect("archived agent must release batch capacity");
    } else {
        h.spawn_workflow_test_agent(session.id(), "replacement");
    }
    let LocalDaemonResponse::AgentsListed { agents } = h
        .dispatch(LocalDaemonRequest::ListAgents(ListAgentsRequest {
            session_id: session.id().into(),
        }))
        .unwrap()
    else {
        panic!("agents")
    };
    assert_eq!(
        agents.iter().filter(|a| !a.archived()).count(),
        session.max_agents() as usize
    );
    assert!(
        h.dispatch(archive_request(session.id(), agent.id(), false))
            .is_err(),
        "unarchive still obeys active capacity"
    );
}

#[test]
fn archive_agent_releases_single_spawn_capacity() {
    assert_archive_agent_capacity_reused(false);
}
#[test]
fn archive_agent_releases_batch_spawn_capacity() {
    assert_archive_agent_capacity_reused(true);
}

fn assert_archive_agent_layout_after(operation: &str) {
    let worktree = crate::test_support::TestWorktree::new("archive-layout");
    let h = LocalRouterTestHarness::new();
    let LocalDaemonResponse::SessionCreated { session, agent, .. } = h
        .dispatch(LocalDaemonRequest::CreateSession(
            worktree.session_request(),
        ))
        .unwrap()
    else {
        panic!("session")
    };
    let visible = h.spawn_workflow_test_agent(session.id(), "visible");
    h.dispatch(archive_request(session.id(), agent.id(), true))
        .unwrap();
    match operation {
        "focus" => {
            h.dispatch(LocalDaemonRequest::FocusAgent(FocusAgentRequest {
                session_id: session.id().into(),
                agent_id: visible.id().into(),
            }))
            .unwrap();
        }
        "spawn" => {
            h.spawn_workflow_test_agent(session.id(), "new");
        }
        "delete" => {
            let temporary = h.spawn_workflow_test_agent(session.id(), "temporary");
            h.dispatch(LocalDaemonRequest::DestroyAgent(DestroyAgentRequest {
                session_id: session.id().into(),
                agent_id: temporary.id().into(),
            }))
            .unwrap();
        }
        _ => panic!("unknown operation"),
    }
    let LocalDaemonResponse::AgentsListed { agents } = h
        .dispatch(LocalDaemonRequest::ListAgents(ListAgentsRequest {
            session_id: session.id().into(),
        }))
        .unwrap()
    else {
        panic!("agents")
    };
    let positions = agents
        .iter()
        .filter(|a| !a.archived())
        .map(|a| a.position().clone())
        .collect::<Vec<_>>();
    assert_eq!(
        positions,
        crate::agent::calculate_agent_layout(positions.len()),
        "{operation} must not allocate hidden archived slots"
    );
}

#[test]
fn archive_agent_layout_stays_compact_after_focus() {
    assert_archive_agent_layout_after("focus");
}
#[test]
fn archive_agent_layout_stays_compact_after_spawn() {
    assert_archive_agent_layout_after("spawn");
}
#[test]
fn archive_agent_layout_stays_compact_after_delete() {
    assert_archive_agent_layout_after("delete");
}

#[test]
fn archive_agent_focus_cycle_skips_archived_agents() {
    let worktree = crate::test_support::TestWorktree::new("archive-cycle");
    let h = LocalRouterTestHarness::new();
    let LocalDaemonResponse::SessionCreated { session, agent, .. } = h
        .dispatch(LocalDaemonRequest::CreateSession(
            worktree.session_request(),
        ))
        .unwrap()
    else {
        panic!("session")
    };
    let hidden = h.spawn_workflow_test_agent(session.id(), "hidden");
    h.dispatch(archive_request(session.id(), hidden.id(), true))
        .unwrap();
    let LocalDaemonResponse::AgentFocusCycled { agent: focused } = h
        .dispatch(LocalDaemonRequest::CycleAgentFocus(
            CycleAgentFocusRequest {
                session_id: session.id().into(),
            },
        ))
        .expect("cycling one active agent must succeed")
    else {
        panic!("cycle")
    };
    assert_eq!(focused.unwrap().id(), agent.id());
    h.dispatch(archive_request(session.id(), agent.id(), true))
        .unwrap();
    let LocalDaemonResponse::AgentFocusCycled { agent: focused } = h
        .dispatch(LocalDaemonRequest::CycleAgentFocus(
            CycleAgentFocusRequest {
                session_id: session.id().into(),
            },
        ))
        .expect("cycling an archived-only Room must succeed")
    else {
        panic!("cycle")
    };
    assert!(focused.is_none());
}

#[test]
fn archived_agents_protocol_636_shape_and_hash() {
    let value = serde_json::json!({"ListArchivedAgents": {"session_id":"room"}});
    let request: LocalDaemonRequest = serde_json::from_value(value.clone()).unwrap();
    assert_eq!(serde_json::to_value(request).unwrap(), value);
    assert_eq!(
        format!("{:x}", Sha256::digest(serde_json::to_vec(&value).unwrap())),
        "10baa1309adeacd62815a797d64b96e234c13cfa94a41cae85ee00c88c17a5e1"
    );
    assert!(crate::local::LOCAL_DAEMON_PROTOCOL_VERSION >= 636);
    assert!(crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION >= 239);
}
