//! MP-08 / MP-10 / MP-11: loss/retry, shared push/poll, paging and admission.
use super::*;
use crate::transport::relay_peer::RelayProjectionDelivery;

#[test]
fn mp08_projection_acknowledgement_cannot_skip_or_cross_a_binding() {
    let (mut app, leased) = super::history_tests::worker_lease();
    let run = super::explicit_completion_tests::provider_run(
        "run",
        &leased.backing_session_id,
        &leased.backing_agent_id,
        "managed-dev-stub",
        "managed-dev-stub",
        crate::provider::ProviderClientInterface::Chariox,
    );
    app.providers_mut().insert_run_for_test(run);
    app.terminal_mut().fan_out_output(
        &leased.backing_session_id,
        "run",
        Some(&leased.backing_agent_id),
        TerminalOutputKind::ProviderOutput,
        Some("message".into()),
        vec![leased.backing_attachment_id.clone()],
        b"answer",
    );
    let first = RemoteLeaseRuntime::new(&mut app)
        .drain_retained_leased_runtime_projection(&leased.id, "run", false, false)
        .unwrap()
        .unwrap();
    let RelayPeerEvent::LeasedRuntimeProjection {
        delivery: Some(receipt),
        ..
    } = &first.1
    else {
        panic!("missing receipt");
    };
    let receipt = receipt.clone();
    for invalid in [
        RelayProjectionDelivery {
            execution_lease_id: "other".into(),
            ..receipt.clone()
        },
        RelayProjectionDelivery {
            leased_agent_id: "other".into(),
            ..receipt.clone()
        },
        RelayProjectionDelivery {
            sequence: 0,
            ..receipt.clone()
        },
        RelayProjectionDelivery {
            sequence: receipt.sequence + 1,
            ..receipt.clone()
        },
    ] {
        assert!(!RemoteLeaseRuntime::new(&mut app)
            .acknowledge_leased_runtime_projection(&leased.id, "run", &invalid)
            .unwrap());
        let replay = RemoteLeaseRuntime::new(&mut app)
            .drain_retained_leased_runtime_projection(&leased.id, "run", false, false)
            .unwrap();
        assert_eq!(
            serde_json::to_value(replay).unwrap(),
            serde_json::to_value(Some(first.clone())).unwrap()
        );
    }
    assert!(RemoteLeaseRuntime::new(&mut app)
        .acknowledge_leased_runtime_projection(&leased.id, "unrelated-run", &receipt)
        .is_err());
    assert!(RemoteLeaseRuntime::new(&mut app)
        .acknowledge_leased_runtime_projection(&leased.id, "run", &receipt)
        .unwrap());
    assert!(
        RemoteLeaseRuntime::new(&mut app)
            .acknowledge_leased_runtime_projection(&leased.id, "run", &receipt)
            .unwrap(),
        "lost ACK reply must be safe"
    );
    assert!(RemoteLeaseRuntime::new(&mut app)
        .drain_retained_leased_runtime_projection(&leased.id, "run", false, false)
        .unwrap()
        .is_none());
}

#[test]
fn mp08_projection_pages_retain_output_before_settlement_and_replay_exact_bytes() {
    let receipt = RelayProjectionDelivery {
        execution_lease_id: "lease".into(),
        leased_agent_id: "leased".into(),
        sequence: 1,
    };
    let chunks = (0..40)
        .map(|n| RelayProjectedOutputChunk {
            kind: TerminalOutputKind::ProviderOutput,
            merge_key: Some(format!("message-{n}")),
            bytes: vec![b'x'; 16 * 1024],
        })
        .collect::<Vec<_>>();
    let event = RelayPeerEvent::LeasedRuntimeProjection {
        home_session_id: "home".into(),
        home_agent_id: "agent".into(),
        provider_run_id: "run".into(),
        delivery: None,
        provider_run: None,
        prompts: vec![RelayProjectedPrompt {
            prompt_id: "prompt".into(),
            text: "question".into(),
        }],
        output_chunks: chunks.clone(),
        notices: vec![],
        completions: vec![RelayProjectedCompletion {
            message_id: "done".into(),
            completed_at_ms: 1,
            home_prompt_id: Some("prompt".into()),
            settlement_status: None,
            provider_termination: None,
        }],
    };
    let pages = delivery::pages(event, receipt);
    assert!(pages.len() > 1);
    let mut replayed = vec![];
    for (index, page) in pages.iter().enumerate() {
        assert!(serde_json::to_vec(page).unwrap().len() <= 128 * 1024);
        let RelayPeerEvent::LeasedRuntimeProjection {
            delivery: Some(receipt),
            prompts,
            output_chunks,
            completions,
            ..
        } = page
        else {
            panic!("missing receipt");
        };
        assert_eq!(receipt.sequence, index as u64 + 1);
        if index > 0 {
            assert!(prompts.is_empty());
        }
        if index + 1 < pages.len() {
            assert!(completions.is_empty());
        }
        replayed.extend(output_chunks.clone());
    }
    assert_eq!(replayed, chunks);
    let (app, _) = super::history_tests::worker_lease();
    let store = app.operational_history_store();
    store
        .commit_leased_projection_cursor_with_delivery(
            "projection",
            &Default::default(),
            &[],
            &[],
            &[],
            &pages,
        )
        .unwrap();
    for (index, page) in pages.into_iter().enumerate() {
        assert_eq!(
            store
                .pending_leased_projection_delivery("projection")
                .unwrap(),
            Some(page)
        );
        assert!(store
            .acknowledge_leased_projection_delivery("projection", index as u64 + 1)
            .unwrap());
    }
    assert!(store
        .pending_leased_projection_delivery("projection")
        .unwrap()
        .is_none());
}

