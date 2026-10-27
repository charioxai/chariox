use super::*;

#[derive(Clone, Copy)]
enum SettlementPath {
    AppOutput,
    LeasedQuiet,
    LeasedOutputHistory,
    LeasedCompletionRecord,
}

async fn quiet_settlement_during_runtime_mcp(path: SettlementPath) {
    let mut config = crate::config::DaemonConfig::for_tests();
    config.accept_remote_leases = true;
    let mut app = DaemonApp::bootstrap(config).expect("daemon bootstrap");
    let lease = crate::app::RemoteLeaseRuntime::new(&mut app)
        .create_execution_lease("home-kernel", "home-room", "home-agent", false, "home-user")
        .expect("execution lease");
    let leased = crate::app::RemoteLeaseRuntime::new(&mut app)
        .create_leased_agent(
            &lease.id,
            "managed-dev-stub",
            "default",
            Some("native-tui-idle".into()),
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .expect("leased agent");
    let (run_id, outcome) = crate::app::RemoteLeaseRuntime::new(&mut app)
        .submit_leased_prompt(&leased.id, "fixture task", Vec::new())
        .expect("leased prompt");
    let crate::session::PromptSubmissionOutcome::Started { prompt } = outcome else {
        panic!("prompt starts");
    };
    if matches!(
        path,
        SettlementPath::LeasedOutputHistory | SettlementPath::LeasedCompletionRecord
    ) {
        crate::app::RemoteLeaseRuntime::new(&mut app)
            .drain_leased_runtime_projection(&leased.id, &run_id, false)
            .expect("prime initial prompt projection");
    }
    let run = app.providers.get_run(&run_id).expect("provider run");
    let token = run
        .runtime_mcp_auth_token()
        .expect("runtime MCP auth")
        .to_string();
    let recipient = crate::app::KernelSessionService::new(&mut app)
        .spawn_agent(crate::agent::CreateAgentRequest::new(
            &leased.backing_session_id,
            "dev-stub",
        ))
        .expect("recipient");
    let app = Arc::new(Mutex::new(app));
    let runtime = owned_runtime_state(&app).await;
    let gate = runtime.owned.agent_message_idempotency.lock().await;
    let call_runtime = runtime.clone();
    let call = tokio::spawn(async move {
        call_runtime.dispatch_authenticated_runtime_tool_call(&token,
            crate::transport::runtime_tools::SEND_AGENT_MESSAGE_TOOL,
            serde_json::json!({"agent":recipient.id(), "message":"fixture message", "origin_prompt_id":prompt.id(), "idempotency_key":"quiet-settlement-guard"})).await
    });
    tokio::time::timeout(std::time::Duration::from_secs(2), async {
        while runtime
            .owned
            .runtime_tool_call_activity
            .active_count(&run_id)
            == 0
        {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("actual runtime MCP dispatch is blocked in its handler");
    let (prompt_retained, run_state, output_projected, completion_deferred) = {
        let mut app = app.lock().await;
        crate::transport::flow_control::note_prompt_response_content(&mut app, &run_id);
        app.prompt_activity
            .write()
            .get_mut(&run_id)
            .expect("prompt activity")
            .last_output_at =
            Some(std::time::Instant::now() - std::time::Duration::from_millis(100));
        if matches!(path, SettlementPath::LeasedOutputHistory) {
            app.fan_out_output_for_agent(
                &leased.backing_session_id,
                &run_id,
                Some(&leased.backing_agent_id),
                crate::terminal::TerminalOutputKind::ProviderOutput,
                Some("fixture-runtime-output".into()),
                vec![leased.backing_attachment_id.clone()],
                b"fixture buffered provider output",
            );
        }
        if matches!(path, SettlementPath::LeasedCompletionRecord) {
            app.terminal().record_assistant_message_completion(
                &leased.backing_session_id,
                &run_id,
                Some(&leased.backing_agent_id),
                vec![leased.backing_attachment_id.clone()],
                "fixture-runtime-tool-completion",
                crate::session::unix_epoch_ms(),
            );
        }
        let mut output_projected = false;
        let mut completion_deferred = true;
        if !matches!(path, SettlementPath::AppOutput) {
            let _released_projection = crate::app::RemoteLeaseRuntime::new(&mut app)
                .drain_leased_runtime_projection(&leased.id, &run_id, true)
                .expect("leased projection drain")
                .map(|(_, event)| {
                    let crate::transport::relay_peer::RelayPeerEvent::LeasedRuntimeProjection {
                        output_chunks,
                        completions,
                        ..
                    } = event;
                    output_projected = !output_chunks.is_empty();
                    completion_deferred = completions.is_empty();
                });
        } else {
            crate::app::provider_output::pump_terminal_output_for_attachment(
                &mut app,
                &leased.backing_session_id,
                &leased.backing_attachment_id,
            )
            .expect("app output pump");
        }
        (
            app.prompt_owner_active_prompt_for_agent_snapshot(
                &leased.backing_session_id,
                &leased.backing_agent_id,
            )
            .expect("active prompt snapshot")
            .is_some(),
            app.providers.get_run(&run_id).unwrap().state(),
            output_projected,
            completion_deferred,
        )
    };
    drop(gate);
    call.await
        .expect("handler task")
        .expect("handler transport");
    assert_eq!(
        runtime
            .owned
            .runtime_tool_call_activity
            .active_count(&run_id),
        0
    );
    {
        let mut app = app.lock().await;
        if prompt_retained {
            app.prompt_activity
                .write()
                .get_mut(&run_id)
                .expect("retained prompt activity")
                .last_output_at =
                Some(std::time::Instant::now() - std::time::Duration::from_millis(100));
            let released_projection = crate::app::RemoteLeaseRuntime::new(&mut app)
                .drain_leased_runtime_projection(&leased.id, &run_id, true)
                .expect("post-handler quiet projection");
            if matches!(path, SettlementPath::LeasedCompletionRecord) {
                let Some((
                    _,
                    crate::transport::relay_peer::RelayPeerEvent::LeasedRuntimeProjection {
                        completions,
                        ..
                    },
                )) = released_projection
                else {
                    panic!("completion projection after guard drop");
                };
                assert!(
                    completions.iter().any(
                        |completion| completion.message_id == "fixture-runtime-tool-completion"
                    ),
                    "original completion identity retained through guard drop"
                );
                let duplicate = crate::app::RemoteLeaseRuntime::new(&mut app)
                    .drain_leased_runtime_projection(&leased.id, &run_id, false)
                    .expect("duplicate completion drain");
                if let Some((
                    _,
                    crate::transport::relay_peer::RelayPeerEvent::LeasedRuntimeProjection {
                        completions,
                        ..
                    },
                )) = duplicate
                {
                    assert!(
                        completions.is_empty(),
                        "deferred completion projects exactly once"
                    );
                }
            }
            assert!(
                app.prompt_owner_active_prompt_for_agent_snapshot(
                    &leased.backing_session_id,
                    &leased.backing_agent_id
                )
                .expect("post-handler prompt snapshot")
                .is_none(),
                "quiet completion still proceeds after guard drop"
            );
        }
        app.teardown_provider_processes(None, true)
            .expect("fixture cleanup");
    }
    assert!(
        prompt_retained,
        "app quiet settlement must retain an in-flight runtime MCP prompt"
    );
    assert_eq!(run_state, crate::provider::ProviderRunState::Running);
    assert!(
        completion_deferred,
        "completion must not project while handler is active"
    );
    if matches!(path, SettlementPath::LeasedOutputHistory) {
        assert!(
            output_projected,
            "guard must allow buffered output to project"
        );
    }
}

#[tokio::test]
async fn app_output_quiet_settlement_preserves_active_runtime_mcp_handler() {
    quiet_settlement_during_runtime_mcp(SettlementPath::AppOutput).await;
}

#[tokio::test]
async fn leased_projection_quiet_settlement_preserves_active_runtime_mcp_handler() {
    quiet_settlement_during_runtime_mcp(SettlementPath::LeasedQuiet).await;
}

#[tokio::test]
async fn leased_output_history_preserves_active_runtime_mcp_handler() {
    quiet_settlement_during_runtime_mcp(SettlementPath::LeasedOutputHistory).await;
}

#[tokio::test]
async fn leased_completion_record_preserves_active_runtime_mcp_handler() {
    quiet_settlement_during_runtime_mcp(SettlementPath::LeasedCompletionRecord).await;
}
