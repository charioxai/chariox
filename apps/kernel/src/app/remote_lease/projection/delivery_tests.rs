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
