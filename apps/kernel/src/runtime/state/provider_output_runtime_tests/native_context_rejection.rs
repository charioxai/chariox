//! MP-08 / MP-10 / MP-11: hook rejection settles a prompt, preserving the live native provider.
use super::*;
use std::fs;

#[tokio::test]
async fn native_context_rejection_keeps_the_run_idle_and_allows_retry() {
    rejection_scenario(NativeContextScenario::default()).await;
}

#[tokio::test]
async fn native_context_rejection_survives_subscription_mailbox_interleaving() {
    rejection_scenario(NativeContextScenario {
        subscription: true,
        before_submit: true,
        ..Default::default()
    })
    .await;
}

#[tokio::test]
async fn native_context_rejection_notifies_metaagent_once() {
    rejection_scenario(NativeContextScenario {
        with_meta: true,
        ..Default::default()
    })
    .await;
}

#[tokio::test]
async fn native_context_rejection_survives_subscription_failure_interleaving() {
    rejection_scenario(NativeContextScenario {
        subscription: true,
        ..Default::default()
    })
    .await;
}

#[tokio::test]
async fn home_projection_drain_pumps_leased_native_hooks() {
    rejection_scenario(NativeContextScenario {
        leased: true,
        ..Default::default()
    })
    .await;
}

#[tokio::test]
async fn headless_native_prompt_fails_on_recent_terminal_error() {
    rejection_scenario(NativeContextScenario {
        recent_failure: true,
        ..Default::default()
    })
    .await;
}

#[tokio::test]
async fn home_projection_drain_leaves_hook_for_a_busy_provider_lane() {
    rejection_scenario(NativeContextScenario {
        leased: true,
        hold_provider_lane: true,
        ..Default::default()
    })
    .await;
}

#[tokio::test]
async fn native_context_rejection_does_not_drop_a_retry_in_the_same_batch() {
    rejection_scenario(NativeContextScenario {
        same_batch: true,
        ..Default::default()
    })
    .await;
}

#[tokio::test]
async fn expired_managed_hook_replaces_a_settled_previous_delivery() {
    rejection_scenario(NativeContextScenario {
        stale_expired_delivery: true,
        ..Default::default()
    })
    .await;
}

#[derive(Default)]
struct NativeContextScenario {
    subscription: bool,
    with_meta: bool,
    before_submit: bool,
    leased: bool,
    recent_failure: bool,
    hold_provider_lane: bool,
    same_batch: bool,
    stale_expired_delivery: bool,
}

