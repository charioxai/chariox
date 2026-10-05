//! MP-08 / MP-11: home restart must settle the independently running worker.
use super::*;
use crate::app::{DaemonApp, KernelSessionService, RemoteLeaseRuntime};
use crate::session::{
    CreateSessionRequest, DurablePromptDeliveryPhase, PromptQueueItem, PromptStatus,
};

#[tokio::test]
async fn retired_remote_meta_restart_holds_exact_turn_until_worker_settlement() {
    assert_remote_retirement(Some(DurablePromptDeliveryPhase::Delivered)).await;
}

#[tokio::test]
async fn retired_remote_meta_restart_reconciles_uncertain_submission_before_cancelling() {
    assert_remote_retirement(Some(DurablePromptDeliveryPhase::Dispatching)).await;
}

#[tokio::test]
async fn retired_remote_meta_restart_clears_idle_worker_policy_before_followup() {
    assert_remote_retirement(None).await;
}

async fn assert_remote_retirement(phase: Option<DurablePromptDeliveryPhase>) {
    let offline_listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let relay_url = format!("ws://{}", offline_listener.local_addr().unwrap());
    let mut config = crate::config::DaemonConfig::for_tests();
    config.relay_url = Some(relay_url.clone());
    config.relay_token = Some("disposable-retirement-fixture".into());
    let mut home = DaemonApp::bootstrap(config.clone()).unwrap();
    let (session, agent) = KernelSessionService::new(&mut home)
        .create_session(CreateSessionRequest::new(
            "remote-retirement",
            "remote-retirement",
        ))
        .unwrap();
    let attachment = KernelSessionService::new(&mut home)
        .attach(crate::attachment::AttachRequest::new(
            session.id(),
            "owner-terminal",
            crate::attachment::ClientCapabilityLevel::FullTerminal,
        ))
        .unwrap();
    home.agents_mut()
        .set_agent_runtime_profile_with_account_profile(
            agent.id(),
            "managed-dev-stub",
            Some("sonnet".into()),
            None,
            Some("default".into()),
            Default::default(),
        )
        .unwrap();
    home.agents_mut()
        .activate_agent_meta_mode(agent.id(), None)
        .unwrap();
    home.sessions_mut()
        .start_or_update_metaagent_task(session.id(), agent.id(), "Retired task")
        .unwrap();

    let mut worker_config = crate::config::DaemonConfig::for_tests();
    worker_config.accept_remote_leases = true;
    let mut worker = DaemonApp::bootstrap(worker_config.clone()).unwrap();
    let lease = RemoteLeaseRuntime::new(&mut worker)
        .create_execution_lease(
            &config.daemon_id,
            session.id(),
            agent.id(),
            true,
            agent.owner_user_id(),
        )
        .unwrap();
    let leased = RemoteLeaseRuntime::new(&mut worker)
        .create_leased_agent(
            &lease.id,
            "managed-dev-stub",
            "default",
            Some("sonnet".into()),
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .unwrap();
    let home_prompt_id = "retired-live-meta-turn";
    let run = if phase.is_some() {
        RemoteLeaseRuntime::new(&mut worker)
            .begin_leased_prompt_receipt(&leased.id, home_prompt_id)
            .unwrap();
        let (run, _) = RemoteLeaseRuntime::new(&mut worker)
            .submit_leased_prompt_with_workflow_context(
                &leased.id,
                "Legacy Meta turn\n",
                Vec::new(),
                None,
                Some(crate::transport::relay_peer::RemoteGitTurnContext {
                    home_session_id: session.id().into(),
                    home_agent_id: agent.id().into(),
                    home_prompt_id: home_prompt_id.into(),
                    home_turn_id: "legacy-turn".into(),
                    source_attachment_id: None,
                    workspace_live_sync_mode: None,
                    prompt_origin: None,
                    external_provider: None,
                    external_provider_session_id: None,
                    external_provider_turn_id: None,
                    prompt_summary: "Legacy Meta turn".into(),
                }),
                Vec::new(),
                None,
                crate::extension::RemoteExtensionManifest::default(),
            )
            .unwrap();
        RemoteLeaseRuntime::new(&mut worker)
            .update_leased_prompt_receipt(
                &leased.id,
                home_prompt_id,
                crate::durable_state::worker_prompt_receipts::WorkerPromptReceiptPhase::Accepted,
                Some(&run),
            )
            .unwrap();
        run
    } else {
        "no-worker-run".into()
    };
    home.agents_mut()
        .bind_remote_execution(
            agent.id(),
            crate::agent::RemoteAgentBinding {
                worker_kernel_id: WORKER_ID.into(),
                worker_machine_id: "fixture-worker-machine".into(),
                execution_lease_id: lease.id.clone(),
                leased_agent_id: leased.id.clone(),
                active_worker_provider_run_id: (phase
                    == Some(DurablePromptDeliveryPhase::Delivered))
                .then(|| run.clone()),
                relay_url: None,
                relay_token: None,
                relay_peer_protocol_version: Some(
                    crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION,
                ),
            },
        )
        .unwrap();
    if phase.is_some() {
        let mut prompt = PromptQueueItem::new(
            home_prompt_id,
            attachment.id(),
            agent.id(),
            "Legacy Meta turn",
            PromptStatus::Running,
        )
        .with_hidden_system_context("Legacy Meta policy");
        prompt.set_durable_delivery(
            phase.unwrap(),
            (phase == Some(DurablePromptDeliveryPhase::Delivered)).then(|| run.clone()),
            None,
        );
        home.prompt_owner_sync_external_active_prompt(session.id(), agent.id(), Some(prompt))
            .unwrap();
    }
    home.save_durable_state_snapshot().unwrap();
    drop(home);

    let mut home = DaemonApp::bootstrap(config.clone()).unwrap();
    assert!(!home.agents().get_agent(agent.id()).unwrap().is_metaagent());
    assert!(worker
        .agents()
        .get_agent(&leased.backing_agent_id)
        .unwrap()
        .is_metaagent());
    assert_eq!(
        worker
            .prompt_owner_active_prompt_for_agent(
                &leased.backing_session_id,
                &leased.backing_agent_id
            )
            .unwrap()
            .is_some(),
        phase.is_some(),
        "worker lifetime is independent of the home restart"
    );
    assert_eq!(
        home.sessions()
            .get_session(session.id())
            .unwrap()
            .metaagent_task(agent.id())
            .unwrap()
            .status(),
        crate::session::MetaagentTaskStatus::Aborted
    );
    let prompt = home
        .prompt_owner_active_prompt_for_agent(session.id(), agent.id())
        .unwrap()
        .expect("retirement must retain the home prompt until the worker is settled");
    if phase.is_some() {
        assert_eq!(prompt.id(), home_prompt_id);
    }
    let home_prompt_id = prompt.id().to_string();
    assert_eq!(prompt.status(), PromptStatus::Cancelling);
    assert_eq!(
        prompt.durable_delivery_provider_run_id(),
        (phase == Some(DurablePromptDeliveryPhase::Delivered)).then_some(run.as_str())
    );
    drop(offline_listener);
    let home = Arc::new(Mutex::new(home));
    let runtime = owned_runtime_state(&home).await;
    let unavailable = runtime.recover_durable_runtime_after_restart().await;
    assert_eq!(unavailable.failed_reconciliations, 1);
    assert_eq!(unavailable.remote_meta_retirements_pending, 1);
    assert_eq!(
        runtime
            .owned
            .session_store
            .get_session(session.id())
            .unwrap()
            .active_prompt_for_agent(agent.id())
            .unwrap()
            .id(),
        home_prompt_id
    );
    drop(runtime);
    drop(home);

    // A second home restart must preserve the durable private retirement marker,
    // exact run and lease while leaving the independently live worker untouched.
    let mut home = DaemonApp::bootstrap(config.clone()).unwrap();
    let attachment = KernelSessionService::new(&mut home)
        .attach(crate::attachment::AttachRequest::new(
            session.id(),
            "reconnected-owner-terminal",
            crate::attachment::ClientCapabilityLevel::FullTerminal,
        ))
        .unwrap();
    let home = Arc::new(Mutex::new(home));
    let runtime = owned_runtime_state(&home).await;
    let prepared = crate::app::KernelPreparedPromptSubmission {
        session_id: session.id().into(),
        prompt: PromptQueueItem::new(
            "ordinary-followup",
            attachment.id(),
            agent.id(),
            "Ordinary follow-up",
            PromptStatus::Queued,
        ),
        force_queue: false,
        refresh_projection: true,
    };
    let queued = runtime
        .owned
        .submit_remote_prepared_prompt(&prepared)
        .unwrap()
        .unwrap();
    assert!(matches!(
        queued.outcome,
        crate::session::PromptSubmissionOutcome::Queued { .. }
    ));
    assert!(queued.remote_dispatch.is_none());
    assert!(
        runtime
            .owned
            .complete_remote_prompt_owner(session.id(), agent.id(), &run, None)
            .is_err(),
        "ordinary completion cannot discard the retirement intent"
    );
    assert!(
        runtime
            .owned
            .finalize_remote_prompt_cancellation_after_worker_settled(
                session.id(),
                agent.id(),
                attachment.id()
            )
            .is_err(),
        "ordinary cancellation cannot bypass mode-off ACK"
    );
    home.lock().await.save_durable_state_snapshot().unwrap();

    let relay_state = Arc::clone(&runtime.owned.relay_state);
    let (outgoing, mut requests, _events) =
        crate::transport::relay_client::RelayOutgoingSender::channel(32);
    {
        let mut relay = relay_state.write().await;
        relay.test_set_connected_sender(outgoing, &relay_url);
        relay.remember_peer_public_key(WORKER_ID, worker_config.relay_public_key.clone());
    }
    let worker = Arc::new(Mutex::new(worker));
    let server_worker = Arc::clone(&worker);
    let mode_requested = Arc::new(tokio::sync::Notify::new());
    let mode_release = Arc::new(tokio::sync::Notify::new());
    let observed_mode = Arc::clone(&mode_requested);
    let release_mode = Arc::clone(&mode_release);
    let followup_accepted = Arc::new(tokio::sync::Notify::new());
    let accepted = Arc::clone(&followup_accepted);
    let home_public_key = config.relay_public_key.clone();
    let worker_private_key = worker_config.relay_private_key.clone();
    let worker_leased = leased.clone();
    let server = tokio::spawn(async move {
        let mut cancelled = false;
        let mut cleared = false;
        while let Some(envelope) = requests.recv().await {
            let (request_id, target, request) = decode_peer_request(envelope, &worker_private_key);
            assert_eq!(target, WORKER_ID);
            if matches!(request, RelayPeerRequest::UpdateLeasedAgentMetaMode { .. }) && !cleared {
                observed_mode.notify_one();
                release_mode.notified().await;
            }
            let mut worker = server_worker.lock().await;
            let mut lease_runtime = RemoteLeaseRuntime::new(&mut worker);
            let response = match request {
                RelayPeerRequest::GetLeasedPromptReceipt {
                    leased_agent_id,
                    home_prompt_id,
                } => {
                    assert_eq!(leased_agent_id, worker_leased.id);
                    assert_eq!(home_prompt_id, "retired-live-meta-turn");
                    RelayPeerResponse::LeasedPromptReceiptQueried {
                        receipt: lease_runtime
                            .leased_prompt_receipt(&leased_agent_id, &home_prompt_id)
                            .unwrap(),
                    }
                }
                RelayPeerRequest::CancelLeasedPrompt {
                    leased_agent_id,
                    home_prompt_id,
                    worker_provider_run_id,
                } => {
                    assert!(!cleared);
                    let cancellation = lease_runtime
                        .cancel_leased_prompt(
                            &leased_agent_id,
                            &home_prompt_id,
                            &worker_provider_run_id,
                        )
                        .unwrap();
                    assert_eq!(cancellation.prompt.status(), PromptStatus::Cancelling);
                    cancelled = true;
                    RelayPeerResponse::LeasedPromptCancelled { cancellation }
                }
                RelayPeerRequest::DrainLeasedRuntimeProjection {
                    leased_agent_id,
                    provider_run_id,
                    pump_output,
                } => {
                    let event = lease_runtime
                        .drain_leased_runtime_projection_with_recovery(
                            &leased_agent_id,
                            &provider_run_id,
                            pump_output,
                            true,
                        )
                        .unwrap()
                        .map(|(_, event)| event);
                    RelayPeerResponse::LeasedRuntimeProjectionDrained { event }
                }
                RelayPeerRequest::UpdateLeasedAgentMetaMode {
                    leased_agent_id,
                    active,
                } => {
                    assert!(cancelled || phase.is_none());
                    assert!(!active);
                    let leased_agent = lease_runtime
                        .update_leased_agent_meta_mode(&leased_agent_id, active)
                        .unwrap();
                    cleared = true;
                    RelayPeerResponse::LeasedAgentMetaModeUpdated { leased_agent }
                }
                RelayPeerRequest::UpdateLeasedAgentRemoteExtensionManifest {
                    leased_agent_id,
                    remote_extension_manifest,
                } => {
                    assert!(
                        cleared,
                        "follow-ups cannot synchronize provider policy before retirement ACK"
                    );
                    lease_runtime
                        .update_leased_agent_remote_extension_manifest(
                            &leased_agent_id,
                            remote_extension_manifest,
                        )
                        .unwrap();
                    RelayPeerResponse::LeasedAgentRemoteExtensionManifestUpdated { leased_agent_id }
                }
                RelayPeerRequest::SubmitLeasedPrompt {
                    leased_agent_id,
                    prompt,
                    hidden_system_context,
                    attachments,
                    workflow_context,
                    git_context,
                    required_mcps,
                    required_skills,
                    remote_extension_manifest,
                    ..
                } => {
                    assert!(
                        cleared,
                        "ordinary worker admission must follow mode-off ACK"
                    );
                    assert_eq!(prompt.trim(), "Ordinary follow-up");
                    assert!(!hidden_system_context.contains("kernel-remote-meta-retirement:"));
                    assert!(!worker
                        .agents()
                        .get_agent(&worker_leased.backing_agent_id)
                        .unwrap()
                        .is_metaagent());
                    let (provider_run_id, outcome) = RemoteLeaseRuntime::new(&mut worker)
                        .submit_leased_prompt_with_workflow_context(
                            &leased_agent_id,
                            &prompt,
                            attachments,
                            workflow_context,
                            git_context,
                            required_mcps,
                            required_skills,
                            remote_extension_manifest,
                        )
                        .unwrap();
                    accepted.notify_one();
                    RelayPeerResponse::LeasedPromptSubmitted {
                        provider_run_id,
                        outcome,
                    }
                }
                _ => panic!("unexpected retirement worker request kind"),
            };
            drop(worker);
            acknowledge_peer_request(
                &relay_state,
                request_id,
                &worker_private_key,
                &home_public_key,
                response,
            )
            .await;
        }
    });
    struct StopServer(tokio::task::JoinHandle<()>);
    impl Drop for StopServer {
        fn drop(&mut self) {
            self.0.abort();
        }
    }
    let _server = StopServer(server);
    let connection = rusqlite::Connection::open(config.durable_state_path()).unwrap();
    connection
        .execute_batch(
            "CREATE TRIGGER fail_remote_retirement BEFORE INSERT ON durable_state_events
        WHEN NEW.kind = 'session.prompt_state.updated'
        BEGIN SELECT RAISE(FAIL, 'injected remote retirement write failure'); END;",
        )
        .unwrap();
    let recovery_state = runtime.clone();
    let recovery =
        tokio::spawn(async move { recovery_state.recover_durable_runtime_after_restart().await });
    tokio::time::timeout(Duration::from_secs(3), mode_requested.notified())
        .await
        .unwrap();
    let held = runtime
        .owned
        .session_store
        .get_session(session.id())
        .unwrap();
    assert_eq!(
        held.active_prompt_for_agent(agent.id()).unwrap().id(),
        home_prompt_id
    );
    assert_eq!(held.queued_prompts_for_agent(agent.id()).unwrap().len(), 1);
    assert!(
        worker
            .lock()
            .await
            .agents()
            .get_agent(&leased.backing_agent_id)
            .unwrap()
            .is_metaagent(),
        "home must not discard the intent before worker mode-off acknowledgement"
    );
    mode_release.notify_one();
    let summary = tokio::time::timeout(Duration::from_secs(5), recovery)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(summary.failed_reconciliations, 1);
    assert_eq!(summary.remote_meta_retirements_pending, 1);
    assert_eq!(
        runtime
            .owned
            .session_store
            .get_session(session.id())
            .unwrap()
            .active_prompt_for_agent(agent.id())
            .unwrap()
            .id(),
        home_prompt_id
    );
    assert!(!worker
        .lock()
        .await
        .agents()
        .get_agent(&leased.backing_agent_id)
        .unwrap()
        .is_metaagent());
    connection
        .execute_batch("DROP TRIGGER fail_remote_retirement;")
        .unwrap();
    drop(connection);
    let recovery_task = runtime.spawn_durable_restart_recovery();
    tokio::time::timeout(Duration::from_secs(5), followup_accepted.notified())
        .await
        .unwrap();
    assert!(!worker
        .lock()
        .await
        .agents()
        .get_agent(&leased.backing_agent_id)
        .unwrap()
        .is_metaagent());
    let current = runtime
        .owned
        .session_store
        .get_session(session.id())
        .unwrap();
    assert!(current
        .active_prompt_for_agent(agent.id())
        .is_none_or(|prompt| prompt.id() != home_prompt_id));
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let current = runtime
                .owned
                .session_store
                .get_session(session.id())
                .unwrap();
            if current.active_prompt_for_agent(agent.id()).is_none()
                && runtime
                    .owned
                    .remote_prompt_recoveries
                    .lock()
                    .unwrap()
                    .is_empty()
                && runtime
                    .owned
                    .remote_prompt_projection_drains
                    .lock()
                    .unwrap()
                    .is_empty()
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    drop(recovery_task);
    drop(runtime);
    drop(home);
    // Replaying the retirement commit and ordinary admission after a third
    // restart must never resurrect the cancelled Meta prompt or its policy.
    let mut restored = DaemonApp::bootstrap(config).unwrap();
    assert!(!restored
        .agents()
        .get_agent(agent.id())
        .unwrap()
        .is_metaagent());
    assert!(restored
        .prompt_owner_active_prompt_for_agent(session.id(), agent.id())
        .unwrap()
        .is_none_or(|prompt| prompt.id() != home_prompt_id));
}
