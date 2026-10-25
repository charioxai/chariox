//! MP-08 / MP-10 / MP-11 F4: the enrolled owner administers every agent of its pre-enrollment
//! room, whether it was created under the `local` identity or under its Cloud identity.
use super::historical_workflows::terminal_command;
use super::*;

#[test]
fn mp11_f4_enrolled_owner_administers_local_and_cloud_owned_agents() {
    run_remote_authorization_large_stack_test("f4-legacy-agents", f4_legacy_agents_inner);
}

async fn f4_legacy_agents_inner() {
    let root = crate::test_support::TestWorktree::new("f4-legacy-agents");
    let mut config = DaemonConfig::for_tests();
    config.cloud_relay = Some(crate::config::PersistedCloudRelayProfile {
        user_id: "kernel-owner".into(),
        realm_id: "test-realm".into(),
        ..Default::default()
    });
    let mut app = DaemonApp::bootstrap(config).unwrap();
    let id = app
        .sessions_mut()
        .create_session(root.session_request())
        .unwrap()
        .id()
        .to_string();
    assert_eq!(
        app.session_state_store()
            .get_session(&id)
            .unwrap()
            .owner_user_id(),
        DEFAULT_LOCAL_USER_ID
    );
    let mut agent_ids = Vec::new();
    for (alias, owner) in [
        ("local-made", DEFAULT_LOCAL_USER_ID),
        ("cloud-made", "kernel-owner"),
    ] {
        let agent = crate::app::KernelSessionService::new(&mut app)
            .spawn_agent(
                CreateAgentRequest::new(&id, "dev-stub")
                    .with_alias(alias)
                    .with_owner_user_id(owner),
            )
            .unwrap();
        assert_eq!(agent.owner_user_id(), owner);
        agent_ids.push(agent.id().to_string());
    }
    let router = CommandRouter::with_interactive_capacity(Arc::new(Mutex::new(app)), 4);

    for user in ["kernel-owner", "stranger"] {
        let owner = user == "kernel-owner";
        let attach = LocalDaemonRequest::AttachToSession(crate::local::AttachToSessionRequest {
            session_id: id.clone(),
            client_id: format!("client-{user}"),
            capability_level: crate::attachment::ClientCapabilityLevel::FullTerminal,
        });
        let attached = router
            .dispatch(terminal_command(&attach, Some(user)), attach)
            .await;
        assert_eq!(attached.is_ok(), owner, "attach as {user}: {attached:?}");
        let attachment_id =
            if let Ok(LocalDaemonResponse::SessionAttached { attachment }) = attached {
                // The attachment belongs to the room member the caller is, so the pushed room
                // snapshots show the owner everything its room's `local` member sees.
                assert_eq!(attachment.owner_user_id(), DEFAULT_LOCAL_USER_ID);
                attachment.id().to_string()
            } else {
                "unattached".to_string()
            };
        if owner {
            let state = LocalDaemonRequest::GetSessionState(crate::local::GetSessionStateRequest {
                session_id: id.clone(),
            });
            let Ok(LocalDaemonResponse::SessionState { session, .. }) = router
                .dispatch(terminal_command(&state, Some(user)), state)
                .await
            else {
                panic!("session state as {user}");
            };
            assert_eq!(
                session.agents().len(),
                agent_ids.len(),
                "the owner sees the local-made and the Cloud-made agent"
            );
        }

        if owner {
            let spawn = LocalDaemonRequest::SpawnAgent(crate::local::SpawnAgentRequest {
                session_id: id.clone(),
                alias: Some("spawned-by-owner".into()),
                provider: Some("dev-stub".into()),
                account_profile: None,
                model: None,
                effort: None,
                execution_mode: None,
                permission_level: None,
                worktree_id: None,
                kernel_ref: None,
                slice_ref: None,
                worktree_placement: None,
                metaagent: false,
            });
            let Ok(LocalDaemonResponse::AgentSpawned { agent, .. }) = router
                .dispatch(terminal_command(&spawn, Some(user)), spawn)
                .await
            else {
                panic!("spawn as {user}");
            };
            // The room's own member owns what the owner makes there, so it is shown to the
            // owner like the rest of the room's agents.
            assert_eq!(agent.owner_user_id(), DEFAULT_LOCAL_USER_ID);
        }

        for agent_id in &agent_ids {
            let requests = [
                (
                    "update profile",
                    LocalDaemonRequest::UpdateAgentProfile(
                        crate::local::UpdateAgentProfileRequest {
                            session_id: id.clone(),
                            agent_id: agent_id.clone(),
                            provider: None,
                            account_profile: None,
                            model: Some("dev-model".into()),
                            effort: None,
                            clear_effort: false,
                        },
                    ),
                ),
                (
                    "update config",
                    LocalDaemonRequest::UpdateAgentConfig(crate::local::UpdateAgentConfigRequest {
                        session_id: id.clone(),
                        agent_id: agent_id.clone(),
                        execution_mode: None,
                        clear_execution_mode: true,
                        permission_level: None,
                        clear_permission_level: false,
                        workspace_id: None,
                        clear_workspace_id: false,
                        worktree_id: None,
                        clear_worktree_id: false,
                    }),
                ),
                (
                    "alias",
                    LocalDaemonRequest::AliasAgent(crate::local::AliasAgentRequest {
                        session_id: id.clone(),
                        agent_id: agent_id.clone(),
                        alias: format!("renamed-{user}-{}", &agent_id[agent_id.len() - 6..]),
                    }),
                ),
                (
                    "substitutes",
                    LocalDaemonRequest::UpdateAgentSubstitutes(
                        crate::local::UpdateAgentSubstitutesRequest {
                            session_id: id.clone(),
                            agent_id: agent_id.clone(),
                            action: crate::local::AgentSubstituteAction::SetTimeout {
                                timeout_ms: Some(60_000),
                            },
                        },
                    ),
                ),
                (
                    "prompt",
                    LocalDaemonRequest::SubmitPrompt(SubmitPromptRequest {
                        session_id: id.clone(),
                        attachment_id: attachment_id.clone(),
                        target_agent_id: Some(agent_id.clone()),
                        prompt: "Reply with the single word ok.".into(),
                        attachments: Vec::new(),
                    }),
                ),
            ];
            for (operation, request) in requests {
                let mut command = terminal_command(&request, Some(user));
                command.command_id = format!("{operation}-{user}-{agent_id}");
                let result = router.dispatch(command, request).await;
                assert_eq!(
                    result.is_ok(),
                    owner,
                    "{operation} on `{agent_id}` as {user}: {result:?}"
                );
            }
        }
    }
}