async fn rejection_scenario(scenario: NativeContextScenario) {
    let NativeContextScenario {
        subscription,
        with_meta,
        before_submit,
        leased,
        recent_failure,
        hold_provider_lane,
        same_batch,
        stale_expired_delivery,
    } = scenario;
    let worktree = crate::test_support::TestWorktree::new("native-context-failure");
    let mut config = crate::config::DaemonConfig::for_tests();
    config.accept_remote_leases = leased;
    let mut app = DaemonApp::bootstrap(config).unwrap();
    let (mut session, mut agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(worktree.session_request())
        .unwrap();
    let leased_id = if leased {
        let mut worker = crate::app::RemoteLeaseRuntime::new(&mut app);
        let lease = worker
            .create_execution_lease(
                "home-kernel",
                "home-session",
                "home-agent",
                false,
                crate::session::DEFAULT_LOCAL_USER_ID,
            )
            .unwrap();
        let binding = worker
            .create_leased_agent_from_base_directory(
                worktree.path(),
                &lease.id,
                "claude",
                "default",
                Some("haiku".into()),
                None,
                None,
                None,
                None,
                None,
                None,
            )
            .unwrap();
        session = app
            .sessions()
            .get_session(&binding.backing_session_id)
            .unwrap();
        agent = app.agents().get_agent(&binding.backing_agent_id).unwrap();
        Some(binding.id)
    } else {
        None
    };
    let attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(crate::attachment::AttachRequest::new(
            session.id(),
            "context-client",
            crate::attachment::ClientCapabilityLevel::FullTerminal,
        ))
        .unwrap();
    let metaagent = if with_meta {
        let meta = crate::app::KernelSessionService::new(&mut app)
            .spawn_agent(
                crate::agent::CreateAgentRequest::new(session.id(), "codex").with_alias("meta"),
            )
            .unwrap();
        Some(
            app.agents_mut()
                .activate_agent_meta_mode(meta.id(), None)
                .unwrap(),
        )
    } else {
        None
    };
    let history = app.operational_history_store();
    history
        .append(&crate::history::HistoryEvent::transcript(
            1,
            &crate::history::SessionHistoryEntry::user_prompt(
                session.id(),
                attachment.id(),
                agent.id(),
                "remember amber-kestrel",
            ),
            crate::history::HistoryEventTurnContext {
                prompt_id: Some("old-prompt".into()),
                ..Default::default()
            },
        ))
        .unwrap();
    history
        .append(&crate::history::HistoryEvent::transcript(
            2,
            &crate::history::SessionHistoryEntry::provider_output(
                session.id(),
                "old-codex-run",
                Some(agent.id()),
                crate::terminal::TerminalOutputKind::ProviderOutput,
                None,
                "amber-kestrel recorded",
            ),
            crate::history::HistoryEventTurnContext {
                provider: Some("codex".into()),
                model: Some("gpt-6".into()),
                provider_run_id: Some("old-codex-run".into()),
                provider_session_id: Some("old-codex-thread".into()),
                prompt_id: Some("old-prompt".into()),
                ..Default::default()
            },
        ))
        .unwrap();
    let context = worktree.path().join("context.txt");
    let events = worktree.path().join("events.jsonl");
    fs::write(&context, "").unwrap();
    fs::create_dir(worktree.path().join("hook-context-responses")).unwrap();
    let request = crate::provider::LaunchProviderRequest::new(
        session.id(),
        "claude",
        if recent_failure {
            "claude-headless"
        } else {
            "claude"
        },
        "default",
        "haiku",
    )
    .with_agent_id(agent.id());
    let request = if recent_failure {
        request
    } else {
        request.with_client_interface(crate::provider::ProviderClientInterface::NativeTui)
    };
    let mut run = crate::provider::RuntimeProviderRun::new(
        "context-failure-run",
        &request,
        crate::provider::ProviderLaunchResult {
            endpoint_mode: crate::provider::AgentEndpointMode::Managed,
            process_label: "native-context-test".into(),
            pty_target: None,
            pty_program: None,
            pty_args: Vec::new(),
            pty_env: std::collections::BTreeMap::from([
                (
                    "CHARIOX_CLAUDE_NATIVE_CONTEXT".into(),
                    context.display().to_string(),
                ),
                (
                    "CHARIOX_CLAUDE_NATIVE_EVENTS".into(),
                    events.display().to_string(),
                ),
                ("CHARIOX_CLAUDE_NATIVE_CONTEXT_RECEIPTS".into(), "1".into()),
            ]),
            pty_env_remove: Vec::new(),
            working_directory: None,
            structured_endpoint: None,
        },
    );
    run.mark_running();
    app.providers_mut().insert_run_for_test(run.clone());
    app.update_provider_run_projection(run.clone());
    spawn_inert_pty_for_run(&mut app, run.id());
    let app = Arc::new(Mutex::new(app));
    let _cleanup = InertPtyCleanup {
        app: Arc::clone(&app),
        provider_run_id: run.id().into(),
    };
    let runtime = owned_runtime_state(&app).await;
    // Native hook prompts use ordinary account admission too. Authenticate only
    // this disposable fixture's Claude profile, never an ambient builder login.
    crate::test_support::authenticate_provider_account(
        &runtime.owned.provider_account_profiles,
        crate::session::DEFAULT_LOCAL_USER_ID,
        "claude",
        "default",
    )
    .unwrap();
    fs::write(&events, serde_json::json!({"hook_event_name":"UserPromptSubmit", "prompt":"first native request", "hook_context_request_id":"failed-request", "hook_context_deadline_ms":crate::session::unix_epoch_ms() + 5000}).to_string()).unwrap();
    if subscription && before_submit {
        let mut locked = app.lock().await;
        crate::runtime_transport::watch_subscription_state(
            &mut locked,
            session.id(),
            attachment.id(),
            false,
            None,
            0,
        );
        assert!(
            !fs::read_to_string(&events).unwrap().is_empty(),
            "subscription consumed the native hook mailbox"
        );
    }
    // Like the home, acknowledge each applied projection before draining again.
    let mut acknowledged = 0;
    if hold_provider_lane {
        let permit = runtime.provider_runtime_lanes.acquire(run.id()).await;
        if let Some(drained) = runtime
            .drain_relay_leased_runtime_projection(
                leased_id.as_ref().unwrap(),
                run.id(),
                true,
                false,
                false,
                acknowledged,
            )
            .await
            .unwrap()
        {
            acknowledged = drained.sequence;
        }
        assert!(
            fs::read_to_string(&context).unwrap().is_empty(),
            "a home drain consumed the native hook while another pump owned the provider lane"
        );
        assert!(
            !fs::read_to_string(&events).unwrap().is_empty(),
            "a busy provider lane must leave its hook mailbox for the lane owner"
        );
        drop(permit);
    }
    if let Some(leased_id) = &leased_id {
        runtime
            .drain_relay_leased_runtime_projection(
                leased_id,
                run.id(),
                true,
                false,
                false,
                acknowledged,
            )
            .await
            .unwrap();
    } else {
        runtime
            .pump_owned_provider_output(session.id(), run.id(), vec![], true)
            .await
            .unwrap();
    }
    let hidden = fs::read_to_string(&context).unwrap();
    assert!(
        hidden.contains("amber-kestrel"),
        "native prompt lost the conversation handoff"
    );
    if subscription {
        let mut locked = app.lock().await;
        crate::runtime_transport::watch_subscription_state(
            &mut locked,
            session.id(),
            attachment.id(),
            false,
            None,
            0,
        );
    }
    let original_prompt_id = runtime
        .owned
        .session_snapshot(session.id())
        .unwrap()
        .active_prompt_for_agent(agent.id())
        .expect("original prompt must be active")
        .id()
        .to_string();
    if stale_expired_delivery {
        let previous = runtime
            .owned
            .session_snapshot(session.id())
            .unwrap()
            .active_prompt_for_agent(agent.id())
            .unwrap()
            .clone();
        let delivery_path = format!("{}.context-delivery", context.display());
        let old_delivery = fs::read(&delivery_path).unwrap();
        runtime
            .owned
            .cancel_active_prompt_only(session.id(), agent.id())
            .unwrap();
        // Reproduce a late binding left by the settled previous hook. No provider
        // cancellation contract is implemented by this expiry regression.
        fs::write(&delivery_path, old_delivery).unwrap();
        let admitted = {
            let mut locked = app.lock().await;
            match locked
                .record_native_prompt_started_with_attachments(
                    session.id(),
                    attachment.id(),
                    attachment.id(),
                    agent.id(),
                    "expired managed retry",
                    Vec::new(),
                )
                .unwrap()
            {
                crate::session::PromptSubmissionOutcome::Started { prompt } => prompt,
                other => panic!("unexpected current admission: {other:?}"),
            }
        };
        assert_ne!(previous.id(), admitted.id());
        fs::write(
            worktree.path().join("active-prompt-id"),
            format!(
                "submit-wait:{}:{}",
                admitted.id(),
                crate::session::unix_epoch_ms()
            ),
        )
        .unwrap();
        fs::write(worktree.path().join("headless-submit-retry.json"), serde_json::json!({
            "prompt_id":admitted.id(), "count":0, "last_attempt_ms":crate::session::unix_epoch_ms(),
            "visible_prompt":"expired managed retry"
        }).to_string()).unwrap();
        fs::write(
            &events,
            format!(
                "{}\n{}",
                serde_json::json!({
                    "hook_event_name":"UserPromptSubmit", "prompt":"expired managed retry",
                    "hook_context_request_id":"expired-current-request",
                    "hook_context_deadline_ms":crate::session::unix_epoch_ms().saturating_sub(1)
                }),
                serde_json::json!({"hook_event_name":"CharioxContextFailed",
            "hook_context_request_id":"expired-current-request"})
            ),
        )
        .unwrap();
        runtime
            .pump_owned_provider_output(session.id(), run.id(), vec![], true)
            .await
            .unwrap();
        assert!(
            runtime
                .owned
                .session_snapshot(session.id())
                .unwrap()
                .active_prompt_for_agent(agent.id())
                .is_none(),
            "obsolete delivery blocked rejection of the new admitted expired prompt"
        );
        assert_eq!(
            runtime
                .owned
                .provider_store
                .get_run(run.id())
                .unwrap()
                .state(),
            crate::provider::ProviderRunState::Running,
            "hook rejection retired a healthy provider"
        );
        assert!(!std::path::Path::new(&delivery_path).exists());
        return;
    }
    if recent_failure {
        // The visible error arrived in earlier PTY chunks; this poll has no new bytes.
        fs::write(
            worktree.path().join("permission-recent.txt"),
            "Credit balance is too low",
        )
        .unwrap();
    } else {
        let rejection = serde_json::json!({"hook_event_name":"CharioxContextFailed", "hook_context_request_id":"failed-request"});
        let retry = serde_json::json!({"hook_event_name":"UserPromptSubmit", "prompt":"retry native request", "hook_context_request_id":"retry-request", "hook_context_deadline_ms":crate::session::unix_epoch_ms() + 5000});
        let rows = if same_batch {
            format!("{rejection}\n{retry}\n")
        } else {
            rejection.to_string()
        };
        fs::write(&events, rows).unwrap();
    }
    if subscription {
        let mut locked = app.lock().await;
        crate::runtime_transport::watch_subscription_state(
            &mut locked,
            session.id(),
            attachment.id(),
            false,
            None,
            0,
        );
        assert!(
            !fs::read_to_string(&events).unwrap().is_empty(),
            "subscription consumed the native hook mailbox"
        );
    }
    runtime
        .pump_owned_provider_output(session.id(), run.id(), vec![], true)
        .await
        .unwrap();
    assert_eq!(
        runtime
            .owned
            .provider_store
            .get_run(run.id())
            .unwrap()
            .state(),
        if recent_failure {
            crate::provider::ProviderRunState::Ended
        } else {
            crate::provider::ProviderRunState::Running
        },
        "native failure must settle without confusing hook rejection with a provider failure"
    );
    if same_batch {
        let snapshot = runtime.owned.session_snapshot(session.id()).unwrap();
        let retry = snapshot
            .active_prompt_for_agent(agent.id())
            .expect("later retry row was dropped");
        assert_ne!(
            retry.id(),
            original_prompt_id,
            "the rejected original prompt remained active"
        );
        assert_eq!(retry.prompt(), "retry native request");
        return;
    }
    assert!(runtime
        .owned
        .session_snapshot(session.id())
        .unwrap()
        .active_prompt_for_agent(agent.id())
        .is_none());
    assert_eq!(
        runtime
            .owned
            .agent_store
            .get_agent(agent.id())
            .unwrap()
            .state(),
        if recent_failure {
            crate::agent::AgentState::Error
        } else {
            crate::agent::AgentState::Idle
        }
    );
    if recent_failure {
        return;
    }
    if let Some(meta) = &metaagent {
        let failures =
            runtime
                .owned
                .metaagent_events
                .list(meta.id(), Some("agent.turn.failed"), None, 10);
        assert_eq!(
            failures.len(),
            1,
            "hook rejection must notify the Meta agent"
        );
        assert_eq!(failures[0].source_agent_id.as_deref(), Some(agent.id()));
        assert!(failures[0].summary.contains("context delivery failed"));
        // A duplicated receipt must not emit another failure.
        let rejection = serde_json::json!({"hook_event_name":"CharioxContextFailed", "hook_context_request_id":"failed-request"});
        let retry = serde_json::json!({"hook_event_name":"UserPromptSubmit", "prompt":"retry native request", "hook_context_request_id":"retry-request", "hook_context_deadline_ms":crate::session::unix_epoch_ms() + 5000});
        let rows = if same_batch {
            format!("{rejection}\n{retry}\n")
        } else {
            rejection.to_string()
        };
        fs::write(&events, rows).unwrap();
        runtime
            .pump_owned_provider_output(session.id(), run.id(), vec![], true)
            .await
            .unwrap();
        assert_eq!(
            runtime
                .owned
                .metaagent_events
                .list(meta.id(), Some("agent.turn.failed"), None, 10)
                .len(),
            1
        );
    }
    fs::write(&events, serde_json::json!({"hook_event_name":"UserPromptSubmit", "prompt":"retry native request", "hook_context_request_id":"retry-request", "hook_context_deadline_ms":crate::session::unix_epoch_ms() + 5000}).to_string()).unwrap();
    runtime
        .pump_owned_provider_output(session.id(), run.id(), vec![], true)
        .await
        .unwrap();
    assert!(runtime
        .owned
        .session_snapshot(session.id())
        .unwrap()
        .active_prompt_for_agent(agent.id())
        .is_some());
}
