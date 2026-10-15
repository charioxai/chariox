//! MP-08 / MP-10 / MP-11: receipt reconciliation must consume retained history pages.
use super::*;
use crate::app::RemoteLeaseRuntime;
use crate::terminal::TerminalOutputKind;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mp08_completed_receipt_reconciles_history_backlog_without_resubmitting() {
    completed_history_recovery(false, false, false).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mp08_completed_receipt_rejects_unadmitted_history_page() {
    completed_history_recovery(true, false, false).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mp08_completed_receipt_reacks_lost_ack_without_replaying_history() {
    completed_history_recovery(false, true, false).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mp08_completed_receipt_reconciles_dispatching_prompt_with_history_backlog() {
    completed_history_recovery(false, false, true).await;
}

async fn completed_history_recovery(future_page: bool, lost_ack: bool, dispatching: bool) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let relay_url = format!("ws://{}", listener.local_addr().unwrap());
    let mut fixture = make_receipt_reconciliation_fixture(&relay_url, "history-pages").await;
    if dispatching {
        let session = fixture
            .runtime
            .owned
            .session_store
            .get_session(&fixture.session_id)
            .unwrap();
        let original = fixture
            .runtime
            .owned
            .prompt_state_owner
            .active_prompt_for_agent(&session, &fixture.agent_id)
            .unwrap();
        let mut pending = original.clone();
        pending.set_status(crate::session::PromptStatus::Dispatching);
        assert!(fixture
            .runtime
            .owned
            .prompt_state_owner
            .replace_active_prompt_if_matches(&session, &fixture.agent_id, &original, pending,));
    }
    let mut worker_config = crate::config::DaemonConfig::for_tests();
    worker_config.accept_remote_leases = true;
    let mut worker = DaemonApp::bootstrap(worker_config).unwrap();
    let lease = RemoteLeaseRuntime::new(&mut worker)
        .create_execution_lease(
            "home",
            &fixture.session_id,
            &fixture.agent_id,
            false,
            "user",
        )
        .unwrap();
    let leased = RemoteLeaseRuntime::new(&mut worker)
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
    let mut binding = fixture
        .runtime
        .owned
        .agent_store
        .get_agent(&fixture.agent_id)
        .unwrap()
        .remote_execution()
        .unwrap()
        .clone();
    binding.execution_lease_id = lease.id.clone();
    binding.leased_agent_id = leased.id.clone();
    fixture
        .runtime
        .owned
        .agent_store
        .bind_remote_execution(&fixture.agent_id, binding)
        .unwrap();
    fixture.dispatch.leased_agent_id = leased.id.clone();
    let context = crate::transport::relay_peer::RemoteGitTurnContext {
        home_session_id: fixture.session_id.clone(),
        home_agent_id: fixture.agent_id.clone(),
        home_prompt_id: fixture.dispatch.prompt_id.clone(),
        home_turn_id: "lost-submit-ack".into(),
        source_attachment_id: None,
        workspace_live_sync_mode: None,
        prompt_origin: None,
        external_provider: None,
        external_provider_session_id: None,
        external_provider_turn_id: None,
        prompt_summary: "completed history recovery".into(),
    };
    let (run, submission) = RemoteLeaseRuntime::new(&mut worker)
        .submit_leased_prompt_with_workflow_context(
            &leased.id,
            "completed history recovery",
            vec![],
            None,
            Some(context),
            vec![],
            None,
            crate::extension::RemoteExtensionManifest::default(),
        )
        .unwrap();
    // Admission really happened on the worker; its reply was lost at home.
    RemoteLeaseRuntime::new(&mut worker)
        .begin_leased_prompt_receipt(&leased.id, &fixture.dispatch.prompt_id)
        .unwrap();
    let crate::session::PromptSubmissionOutcome::Started {
        prompt: backing_prompt,
    } = submission
    else {
        panic!("worker must accept the original prompt once");
    };
    RemoteLeaseRuntime::new(&mut worker)
        .accept_leased_prompt_receipt(
            &leased.id,
            &fixture.dispatch.prompt_id,
            &run,
            backing_prompt.id(),
        )
        .unwrap();
    for index in 0..130 {
        worker.append_history_entry(
            &leased.backing_session_id,
            crate::history::SessionHistoryEntry::provider_output(
                &leased.backing_session_id,
                &run,
                Some(&leased.backing_agent_id),
                TerminalOutputKind::ProviderOutput,
                Some(format!("backlog-{index}")),
                format!("retained history record {index:03}"),
            ),
        );
    }
    RemoteLeaseRuntime::new(&mut worker)
        .complete_leased_prompt(&leased.id)
        .unwrap();
    assert_eq!(
        RemoteLeaseRuntime::new(&mut worker)
            .leased_prompt_receipt(&leased.id, &fixture.dispatch.prompt_id)
            .unwrap()
            .unwrap()
            .phase,
        crate::transport::relay_peer::LeasedPromptReceiptPhase::Completed
    );
    let claim = RemotePromptAgentClaim::try_acquire(
        Arc::clone(&fixture.runtime.owned.remote_prompt_recoveries),
        &fixture.session_id,
        &fixture.agent_id,
    )
    .unwrap();
    let worker = Arc::new(std::sync::Mutex::new(worker));
    let server_worker = worker.clone();
    let worker_id = fixture.worker_id.clone();
    let machine_id = fixture.worker_machine_id.clone();
    let public_key = fixture.worker_public_key.clone();
    let private_key = fixture.worker_private_key.clone();
    let home_key = fixture.home_public_key.clone();
    let leased_id = leased.id.clone();
    let home_prompt = fixture.dispatch.prompt_id.clone();
    let server_run = run.clone();
    let home_runtime = fixture.runtime.clone();
    let home_session = fixture.session_id.clone();
    let home_agent = fixture.agent_id.clone();
    let server = tokio::spawn(async move {
        let mut acknowledged = Vec::new();
        let mut intermediate = 0;
        let mut final_page = None;
        let mut dropped_ack = false;
        loop {
            let request = receive_fake_worker_peer_request_with_timeout(
                &listener,
                &worker_id,
                &machine_id,
                &public_key,
                &private_key,
                None,
                // Applying 64 durable records may outlast the old fixture's
                // three-second listen window under concurrent builder I/O.
                std::time::Duration::from_secs(60),
            )
            .await;
            let response = {
                let mut app = server_worker.lock().unwrap();
                let mut runtime = RemoteLeaseRuntime::new(&mut app);
                match &request.request {
                    RelayPeerRequest::GetLeasedPromptReceipt {
                        leased_agent_id,
                        home_prompt_id,
                    } => {
                        assert_eq!(leased_agent_id, &leased_id);
                        assert_eq!(home_prompt_id, &home_prompt);
                        RelayPeerResponse::LeasedPromptReceiptQueried {
                            receipt: runtime
                                .leased_prompt_receipt(leased_agent_id, home_prompt_id)
                                .unwrap(),
                        }
                    }
                    RelayPeerRequest::DrainLeasedRuntimeProjection {
                        leased_agent_id,
                        provider_run_id,
                        ..
                    } => {
                        assert_eq!(leased_agent_id, &leased_id);
                        assert_eq!(provider_run_id, &server_run);
                        let (_, mut event) = runtime
                            .drain_retained_leased_runtime_projection(
                                leased_agent_id,
                                provider_run_id,
                                false,
                                true,
                            )
                            .unwrap()
                            .expect("completed receipt must retain its next page");
                        let RelayPeerEvent::LeasedRuntimeProjection {
                            delivery,
                            completions,
                            ..
                        } = &mut event;
                        if completions
                            .iter()
                            .any(|c| c.home_prompt_id.as_deref() == Some(&home_prompt))
                        {
                            final_page = Some(delivery.as_ref().unwrap().sequence);
                        } else {
                            intermediate += 1;
                            let session = home_runtime
                                .owned
                                .session_store
                                .get_session(&home_session)
                                .unwrap();
                            let (active, queued) = home_runtime
                                .owned
                                .prompt_state_owner
                                .state_parts(&session, &home_agent);
                            assert_eq!(active.unwrap().id(), home_prompt);
                            assert_eq!(
                                queued.len(),
                                1,
                                "intermediate pages cannot admit successors"
                            );
                        }
                        if future_page {
                            delivery.as_mut().unwrap().sequence += 1;
                        }
                        RelayPeerResponse::LeasedRuntimeProjectionDrained { event: Some(event) }
                    }
                    RelayPeerRequest::AcknowledgeLeasedRuntimeProjection {
                        leased_agent_id,
                        provider_run_id,
                        delivery,
                    } => {
                        if lost_ack && !dropped_ack {
                            dropped_ack = true;
                            RelayPeerResponse::LeasedRuntimeProjectionAcknowledged {
                                accepted: false,
                            }
                        } else {
                            assert!(runtime
                                .acknowledge_leased_runtime_projection(
                                    leased_agent_id,
                                    provider_run_id,
                                    delivery,
                                )
                                .unwrap());
                            acknowledged.push(delivery.sequence);
                            RelayPeerResponse::LeasedRuntimeProjectionAcknowledged {
                                accepted: true,
                            }
                        }
                    }

                    RelayPeerRequest::ObserveLeasedGitAfter { .. } => {
                        RelayPeerResponse::LeasedGitObserved {
                            provider_run_id: server_run.clone(),
                            git_observations: vec![],
                            workspace_live_sync_change: None,
                        }
                    }
                    other => panic!("recovery must never resubmit the lost prompt: {other:?}"),
                }
            };
            let done = final_page.is_some_and(|last| acknowledged.last() == Some(&last));
            send_fake_worker_peer_response(request, &worker_id, &private_key, &home_key, response)
                .await;
            if done {
                return (intermediate, acknowledged);
            }
        }
    });
    let mut result = tokio::time::timeout(
        std::time::Duration::from_secs(60),
        fixture
            .runtime
            .reconcile_remote_prompt_worker_receipt(&fixture.dispatch),
    )
    .await
    .expect("completed receipt recovery must be bounded");
    if lost_ack {
        assert!(
            result.is_err(),
            "lost ACK must yield bounded, read-only reconciliation"
        );
        result = tokio::time::timeout(
            std::time::Duration::from_secs(60),
            fixture
                .runtime
                .reconcile_remote_prompt_worker_receipt(&fixture.dispatch),
        )
        .await
        .expect("read-only receipt retry must be bounded");
    }
    if future_page {
        server.abort();
        let _ = server.await;
        assert!(
            result.is_err(),
            "a rejected page cannot count as recovered completion"
        );
    } else {
        // Abort first on failure so no test-owned listener/task survives the assertion.
        if result.is_err() {
            server.abort();
        }
        assert!(
            result.is_ok(),
            "completed history must reconcile, got {result:?}"
        );
        let (intermediate, acknowledged) = server.await.unwrap();
        assert!(
            intermediate >= 2,
            "exercise the worker's real 64-record history limit"
        );
        assert!(acknowledged.len() >= 3);
        assert_eq!(
            acknowledged,
            (1..=acknowledged.len() as u64).collect::<Vec<_>>()
        );
        fixture
            .runtime
            .finish_remote_prompt_dispatch(fixture.dispatch.clone(), Ok(run.clone()))
            .await
            .unwrap();
    }
    let session = fixture
        .runtime
        .owned
        .session_store
        .get_session(&fixture.session_id)
        .unwrap();
    let (active, queued) = fixture
        .runtime
        .owned
        .prompt_state_owner
        .state_parts(&session, &fixture.agent_id);
    if future_page {
        assert_eq!(active.unwrap().id(), fixture.dispatch.prompt_id);
        assert_eq!(queued.len(), 1);
    } else {
        assert_eq!(active.unwrap().id(), fixture.successor_prompt_id);
        assert!(queued.is_empty());
    }
    let events = fixture
        .runtime
        .owned
        .operational_history_store
        .load_session_events_for_agent_sequence_range(
            &fixture.session_id,
            &fixture.agent_id,
            0,
            i64::MAX as u64,
        )
        .unwrap();
    assert_eq!(
        events
            .iter()
            .filter(|e| e
                .content
                .as_deref()
                .is_some_and(|c| c.starts_with("retained history record ")))
            .count(),
        if future_page { 0 } else { 130 },
        "each output record is applied once"
    );
    assert_eq!(
        events
            .iter()
            .filter(
                |e| e.prompt_id.as_deref() == Some(&fixture.dispatch.prompt_id)
                    && e.metadata
                        .contains_key(crate::history::PROMPT_SETTLED_AT_MS_METADATA_KEY)
            )
            .count(),
        if future_page { 0 } else { 1 },
        "exact prompt settles once, only after the last page"
    );
    RemoteLeaseRuntime::new(&mut worker.lock().unwrap())
        .destroy_leased_agent(&leased.id)
        .unwrap();
    drop(claim);
}