// A decision bound to the room's `local` owner (an agent's blocked task) or to the enrolled
// owner's Cloud identity is answered by the enrolled owner, and by nobody else.
#[test]
fn mp11_f4_enrolled_owner_answers_decisions_of_its_pre_enrollment_room() {
    run_remote_authorization_large_stack_test("f4-legacy-decisions", f4_legacy_decisions_inner);
}

async fn f4_legacy_decisions_inner() {
    let root = crate::test_support::TestWorktree::new("f4-legacy-decisions");
    let mut config = DaemonConfig::for_tests();
    config.cloud_relay = Some(crate::config::PersistedCloudRelayProfile {
        user_id: "kernel-owner".into(),
        realm_id: "test-realm".into(),
        ..Default::default()
    });
    let app = DaemonApp::bootstrap(config).unwrap();
    let id = app
        .sessions_mut()
        .create_session(root.session_request())
        .unwrap()
        .id()
        .to_string();
    let router = CommandRouter::with_interactive_capacity(Arc::new(Mutex::new(app)), 4);

    for (decision_owner, user) in [
        (DEFAULT_LOCAL_USER_ID, "stranger"),
        (DEFAULT_LOCAL_USER_ID, "kernel-owner"),
        ("kernel-owner", "stranger"),
        ("kernel-owner", "kernel-owner"),
    ] {
        let answered_by_owner = user == "kernel-owner";
        let interaction_id = format!("decision-{decision_owner}-{user}");
        let interaction = crate::session::RuntimeInteraction::for_kernel_operation(
            &interaction_id,
            &interaction_id,
            "Agent needs your action",
            "blocked",
            vec![
                crate::session::RuntimeInteractionChoice::new("cancel", "Cancel", "cancel", None),
                crate::session::RuntimeInteractionChoice::new("resume", "Resume", "resume", None),
            ],
        )
        .with_timeout_sec(900);
        let _pending = router
            .runtime_state
            .create_kernel_operation_interaction(&id, decision_owner, interaction)
            .await
            .unwrap();
        let answer =
            LocalDaemonRequest::RespondToInteraction(crate::local::RespondToInteractionRequest {
                session_id: id.clone(),
                interaction_id: interaction_id.clone(),
                choice_id: "cancel".into(),
                custom_reply: None,
                passkey: None,
                passkey_remember_minutes: None,
            });
        let mut command = terminal_command(&answer, Some(user));
        command.command_id = interaction_id.clone();
        let result = router.dispatch(command, answer).await;
        assert_eq!(
            result.is_ok(),
            answered_by_owner,
            "{user} answers a decision bound to {decision_owner}: {result:?}"
        );
    }
}
