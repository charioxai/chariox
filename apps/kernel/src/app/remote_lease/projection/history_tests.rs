use super::*;
use crate::{app::DaemonApp, config::DaemonConfig};

#[test]
fn leased_projection_compacts_finalized_output_keys_and_survives_store_restart() {
    let (mut app, leased) = worker_lease();
    for index in 0..200 {
        let merge_key = format!("message-{index}");
        app.terminal_mut().fan_out_output(
            &leased.backing_session_id,
            "run",
            Some(&leased.backing_agent_id),
            TerminalOutputKind::ProviderOutput,
            Some(merge_key.clone()),
            vec![leased.backing_attachment_id.clone()],
            b"streamed output",
        );
        let first = RemoteLeaseRuntime::new(&mut app)
            .drain_leased_runtime_projection(&leased.id, "run", false)
            .unwrap()
            .unwrap();
        let RelayPeerEvent::LeasedRuntimeProjection { output_chunks, .. } = first.1;
        assert_eq!(output_chunks.len(), 1);
        app.append_history_entry(
            &leased.backing_session_id,
            crate::history::SessionHistoryEntry::provider_output(
                &leased.backing_session_id,
                "run",
                Some(&leased.backing_agent_id),
                TerminalOutputKind::ProviderOutput,
                Some(merge_key),
                "streamed output".to_string(),
            ),
        );
        assert!(RemoteLeaseRuntime::new(&mut app)
            .drain_leased_runtime_projection(&leased.id, "run", false)
            .unwrap()
            .is_none());
        assert!(RemoteLeaseRuntime::new(&mut app)
            .leased_agent_snapshot_for_test(&leased.id)
            .unwrap()
            .projected_output_history_keys
            .is_empty());
    }
    // Reopen the worker's durable store, retaining only normal lease identity.
    let path = app.operational_history_store().path().to_path_buf();
    app.operational_history = crate::history::OperationalHistoryStore::open(path).unwrap();
    assert!(RemoteLeaseRuntime::new(&mut app)
        .drain_leased_runtime_projection(&leased.id, "run", false)
        .unwrap()
        .is_none());
    // A duplicated finalized transcript must also remain suppressed after reopen.
    app.append_history_entry(
        &leased.backing_session_id,
        crate::history::SessionHistoryEntry::provider_output(
            &leased.backing_session_id,
            "run",
            Some(&leased.backing_agent_id),
            TerminalOutputKind::ProviderOutput,
            Some("message-0".into()),
            "streamed output".to_string(),
        ),
    );
    assert!(RemoteLeaseRuntime::new(&mut app)
        .drain_leased_runtime_projection(&leased.id, "run", false)
        .unwrap()
        .is_none());
    RemoteLeaseRuntime::new(&mut app)
        .destroy_leased_agent(&leased.id)
        .unwrap();
}

#[test]
fn leased_projection_compacts_tool_deltas_with_different_transcript_bytes() {
    let (mut app, leased) = worker_lease();
    let mut output = String::new();
    let mut first_delta = None;
    let mut last_delta = None;
    for index in 0..1000 {
        output.push_str(&format!("line-{index}\n"));
        let bytes = serde_json::to_vec_pretty(&serde_json::json!({
            "id": "tool", "status": "running", "output": output,
        }))
        .unwrap();
        let record = crate::app::provider_output_fanout::ProviderOutputFanout::new(&app)
            .fan_out_for_agent(
                &leased.backing_session_id,
                "run",
                Some(&leased.backing_agent_id),
                TerminalOutputKind::ProviderTool,
                Some("tool".into()),
                vec![leased.backing_attachment_id.clone()],
                &bytes,
            );
        if index == 1 {
            first_delta = Some(record.bytes.clone());
        }
        last_delta = Some(record.bytes);
        let projection = RemoteLeaseRuntime::new(&mut app)
            .drain_leased_runtime_projection(&leased.id, "run", false)
            .unwrap()
            .unwrap();
        let RelayPeerEvent::LeasedRuntimeProjection { output_chunks, .. } = projection.1;
        assert_eq!(
            output_chunks.len(),
            1,
            "each changed tool update must project"
        );
        assert_eq!(
            app.operational_history_store()
                .pending_leased_projection_counts(),
            (0, 0)
        );
    }
    let path = app.operational_history_store().path().to_path_buf();
    app.operational_history = crate::history::OperationalHistoryStore::open(path).unwrap();
    for duplicate in [first_delta.unwrap(), last_delta.unwrap()] {
        app.terminal_mut().fan_out_output(
            &leased.backing_session_id,
            "run",
            Some(&leased.backing_agent_id),
            TerminalOutputKind::ProviderTool,
            Some("tool".into()),
            vec![leased.backing_attachment_id.clone()],
            &duplicate,
        );
        assert!(RemoteLeaseRuntime::new(&mut app)
            .drain_leased_runtime_projection(&leased.id, "run", false)
            .unwrap()
            .is_none());
    }
    assert_eq!(
        app.operational_history_store()
            .pending_leased_projection_counts(),
        (0, 0)
    );
    RemoteLeaseRuntime::new(&mut app)
        .destroy_leased_agent(&leased.id)
        .unwrap();
}

