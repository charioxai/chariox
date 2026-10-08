//! MP-08/MP-11: Meta delegation cannot approve the source owner's copy review.
use super::*;

#[test]
fn mp08_mp11_kernel_only_owner_review_rejects_meta_and_agent_answers() {
    crate::test_support::isolated_env_test!();
    run_large_stack_async_test("kernel-only-human-review", kernel_only_human_review);
}

// MP-08/MP-11: the Project branch must enforce the same owner-only rule.
#[test]
fn mp08_mp11_source_project_owner_review_rejects_meta_and_agent_answers() {
    crate::test_support::isolated_env_test!();
    run_large_stack_async_test("project-human-review", source_project_human_review);
}

async fn kernel_only_human_review() {
    human_review(false, false).await;
}

async fn source_project_human_review() {
    human_review(true, false).await;
}

// MP-08/MP-11: create the Project/session before linking a distinct Cloud owner.
#[test]
fn mp08_mp11_legacy_local_kernel_review_uses_enrolled_terminal_owner() {
    crate::test_support::isolated_env_test!();
    run_large_stack_async_test("legacy-kernel-human-review", || human_review(false, true));
}

#[test]
fn mp08_mp11_legacy_local_project_review_uses_enrolled_terminal_owner() {
    crate::test_support::isolated_env_test!();
    run_large_stack_async_test("legacy-project-human-review", || human_review(true, true));
}

