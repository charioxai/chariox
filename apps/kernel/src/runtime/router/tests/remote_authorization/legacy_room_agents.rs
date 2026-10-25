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
                attachment.id().to_string()
            } else {
                "unattached".to_string()
            };

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