fn worker_lease() -> (DaemonApp, LeasedAgent) {
    let mut config = DaemonConfig::for_tests();
    config.accept_remote_leases = true;
    let mut app = DaemonApp::bootstrap(config).unwrap();
    let lease = RemoteLeaseRuntime::new(&mut app)
        .create_execution_lease("home", "session", "agent", false, "user")
        .unwrap();
    let leased = RemoteLeaseRuntime::new(&mut app)
        .create_leased_agent(
            &lease.id,
            "managed-dev-stub",
            "default",
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .unwrap();
    (app, leased)
}

#[test]
fn leased_projection_separates_json_tool_identities_without_merge_keys() {
    let (mut app, leased) = worker_lease();
    let mut deltas = Vec::new();
    for (field, id, length) in [
        ("id", "x", 4000),
        ("call_id", "y", 5),
        ("id", "x", 4001),
        ("call_id", "y", 6),
    ] {
        let bytes = serde_json::to_vec(
            &serde_json::json!({field: id, "status": "running", "output": "a".repeat(length)}),
        )
        .unwrap();
        let record = crate::app::provider_output_fanout::ProviderOutputFanout::new(&app)
            .fan_out_for_agent(
                &leased.backing_session_id,
                "run",
                Some(&leased.backing_agent_id),
                TerminalOutputKind::ProviderTool,
                None,
                vec![leased.backing_attachment_id.clone()],
                &bytes,
            );
        let projection = RemoteLeaseRuntime::new(&mut app)
            .drain_leased_runtime_projection(&leased.id, "run", false)
            .unwrap()
            .unwrap();
        let RelayPeerEvent::LeasedRuntimeProjection { output_chunks, .. } = projection.1;
        assert_eq!(
            output_chunks.len(),
            1,
            "independent tools must not share offsets"
        );
        if length == 4001 || length == 6 {
            deltas.push(record.bytes);
        }
        assert_eq!(
            app.operational_history_store()
                .pending_leased_projection_counts(),
            (0, 0)
        );
    }
    let path = app.operational_history_store().path().to_path_buf();
    app.operational_history = crate::history::OperationalHistoryStore::open(path).unwrap();
    for duplicate in deltas {
        app.terminal_mut().fan_out_output(
            &leased.backing_session_id,
            "run",
            Some(&leased.backing_agent_id),
            TerminalOutputKind::ProviderTool,
            None,
            vec![leased.backing_attachment_id.clone()],
            &duplicate,
        );
        assert!(RemoteLeaseRuntime::new(&mut app)
            .drain_leased_runtime_projection(&leased.id, "run", false)
            .unwrap()
            .is_none());
    }
    RemoteLeaseRuntime::new(&mut app)
        .destroy_leased_agent(&leased.id)
        .unwrap();
}

// MP-08 / MP-10 / MP-11: reused worker snapshots must not donate counters to a rejection.
#[test]
fn mp08_mp10_mp11_leased_reused_run_preserves_unmeasured_rejection() {
    let worktree = crate::test_support::TestWorktree::new("mp08-leased-reused-usage");
    let mut home = DaemonApp::bootstrap(DaemonConfig::for_tests()).unwrap();
    let (session, agent) = crate::app::KernelSessionService::new(&mut home)
        .create_session(
            worktree
                .session_request()
                .with_agent_defaults(crate::session::SessionAgentDefaults::new("dev-stub")),
        )
        .unwrap();
    let attachment = crate::app::KernelSessionService::new(&mut home)
        .attach(crate::attachment::AttachRequest::new(
            session.id(),
            "client",
            crate::attachment::ClientCapabilityLevel::InteractiveStructured,
        ))
        .unwrap();
    let mut config = DaemonConfig::for_tests();
    config.accept_remote_leases = true;
    let mut app = DaemonApp::bootstrap(config).unwrap();
    let lease = RemoteLeaseRuntime::new(&mut app)
        .create_execution_lease("home", session.id(), agent.id(), false, "user")
        .unwrap();
    let leased = RemoteLeaseRuntime::new(&mut app)
        .create_leased_agent(
            &lease.id,
            "managed-dev-stub",
            "default",
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .unwrap();
    let bind_home_worker = |home: &DaemonApp, run_id: Option<&str>| {
        home.agents
            .bind_remote_execution(
                agent.id(),
                crate::agent::RemoteAgentBinding {
                    worker_kernel_id: "worker-kernel".into(),
                    worker_machine_id: "worker-machine".into(),
                    execution_lease_id: lease.id.clone(),
                    leased_agent_id: leased.id.clone(),
                    active_worker_provider_run_id: run_id.map(str::to_string),
                    relay_url: None,
                    relay_token: None,
                    relay_peer_protocol_version: Some(
                        crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION,
                    ),
                },
            )
            .unwrap();
    };
    bind_home_worker(&home, None);
    let measured = crate::usage_accounting::codex_usage(&serde_json::json!({
        "inputTokens": 30, "cachedInputTokens": 10, "outputTokens": 7
    }))
    .unwrap();
    let mut first_run = None;
    for (instruction, counters) in [
        ("measured first turn", Some(measured)),
        ("unmeasured rejection", None),
    ] {
        let (outcome, _) = crate::app::KernelAgentService::new(&mut home)
            .submit_prompt_holding_dispatch(
                session.id(),
                attachment.id(),
                Some(agent.id()),
                instruction,
                "",
                Vec::new(),
            )
            .unwrap();
        let PromptSubmissionOutcome::Started {
            prompt: home_prompt_item,
        } = outcome
        else {
            panic!("home prompt starts")
        };
        let home_prompt = home_prompt_item.id();
        RemoteLeaseRuntime::new(&mut app)
            .begin_leased_prompt_receipt(&leased.id, home_prompt)
            .unwrap();
        let context = crate::transport::relay_peer::RemoteGitTurnContext {
            home_session_id: session.id().into(),
            home_agent_id: agent.id().into(),
            home_prompt_id: home_prompt.into(),
            home_turn_id: home_prompt.into(),
            source_attachment_id: None,
            workspace_live_sync_mode: None,
            prompt_origin: None,
            external_provider: None,
            external_provider_session_id: None,
            external_provider_turn_id: None,
            prompt_summary: home_prompt.into(),
        };
        let (run_id, outcome) = RemoteLeaseRuntime::new(&mut app)
            .submit_leased_prompt_with_workflow_context(
                &leased.id,
                home_prompt,
                Vec::new(),
                None,
                Some(context),
                Vec::new(),
                None,
                crate::extension::RemoteExtensionManifest::default(),
            )
            .unwrap();
        let PromptSubmissionOutcome::Started { .. } = outcome else {
            panic!("prompt starts")
        };
        if let Some(first) = first_run.as_ref() {
            assert_eq!(&run_id, first, "worker run must be reused");
        } else {
            first_run = Some(run_id.clone());
        }
        RemoteLeaseRuntime::new(&mut app)
            .update_leased_prompt_receipt(
                &leased.id,
                home_prompt,
                crate::durable_state::worker_prompt_receipts::WorkerPromptReceiptPhase::Accepted,
                Some(&run_id),
            )
            .unwrap();
        let batch = crate::provider::ProviderPromptSignalBatch {
            resolved_usage: counters.map(|u| crate::provider::ProviderRunTokenUsage {
                accounting: Some(u),
                turn_accounting: Some(u),
                ..Default::default()
            }),
            terminal_failure: counters.is_none().then(|| "turn/start rejected".into()),
            prompt_completed: true,
            ..Default::default()
        };
        crate::app::provider_output::ProviderOutputPump::new(&mut app)
            .apply_structured_output_metadata_for_tests(&leased.backing_session_id, &run_id, batch)
            .unwrap();
        // Reopen receipts to prove the home/worker prompt mapping is durable.
        app.worker_prompt_receipts =
            crate::durable_state::worker_prompt_receipts::WorkerPromptReceiptStore::restore(
                app.durable_state.clone(),
            )
            .unwrap();
        assert_eq!(
            app.providers
                .get_run(&run_id)
                .unwrap()
                .usage()
                .turn_accounting,
            Some(measured),
            "rejection leaves the old provider snapshot cached"
        );
        RemoteLeaseRuntime::new(&mut app)
            .complete_leased_prompt(&leased.id)
            .unwrap();
        let (_, event) = RemoteLeaseRuntime::new(&mut app)
            .drain_leased_runtime_projection(&leased.id, &run_id, false)
            .unwrap()
            .expect("completed turn projects");
        let RelayPeerEvent::LeasedRuntimeProjection {
            provider_run,
            completions,
            ..
        } = &event;
        assert!(completions
            .iter()
            .any(|c| c.home_prompt_id.as_deref() == Some(home_prompt)));
        let usage = provider_run.as_ref().unwrap().usage();
        assert_eq!(
            usage.turn_accounting, counters,
            "accounting belongs to the completed worker prompt"
        );
        assert_eq!(
            usage.accounting, counters,
            "unmeasured rejection has no current counters"
        );
        bind_home_worker(&home, Some(&run_id));
        RemoteLeaseRuntime::new(&mut home)
            .project_remote_runtime_projection(
                crate::runtime::relay_peer_authority::test_projection_authority("worker-kernel"),
                event,
            )
            .unwrap();
        let report =
            crate::usage_accounting::report::load(&home.operational_history, session.id()).unwrap();
        let projected_run_id =
            crate::provider::projected_leased_provider_run_id(&leased.id, &run_id);
        let turn = report
            .turns
            .iter()
            .find(|t| t.prompt_id == home_prompt && t.provider_run_id == projected_run_id)
            .expect("home accounting persisted");
        assert!(turn.completed);
        assert_eq!(turn.usage, counters);
        assert_eq!(turn.provider_counters, counters);
        assert_eq!(
            report
                .turns
                .iter()
                .filter(|t| t.provider_run_id == projected_run_id && t.usage == Some(measured))
                .count(),
            1,
            "only the first turn is measured"
        );
    }
    RemoteLeaseRuntime::new(&mut app)
        .destroy_leased_agent(&leased.id)
        .unwrap();
}

#[test]
fn mp08_mp10_mp11_replay_accepted_receipt_binds_worker_prompt_usage() {
    let (mut app, leased) = worker_lease();
    let home_prompt = "home-prompt-replayed";
    let context = crate::transport::relay_peer::RemoteGitTurnContext {
        home_session_id: "session".into(),
        home_agent_id: "agent".into(),
        home_prompt_id: home_prompt.into(),
        home_turn_id: home_prompt.into(),
        source_attachment_id: None,
        workspace_live_sync_mode: None,
        prompt_origin: None,
        external_provider: None,
        external_provider_session_id: None,
        external_provider_turn_id: None,
        prompt_summary: home_prompt.into(),
    };
    // The worker prompt is active, but its admission receipt was never recorded.
    let (run_id, _) = RemoteLeaseRuntime::new(&mut app)
        .submit_leased_prompt_with_workflow_context(
            &leased.id,
            "replayed turn",
            Vec::new(),
            None,
            Some(context.clone()),
            Vec::new(),
            None,
            crate::extension::RemoteExtensionManifest::default(),
        )
        .unwrap();
    assert!(RemoteLeaseRuntime::new(&mut app)
        .begin_leased_prompt_receipt(&leased.id, home_prompt)
        .unwrap()
        .is_none());
    let (replayed_run, _) = RemoteLeaseRuntime::new(&mut app)
        .replay_and_accept_leased_prompt(&leased.id, Some(&context), true)
        .unwrap()
        .expect("active prompt replays");
    assert_eq!(replayed_run, run_id);
    let measured = crate::usage_accounting::codex_usage(&serde_json::json!({
        "inputTokens": 30, "cachedInputTokens": 10, "outputTokens": 7
    }))
    .unwrap();
    let batch = crate::provider::ProviderPromptSignalBatch {
        resolved_usage: Some(crate::provider::ProviderRunTokenUsage {
            accounting: Some(measured),
            turn_accounting: Some(measured),
            ..Default::default()
        }),
        prompt_completed: true,
        ..Default::default()
    };
    crate::app::provider_output::ProviderOutputPump::new(&mut app)
        .apply_structured_output_metadata_for_tests(&leased.backing_session_id, &run_id, batch)
        .unwrap();
    RemoteLeaseRuntime::new(&mut app)
        .complete_leased_prompt(&leased.id)
        .unwrap();
    let (_, event) = RemoteLeaseRuntime::new(&mut app)
        .drain_leased_runtime_projection(&leased.id, &run_id, false)
        .unwrap()
        .expect("completed turn projects");
    let RelayPeerEvent::LeasedRuntimeProjection { provider_run, .. } = &event;
    assert_eq!(
        provider_run.as_ref().unwrap().usage().turn_accounting,
        Some(measured),
        "replay-accepted receipt keeps the measured worker turn"
    );
    RemoteLeaseRuntime::new(&mut app)
        .destroy_leased_agent(&leased.id)
        .unwrap();
}