async fn human_review(source_project: bool, legacy_local: bool) {
    let env = TestMetaRuntimeEnv::new("kernel-only-human-review");
    let worktree = crate::test_support::TestWorktree::new("kernel-only-human-review");
    let mut config = DaemonConfig::for_tests();
    config.user_config.state.path = Some(env.root.join("state.db").display().to_string());
    config.user_config_path = env.root.join("config.toml");
    config = config.with_session_history_root(env.root.join("sessions"));
    let mut app = crate::test_support::bootstrap_authenticated_app(config).unwrap();
    let (session, worker) = crate::app::KernelSessionService::new(&mut app)
        .create_session(worktree.session_request())
        .unwrap();
    let meta = crate::app::KernelSessionService::new(&mut app)
        .spawn_agent(CreateAgentRequest::new(session.id(), "dev-stub").with_alias("meta"))
        .unwrap();
    let meta = activate_test_agent_meta_mode(&mut app, meta);
    mark_test_agent_controlled_by_metaagent(&mut app, worker.id(), meta.id());
    let run = launch_test_provider(
        &mut app,
        session.id(),
        meta.id(),
        "dev-stub",
        "dev-stub",
        "meta-model",
    );
    let auth = run.runtime_mcp_auth_token().unwrap().to_owned();
    let projection = app.config_projection_store();
    let mut config = projection.snapshot();
    let owner = if legacy_local {
        assert_eq!(
            session.owner_user_id(),
            crate::session::DEFAULT_LOCAL_USER_ID
        );
        assert_eq!(
            app.sessions()
                .get_project(session.project_id())
                .unwrap()
                .owner_user_id(),
            session.owner_user_id()
        );
        "enrolled-review-owner"
    } else {
        session.owner_user_id()
    };
    config.cloud_relay = Some(crate::config::PersistedCloudRelayProfile {
        user_id: owner.into(),
        ..Default::default()
    });
    projection.update(config);
    let router = CommandRouter::with_interactive_capacity(Arc::new(Mutex::new(app)), 4);
    let runtime = router.runtime_state();

    for choice in ["cancel", "continue"] {
        let context = format!("human-only-{choice}");
        let development = if source_project {
            crate::managed_context::package::ManagedContextDevelopmentSelection::SourceProject {
                project_id: session.project_id().into(),
                repositories: vec![
                    crate::managed_context::development::DevelopmentSourceRepositoryBinding {
                        role:
                            crate::managed_context::development::DevelopmentRepositoryRole::Primary,
                        workspace_id: session.workspace_id().into(),
                        worktree_id: None,
                    },
                ],
            }
        } else {
            crate::managed_context::package::ManagedContextDevelopmentSelection::Empty
        };
        let reviewer = runtime.clone();
        let task = tokio::spawn(async move {
            reviewer
                .review_credential_free_owner_context(
                    &development,
                    &context,
                    "Owner target machine",
                )
                .await
        });
        let interaction = tokio::time::timeout(std::time::Duration::from_secs(10), async {
            loop {
                let snapshot = runtime.session_snapshot(session.id()).await.unwrap();
                if let Some(review) = snapshot.active_interactions().iter().find(|i| {
                    i.id().starts_with("owner-context:")
                        || i.id().starts_with("project-environment:")
                }) {
                    break review.clone();
                }
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        let id = interaction.id().to_string();
        let attempted = router
            .dispatch_authenticated_runtime_tool_call(
                &auth,
                crate::transport::runtime_tools::META_RESOLVE_RUNTIME_INTERACTION_TOOL,
                serde_json::json!({"interaction_id": id, "choice_id": "continue"}),
            )
            .await
            .unwrap();
        assert!(
            !attempted.ok,
            "MP-11 Meta approval bypassed owner review: {:?}",
            attempted.payload
        );
        if source_project {
            assert_eq!(interaction.agent_id(), Some(worker.id()));
            assert!(
                serde_json::to_value(&interaction).unwrap()["project_environment_review"]
                    .is_object()
            );
        } else {
            assert!(interaction.agent_id().is_none());
            assert!(interaction.kernel_operation_id().is_some());
        }
        assert!(runtime
            .resolve_runtime_interaction(session.id(), &id, "continue", None)
            .await
            .is_err());
        assert!(runtime
            .resolve_terminal_runtime_interaction(
                session.id(),
                &id,
                "continue",
                None,
                Some("another-owner"),
            )
            .await
            .is_err());
        for class in [
            None,
            Some(crate::local::KernelConnectionClass::Host),
            Some(crate::local::KernelConnectionClass::KernelAgent),
        ] {
            assert!(
                runtime
                    .answer_terminal_runtime_interaction(
                        session.id(),
                        &id,
                        "continue",
                        None,
                        Some(owner),
                        None,
                        None,
                        class,
                    )
                    .await
                    .is_err(),
                "MP-11 nonterminal owner identity must not approve review"
            );
        }
        assert!(
            !task.is_finished(),
            "MP-11 rejected replies must leave review pending"
        );
        let events = router
            .dispatch_authenticated_runtime_tool_call(
                &auth,
                crate::transport::runtime_tools::META_LIST_EVENTS_TOOL,
                serde_json::json!({"kind": "runtime.interaction"}),
            )
            .await
            .unwrap();
        assert!(events.ok);
        assert!(
            events.payload["events"].as_array().unwrap().is_empty(),
            "MP-11 owner review must not enter Meta prompt/event context"
        );
        if legacy_local {
            let enrolled = projection.snapshot();
            let mut relinked = enrolled.clone();
            relinked.cloud_relay.as_mut().unwrap().user_id = "relinked-owner".into();
            projection.update(relinked);
            for caller in [owner, "relinked-owner"] {
                assert!(
                    runtime
                        .resolve_terminal_runtime_interaction(
                            session.id(),
                            &id,
                            "continue",
                            None,
                            Some(caller),
                        )
                        .await
                        .is_err(),
                    "MP-11 a relink cannot reassign a pending review"
                );
            }
            projection.update(enrolled);
        }
        if legacy_local {
            assert!(
                runtime
                    .resolve_terminal_runtime_interaction(
                        session.id(),
                        &id,
                        "continue",
                        None,
                        Some(session.owner_user_id()),
                    )
                    .await
                    .is_err(),
                "MP-11 literal local identity cannot answer after enrollment"
            );
        }
        let request =
            LocalDaemonRequest::RespondToInteraction(crate::local::RespondToInteractionRequest {
                session_id: session.id().into(),
                interaction_id: id,
                choice_id: choice.into(),
                custom_reply: None,
                passkey: None,
                passkey_remember_minutes: None,
            });
        let mut command =
            KernelCommand::from_local_request("owner-review-answer", None, None, &request);
        command.caller = router
            .local_command_caller(
                crate::runtime::command::KernelCommandSource::LocalCli,
                crate::local::KernelConnectionClass::Terminal,
            )
            .await;
        assert_eq!(command.caller.user_id.as_deref(), Some(owner));
        if legacy_local {
            let observation =
                LocalDaemonRequest::GetSessionState(crate::local::GetSessionStateRequest {
                    session_id: session.id().into(),
                });
            let observe = KernelCommand::from_local_request_with_caller(
                "legacy-observation",
                command.source.clone(),
                command.caller.clone(),
                None,
                None,
                &observation,
            );
            let projected = router.dispatch(observe, observation).await.unwrap();
            if let LocalDaemonResponse::SessionState {
                session: projected, ..
            } = projected
            {
                assert_eq!(
                    projected.focused_agent_id(),
                    Some(meta.id()),
                    "MP-11 the enrolled terminal retains the current local session focus"
                );
            } else {
                panic!("MP-11 expected legacy session projection");
            }
            let mut host = command.clone();
            host.caller.connection_class = Some(crate::local::KernelConnectionClass::Host);
            assert!(
                matches!(
                    router.dispatch(host, request.clone()).await,
                    Err(DaemonError::SessionAccessDenied { .. })
                ),
                "MP-11 legacy terminal ownership never grants Host membership"
            );
        }
        assert!(matches!(
            router.dispatch(command, request).await.unwrap(),
            LocalDaemonResponse::InteractionResponded { .. }
        ));
        let result = tokio::time::timeout(std::time::Duration::from_secs(3), task)
            .await
            .unwrap()
            .unwrap();
        if choice == "continue" {
            result.unwrap();
        } else {
            assert!(matches!(
                result,
                Err(DaemonError::ManagedContext {
                    code: "managed_context_review_cancelled",
                    retryable: false,
                    ..
                })
            ));
        }
    }
}