#[test]
fn mp08_projection_home_deduplicates_replayed_output_and_rechecks_admission() {
    let mut app =
        crate::app::DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests()).unwrap();
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(crate::session::CreateSessionRequest::new("home", "home"))
        .unwrap();
    app.agents
        .bind_remote_execution(
            agent.id(),
            crate::agent::RemoteAgentBinding {
                worker_kernel_id: "worker".into(),
                worker_machine_id: "machine".into(),
                execution_lease_id: "lease".into(),
                leased_agent_id: "leased".into(),
                active_worker_provider_run_id: Some("run".into()),
                relay_url: None,
                relay_token: None,
                relay_peer_protocol_version: Some(139),
            },
        )
        .unwrap();
    let event = RelayPeerEvent::LeasedRuntimeProjection {
        home_session_id: session.id().into(),
        home_agent_id: agent.id().into(),
        provider_run_id: "run".into(),
        provider_run: None,
        delivery: Some(RelayProjectionDelivery {
            execution_lease_id: "lease".into(),
            leased_agent_id: "leased".into(),
            sequence: 1,
        }),
        prompts: vec![],
        output_chunks: vec![RelayProjectedOutputChunk {
            kind: TerminalOutputKind::ProviderOutput,
            merge_key: Some("message".into()),
            bytes: b"exactly once".to_vec(),
        }],
        notices: vec![],
        completions: vec![],
    };
    let accepted = RemoteLeaseRuntime::new(&mut app)
        .project_remote_runtime_projection(
            crate::runtime::relay_peer_authority::test_projection_authority("worker"),
            event.clone(),
        )
        .unwrap();
    assert!(accepted.accepted && accepted.acknowledgement.is_some());
    let path = app.operational_history_store().path().to_path_buf();
    app.operational_history = crate::history::OperationalHistoryStore::open(path).unwrap();
    let replay = RemoteLeaseRuntime::new(&mut app)
        .project_remote_runtime_projection(
            crate::runtime::relay_peer_authority::test_projection_authority("worker"),
            event.clone(),
        )
        .unwrap();
    assert!(replay.accepted && replay.acknowledgement.is_some());
    let history = app
        .operational_history_store()
        .query_events(crate::history::HistoryEventQuery {
            session_id: Some(session.id().into()),
            agent_id: Some(agent.id().into()),
            ..Default::default()
        })
        .unwrap();
    assert_eq!(
        history
            .iter()
            .filter(|entry| entry.content.as_deref() == Some("exactly once"))
            .count(),
        1
    );
    for (worker, lease, sequence) in [
        ("other-worker", "lease", 1),
        ("worker", "other-lease", 1),
        ("worker", "lease", 3),
    ] {
        let mut invalid = event.clone();
        let RelayPeerEvent::LeasedRuntimeProjection { delivery, .. } = &mut invalid;
        *delivery = Some(RelayProjectionDelivery {
            execution_lease_id: lease.into(),
            leased_agent_id: "leased".into(),
            sequence,
        });
        assert!(
            !RemoteLeaseRuntime::new(&mut app)
                .project_remote_runtime_projection(
                    crate::runtime::relay_peer_authority::test_projection_authority(worker),
                    invalid
                )
                .unwrap()
                .accepted
        );
    }
}

