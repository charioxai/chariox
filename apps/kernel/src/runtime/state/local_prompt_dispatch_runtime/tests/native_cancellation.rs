// MP-08 / MP-10 / MP-11: runtime native cancellation before acknowledgement.
use super::*;

#[tokio::test]
async fn runtime_native_cancel_clears_only_its_unacknowledged_composer() {
    native_cancel_composer_fixture(false, false).await;
}

#[tokio::test]
async fn runtime_native_cancel_keeps_composer_ownership_after_settlement() {
    native_cancel_composer_fixture(true, false).await;
}

#[tokio::test]
async fn settled_native_abort_never_cancels_a_successor() {
    native_cancel_composer_fixture(true, true).await;
}

async fn native_cancel_composer_fixture(settles_before_abort: bool, successor_before_abort: bool) {
    for (marker, expected_input) in [
        ("injected:cancel-native", b"\x03\x15".as_slice()),
        ("accepted:cancel-native", b"\x03".as_slice()),
        ("injected:another-prompt", b"\x03".as_slice()),
    ] {
        let mut app = DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests()).unwrap();
        let (session, agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(crate::session::CreateSessionRequest::new(
                "native-cancel-workspace",
                "native-cancel-worktree",
            ))
            .unwrap();
        let attachment = crate::app::KernelSessionService::new(&mut app)
            .attach(crate::attachment::AttachRequest::new(
                session.id(),
                "native-cancel-client",
                crate::attachment::ClientCapabilityLevel::FullTerminal,
            ))
            .unwrap();
        let root = std::env::temp_dir().join(format!(
            "chariox-native-cancel-{}-{}",
            std::process::id(),
            crate::session::unix_epoch_ms()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let context_file = root.join("hidden-context.txt");
        std::fs::write(&context_file, "").unwrap();
        let request = crate::provider::LaunchProviderRequest::new(
            session.id(),
            "claude",
            "claude",
            "default",
            "sonnet",
        )
        .with_agent_id(agent.id())
        .with_client_interface(crate::provider::ProviderClientInterface::NativeTui);
        let mut run = crate::provider::RuntimeProviderRun::new(
            "native-cancel-run",
            &request,
            crate::provider::ProviderLaunchResult {
                endpoint_mode: crate::provider::AgentEndpointMode::Managed,
                process_label: "native-cancel-test".to_string(),
                pty_target: Some("native-cancel-pty".to_string()),
                pty_program: Some("/bin/sh".to_string()),
                pty_args: vec!["-c".to_string(), "trap '' INT; exec cat".to_string()],
                pty_env: std::collections::BTreeMap::from([(
                    "CHARIOX_CLAUDE_NATIVE_CONTEXT".to_string(),
                    context_file.display().to_string(),
                )]),
                pty_env_remove: Vec::new(),
                working_directory: None,
                structured_endpoint: None,
            },
        );
        run.mark_running();
        app.providers_mut().insert_run_for_test(run.clone());
        app.sessions
            .set_active_provider_run(session.id(), Some(run.id().to_string()))
            .unwrap();
        app.pty_mut().spawn_for_run(&run).unwrap();
        crate::app::ProviderProcessTracker::new(&mut app)
            .register_managed_run(&run)
            .unwrap();
        let context = context_file.to_str().unwrap();
        let mut prompt = crate::session::PromptQueueItem::new(
            "cancel-native",
            attachment.id(),
            agent.id(),
            "managed audit prompt",
            crate::session::PromptStatus::Dispatching,
        );
        prompt.set_durable_delivery(
            crate::session::DurablePromptDeliveryPhase::Dispatching,
            Some(run.id().to_string()),
            None,
        );
        app.prompt_owner_submit_prepared_prompt(session.id(), prompt, false)
            .unwrap();
        let prompt_id = app
            .prompt_owner_active_prompt_for_agent(session.id(), agent.id())
            .unwrap()
            .unwrap()
            .id()
            .to_string();
        let marker = match marker {
            "injected:cancel-native" => format!("injected:{prompt_id}"),
            "accepted:cancel-native" => format!("accepted:{prompt_id}"),
            foreign => foreign.to_string(),
        };
        std::fs::write(
            std::path::Path::new(context).with_file_name("active-prompt-id"),
            &marker,
        )
        .unwrap();
        std::fs::write(std::path::Path::new(context).with_file_name("headless-submit-retry.json"), serde_json::to_vec(&serde_json::json!({
            "prompt_id": prompt_id, "count":0, "last_attempt_ms":crate::session::unix_epoch_ms(), "visible_prompt":"managed audit prompt"
        })).unwrap()).unwrap();
        let app = Arc::new(Mutex::new(app));
        let runtime = owned_runtime_state(&app).await;
        let cancellation = runtime
            .owned
            .cancel_local_prompt(session.id(), agent.id(), attachment.id())
            .unwrap()
            .unwrap();
        if settles_before_abort {
            runtime
                .owned
                .finalize_local_prompt_cancellation_with_queued_advance(
                    session.id(),
                    agent.id(),
                    &prompt_id,
                    Some(run.id()),
                )
                .unwrap();
        }
        if successor_before_abort {
            app.lock()
                .await
                .prompt_owner_submit_prepared_prompt(
                    session.id(),
                    crate::session::PromptQueueItem::new(
                        "native-successor",
                        attachment.id(),
                        agent.id(),
                        "successor audit prompt",
                        crate::session::PromptStatus::Dispatching,
                    ),
                    false,
                )
                .unwrap();
        }
        let result = runtime
            .enqueue_prompt_abort(&cancellation.dispatch.unwrap())
            .await;
        let input = runtime
            .owned
            .terminal_stream
            .input_records()
            .last()
            .map(|record| record.bytes.clone());
        if successor_before_abort {
            assert_eq!(
                runtime
                    .owned
                    .prompt_state_owner
                    .active_prompt_for_agent(
                        &runtime
                            .owned
                            .session_store
                            .get_session(session.id())
                            .unwrap(),
                        agent.id(),
                    )
                    .unwrap()
                    .prompt(),
                "successor audit prompt"
            );
        }
        {
            let mut app = app.lock().await;
            crate::app::ProviderProcessTracker::new(&mut app)
                .remove_run(run.id())
                .unwrap();
        }
        std::fs::remove_dir_all(root).unwrap();
        result.unwrap();
        assert_eq!(
            input.as_deref(),
            if successor_before_abort {
                None
            } else {
                Some(expected_input)
            },
            "marker={marker}"
        );
    }
}

#[tokio::test]
async fn runtime_native_cancel_releases_the_acknowledgement_wait() {
    crate::test_support::isolated_env_test!();
    let (_worktree, runtime, session_id, agent_id, source_id, run, dispatch) =
        runtime_with_claude_headless_active_prompt().await;
    // The retirement fixture normally creates metadata only. This test
    // needs a live fake PTY for the native dispatch and abort paths.
    runtime
        .with_app_side_effect(|app| {
            crate::app::ProviderLaunchProcessRuntime::new(app).spawn_for_launch(&run)
        })
        .await
        .unwrap();
    let prompt_id = dispatch.prompt_id.clone();
    let marker = std::path::Path::new(&run.pty_env()["CHARIOX_CLAUDE_NATIVE_CONTEXT"])
        .with_file_name("active-prompt-id");
    // Seed the state after PTY injection, before UserPromptSubmit. This
    // fixture exercises acknowledgement/cancellation, not cold-start UI.
    std::fs::write(&marker, format!("injected:{prompt_id}")).unwrap();
    let origin = std::path::PathBuf::from(format!(
        "{}.approval-origin",
        run.pty_env()["CHARIOX_CLAUDE_NATIVE_CONTEXT"]
    ));
    runtime.spawn_prompt_dispatch(dispatch, runtime.provider_runtime_lanes.clone());
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            if std::fs::read_to_string(&origin).ok().as_deref() == Some(prompt_id.as_str()) {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap_or_else(|_| {
        panic!(
            "dispatch must reach the native acknowledgement wait; failed={:?}",
            failed_requests(&runtime, &agent_id)
        )
    });
    let cancellation = runtime
        .owned
        .cancel_local_prompt(&session_id, &agent_id, &source_id)
        .unwrap()
        .unwrap();
    assert_eq!(
        cancellation.cancellation.prompt.status(),
        PromptStatus::Cancelling
    );
    runtime.spawn_prompt_abort(
        cancellation.dispatch.unwrap(),
        runtime.provider_runtime_lanes.clone(),
    );
    // Let the native dispatch retry while cancellation owns the prompt,
    // before simulating Claude's delayed UserPromptSubmit acknowledgement.
    tokio::time::sleep(std::time::Duration::from_millis(250)).await;
    let settled = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            let session = runtime
                .owned
                .session_store
                .get_session(&session_id)
                .unwrap();
            if runtime
                .owned
                .prompt_state_owner
                .active_prompt_for_agent(&session, &agent_id)
                .is_none()
            {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await;
    runtime
        .retire_owned_provider_run_after_terminal_failure(&session_id, run.id())
        .await;
    settled.expect("cancellation must release the dispatch lane before native acknowledgement");
    assert!(
        failed_requests(&runtime, &agent_id).is_empty(),
        "a cancelled native request must not be remembered as failed"
    );
    assert_ne!(
        runtime
            .owned
            .agent_store
            .get_agent(&agent_id)
            .unwrap()
            .state(),
        crate::agent::AgentState::Error
    );
    let settlement = runtime
        .owned
        .operational_history_store
        .load_prompt_settlement_event(&session_id, &agent_id, &prompt_id)
        .unwrap()
        .unwrap();
    assert_eq!(
        settlement.metadata[crate::history::PROMPT_SETTLEMENT_STATUS_METADATA_KEY],
        "cancelled"
    );
    let history = runtime
        .owned
        .operational_history_store
        .load_session_events(&session_id, Some(&agent_id))
        .unwrap();
    assert!(!history.iter().any(|event| event
        .content
        .as_deref()
        .is_some_and(|text| text.contains("Provider prompt dispatch failed")
            || text.contains("Request not carried out"))));
}