// MP-08 / MP-10 / MP-11: the receipt can fail after durable history and runtime
// effects. Reopen the real stores, replay the retained page, and count effects.
#[test]
fn mp08_home_projection_receipt_failure_is_restart_safe() {
    home_projection_fault_is_restart_safe("receipt");
}

#[test]
fn mp08_home_projection_partial_application_and_queue_boundary_are_restart_safe() {
    for boundary in ["result", "cleanup", "native", "cleanup-dispatching", "cleanup-delivered"] {
        home_projection_fault_is_restart_safe(boundary);
    }
}

fn home_projection_fault_is_restart_safe(boundary: &str) {
    use crate::git_observer::CompletedTurnSettlementStatus as Status;
    for (final_page, status) in [
        (false, None),
        (true, Some(Status::Completed)),
        (true, Some(Status::Cancelled)),
        (true, Some(Status::Failed)),
    ] {
        let config = crate::config::DaemonConfig::for_tests();
        let mut app = crate::app::DaemonApp::bootstrap(config.clone()).unwrap();
        let (session, agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(crate::session::CreateSessionRequest::new("home", "home"))
            .unwrap();
        app.agents
            .bind_remote_execution(
                agent.id(),
                crate::agent::RemoteAgentBinding {
                    worker_kernel_id: "worker".into(),
                    worker_machine_id: "machine".into(),
                    execution_lease_id: "lease".into(),
                    leased_agent_id: "leased".into(),
                    active_worker_provider_run_id: Some("run".into()),
                    relay_url: None,
                    relay_token: None,
                    relay_peer_protocol_version: Some(139),
                },
            )
            .unwrap();
        let attachment = crate::app::KernelSessionService::new(&mut app)
            .attach(crate::attachment::AttachRequest::new(
                session.id(),
                "client",
                crate::attachment::ClientCapabilityLevel::FullTerminal,
            ))
            .unwrap();
        app.prompt_owner_submit_prepared_prompt(
            session.id(),
            PromptQueueItem::new(
                "original",
                attachment.id(),
                agent.id(),
                "question",
                PromptStatus::Queued,
            )
            .with_durable_operation("original-operation", "original-fingerprint"),
            false,
        )
        .unwrap();
        let original = app
            .prompt_owner_active_prompt_for_agent(session.id(), agent.id())
            .unwrap()
            .unwrap();
        app.prompt_owner_submit_prepared_prompt(
            session.id(),
            PromptQueueItem::new(
                "successor",
                attachment.id(),
                agent.id(),
                "next",
                PromptStatus::Queued,
            ),
            true,
        )
        .unwrap();
        let successor = app
            .prompt_owner_peek_next_queued_prompt(session.id(), agent.id())
            .unwrap()
            .unwrap();
        app.save_durable_state_snapshot().unwrap();
        let event = RelayPeerEvent::LeasedRuntimeProjection {
            home_session_id: session.id().into(),
            home_agent_id: agent.id().into(),
            provider_run_id: "run".into(),
            provider_run: matches!(
                boundary,
                "native" | "cleanup-dispatching" | "cleanup-delivered"
            )
            .then(|| {
                let mut run = super::explicit_completion_tests::provider_run(
                    "run",
                    "worker-session",
                    "worker-agent",
                    "claude",
                    "claude",
                    if boundary == "native" {
                        crate::provider::ProviderClientInterface::NativeTui
                    } else {
                        crate::provider::ProviderClientInterface::Chariox
                    },
                );
                run.mark_running();
                run
            }),
            delivery: Some(RelayProjectionDelivery {
                execution_lease_id: "lease".into(),
                leased_agent_id: "leased".into(),
                sequence: 1,
            }),
            prompts: vec![],
            output_chunks: vec![RelayProjectedOutputChunk {
                kind: TerminalOutputKind::ProviderOutput,
                merge_key: None,
                bytes: b"durable exactly once".to_vec(),
            }],
            notices: vec!["notice exactly once".into()],
            completions: if final_page {
                vec![
                    RelayProjectedCompletion {
                        message_id: "done".into(),
                        completed_at_ms: crate::session::unix_epoch_ms(),
                        home_prompt_id: Some(original.id().into()),
                        settlement_status: status,
                        provider_termination: None,
                    },
                    RelayProjectedCompletion {
                        message_id: "unscoped-other-turn".into(),
                        completed_at_ms: crate::session::unix_epoch_ms() + 2000,
                        home_prompt_id: None,
                        settlement_status: Some(if status == Some(Status::Cancelled) {
                            Status::Failed
                        } else {
                            Status::Cancelled
                        }),
                        provider_termination: Some(
                            crate::provider::ProviderRunTermination::runtime_failure(
                                "unscoped receipt must not own this prompt",
                                crate::session::unix_epoch_ms() + 2000,
                            ),
                        ),
                    },
                ]
            } else {
                vec![]
            },
        };
        // Ensure the delivery tables exist, then fail precisely at receipt save.
        app.operational_history_store()
            .leased_projection_delivery_sequence("probe")
            .unwrap();
        let db = app.operational_history_store().path().to_path_buf();
        let connection = rusqlite::Connection::open(&db).unwrap();
        let trigger = match boundary {
            "receipt" | "native" => "CREATE TRIGGER fail_home_receipt BEFORE INSERT ON leased_projection_delivery_receipts BEGIN SELECT RAISE(FAIL, 'injected receipt failure'); END;",
            "result" => "CREATE TRIGGER fail_home_receipt BEFORE UPDATE ON home_projection_applications BEGIN SELECT RAISE(FAIL, 'injected result failure'); END;",
            "cleanup" | "cleanup-dispatching" | "cleanup-delivered" => "CREATE TRIGGER fail_home_receipt BEFORE DELETE ON home_projection_applications BEGIN SELECT RAISE(FAIL, 'injected cleanup failure'); END;",
            _ => unreachable!(),
        };
        connection.execute_batch(trigger).unwrap();
        let first = RemoteLeaseRuntime::new(&mut app).project_remote_runtime_projection(
            crate::runtime::relay_peer_authority::test_projection_authority("worker"),
            event.clone(),
        );
        assert!(first.is_err(), "{boundary} failure must propagate");
        if boundary == "receipt" && final_page {
            assert!(
                app.prompt_owner_active_prompt_for_agent(session.id(), agent.id())
                    .unwrap()
                    .is_none(),
                "successor admission must stay after the receipt"
            );
            assert_eq!(
                app.prompt_owner_peek_next_queued_prompt(session.id(), agent.id())
                    .unwrap()
                    .unwrap()
                    .id(),
                successor.id()
            );
        }
        if matches!(boundary, "cleanup-dispatching" | "cleanup-delivered") && final_page {
            let phase = if boundary == "cleanup-dispatching" {
                crate::session::DurablePromptDeliveryPhase::Dispatching
            } else {
                crate::session::DurablePromptDeliveryPhase::Delivered
            };
            app.mark_active_prompt_delivery(
                session.id(),
                agent.id(),
                successor.id(),
                phase,
                Some("successor-run".into()),
                None,
            )
            .unwrap();
            app.agents
                .set_remote_execution_active_worker_provider_run_id(
                    agent.id(),
                    Some("successor-run".into()),
                )
                .unwrap();
        }
        if boundary.starts_with("cleanup") && final_page {
            // A successor may already start before an old journal cleanup retry.
            app.agents
                .set_agent_state(agent.id(), crate::agent::AgentState::Working)
                .unwrap();
            app.save_durable_state_snapshot().unwrap();
        }
        connection
            .execute_batch("DROP TRIGGER fail_home_receipt")
            .unwrap();
        drop(connection);
        assert!(
            !RemoteLeaseRuntime::new(&mut app)
                .project_remote_runtime_projection(
                    crate::runtime::relay_peer_authority::test_projection_authority(
                        "foreign-worker"
                    ),
                    event.clone(),
                )
                .unwrap()
                .accepted,
            "journal cannot replace worker admission"
        );
        if boundary != "receipt" {
            let mut changed = event.clone();
            let RelayPeerEvent::LeasedRuntimeProjection { output_chunks, .. } = &mut changed;
            output_chunks[0].bytes = b"altered retained page".to_vec();
            assert!(
                RemoteLeaseRuntime::new(&mut app)
                    .project_remote_runtime_projection(
                        crate::runtime::relay_peer_authority::test_projection_authority("worker"),
                        changed,
                    )
                    .is_err(),
                "journal must bind the exact retained bytes"
            );
        }
        drop(app);
        let mut app = crate::app::DaemonApp::bootstrap(config).unwrap();
        let recovered = RemoteLeaseRuntime::new(&mut app)
            .project_remote_runtime_projection(
                crate::runtime::relay_peer_authority::test_projection_authority("worker"),
                event.clone(),
            )
            .unwrap();
        assert!(
            recovered.accepted && recovered.acknowledgement.is_some(),
            "pending page must finish after restart"
        );
        let history = app
            .operational_history_store()
            .query_events(crate::history::HistoryEventQuery {
                session_id: Some(session.id().into()),
                agent_id: Some(agent.id().into()),
                ..Default::default()
            })
            .unwrap();
        for text in ["durable exactly once", "notice exactly once"] {
            assert_eq!(
                history
                    .iter()
                    .filter(|entry| entry.content.as_deref() == Some(text))
                    .count(),
                1,
                "{text}: final={final_page}"
            );
        }
        if final_page {
            let settled = app
                .completed_git_turn_snapshot_store()
                .projection_for_prompt(session.id(), agent.id(), original.id())
                .expect("restart restores the exact runtime settlement");
            assert_eq!(settled.settlement_status, status.unwrap());
            assert!(
                settled.provider_termination.is_none(),
                "only the accepted worker outcome may be restored"
            );
        }
        let settlements = history
            .iter()
            .filter(|event| {
                event
                    .metadata
                    .contains_key(crate::history::PROMPT_SETTLEMENT_STATUS_METADATA_KEY)
            })
            .count();
        assert_eq!(
            settlements,
            usize::from(final_page),
            "one durable settlement: {boundary}"
        );
        assert_eq!(
            recovered.remote_dispatches.len(),
            usize::from(
                final_page && !matches!(boundary, "cleanup-dispatching" | "cleanup-delivered")
            ),
            "one successor admission: {boundary}"
        );
        let active = app
            .prompt_owner_active_prompt_for_agent(session.id(), agent.id())
            .unwrap()
            .unwrap();
        assert_eq!(
            active.id(),
            if final_page {
                successor.id()
            } else {
                original.id()
            }
        );
        if matches!(boundary, "cleanup-dispatching" | "cleanup-delivered") && final_page {
            assert_eq!(
                app.agents
                    .get_agent(agent.id())
                    .unwrap()
                    .remote_execution()
                    .unwrap()
                    .active_worker_provider_run_id
                    .as_deref(),
                Some("successor-run"),
                "an old journal cannot select its old run over the successor"
            );
        }
        if boundary == "native" {
            let run_id = crate::provider::projected_leased_provider_run_id("leased", "run");
            let run = app
                .provider_run_projection
                .get(&run_id)
                .expect("ready journal must restore the retained worker run snapshot");
            assert_eq!(run.state(), crate::provider::ProviderRunState::Running);
            assert_eq!(run.agent_instance_id(), Some(agent.id()));
            assert_eq!(
                app.sessions
                    .get_session(session.id())
                    .unwrap()
                    .active_provider_run_id(),
                Some(run_id.as_str())
            );
            assert_eq!(
                app.agents
                    .get_agent(agent.id())
                    .unwrap()
                    .remote_execution()
                    .unwrap()
                    .active_worker_provider_run_id
                    .as_deref(),
                Some("run"),
                "ready journal must retain the running native binding after restart"
            );
        }
        if boundary == "cleanup" && final_page {
            assert_ne!(
                app.agents.get_agent(agent.id()).unwrap().state(),
                crate::agent::AgentState::Error,
                "old failed receipt must not mark a successor Error"
            );
        }
        let duplicate = RemoteLeaseRuntime::new(&mut app)
            .project_remote_runtime_projection(
                crate::runtime::relay_peer_authority::test_projection_authority("worker"),
                event,
            )
            .unwrap();
        assert!(duplicate.replayed);
        assert!(duplicate.remote_dispatches.is_empty());
        assert_eq!(
            app.prompt_owner_active_prompt_for_agent(session.id(), agent.id())
                .unwrap()
                .unwrap()
                .id(),
            active.id()
        );
    }
}

#[test]
fn mp08_review_f6_cleanup_reemits_the_exact_unsent_successor() {
    home_projection_fault_is_restart_safe("cleanup");
}

#[test]
fn mp08_review_f6_ready_journal_restores_worker_run_snapshot() {
    home_projection_fault_is_restart_safe("native");
}
