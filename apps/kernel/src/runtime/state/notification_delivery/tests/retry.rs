//! MP-08 / MP-10 / MP-11: explicit resend identities and durable worker receipts.
use super::*;
use crate::app::{DaemonApp, RemoteLeaseRuntime};
use crate::transport::relay_peer::{LeasedPromptReceiptPhase, RemoteGitTurnContext};

fn worker_target(
    worker: &mut DaemonApp,
    session: &str,
    agent_id: &str,
    prompt_id: &str,
) -> (
    crate::execution_lease::ExecutionLease,
    crate::execution_lease::LeasedAgent,
    String,
) {
    let lease = RemoteLeaseRuntime::new(worker)
        .create_execution_lease("home", session, agent_id, false, "local")
        .unwrap();
    let leased = RemoteLeaseRuntime::new(worker)
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
    let (run, _) = RemoteLeaseRuntime::new(worker)
        .submit_leased_prompt_with_workflow_context(
            &leased.id,
            "active target",
            Vec::new(),
            None,
            Some(RemoteGitTurnContext {
                home_session_id: session.into(),
                home_agent_id: agent_id.into(),
                home_prompt_id: prompt_id.into(),
                home_turn_id: prompt_id.into(),
                source_attachment_id: None,
                workspace_live_sync_mode: None,
                prompt_origin: None,
                external_provider: None,
                external_provider_session_id: None,
                external_provider_turn_id: None,
                prompt_summary: "active target".into(),
            }),
            Vec::new(),
            None,
            crate::extension::RemoteExtensionManifest::default(),
        )
        .unwrap();
    (lease, leased, run)
}

#[tokio::test]
async fn explicit_retry_has_a_fresh_identity_after_worker_dispatch_receipt_restart() {
    for restart_worker in [false, true] {
        let (_root, runtime, session, id) = fixture(false, 1).await;
        let snapshot = runtime.owned.session_store.get_session(&session).unwrap();
        let agent_id = snapshot.workflow_runs()[0].node_runs()[0]
            .agent_id()
            .to_owned();
        let active = runtime
            .owned
            .prompt_state_owner
            .active_prompt_for_agent(&snapshot, &agent_id)
            .unwrap();
        let mut config = crate::config::DaemonConfig::for_tests();
        config.accept_remote_leases = true;
        let mut worker = DaemonApp::bootstrap(config).unwrap();
        let (lease, leased, worker_run) =
            worker_target(&mut worker, &session, &agent_id, active.id());
        let mut binding = crate::agent::RemoteAgentBinding {
            worker_kernel_id: worker.config().daemon_id.clone(),
            worker_machine_id: "machine".into(),
            leased_agent_id: leased.id.clone(),
            execution_lease_id: lease.id.clone(),
            active_worker_provider_run_id: Some(worker_run.clone()),
            relay_url: None,
            relay_token: None,
            relay_peer_protocol_version: Some(
                crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION,
            ),
        };
        runtime
            .owned
            .agent_store
            .bind_remote_execution(&agent_id, binding.clone())
            .unwrap();
        let original = runtime
            .owned
            .prepare_notification_injection(&session, &id)
            .unwrap()
            .unwrap();
        runtime
            .owned
            .mark_notification_submit(&session, &original, Some(&binding))
            .unwrap();
        RemoteLeaseRuntime::new(&mut worker)
            .reserve_leased_prompt_steer(
                &leased.id,
                &original.dispatch.prompt_id,
                active.id(),
                &worker_run,
            )
            .unwrap();
        if restart_worker {
            let config = worker.config().clone();
            drop(worker);
            worker = DaemonApp::bootstrap(config).unwrap();
        }
        let receipt = RemoteLeaseRuntime::new(&mut worker)
            .reconcile_leased_prompt_steer_receipt(
                &leased.id,
                &original.dispatch.prompt_id,
                active.id(),
                &worker_run,
                &lease.id,
            )
            .unwrap();
        assert_eq!(receipt.phase, LeasedPromptReceiptPhase::SteerDispatching);
        runtime
            .owned
            .settle_notification_remote_receipt(&session, &id, &receipt)
            .unwrap();
        assert_eq!(
            queued_item(&runtime, &session, &id).status(),
            WorkflowQueuedPromptStatus::Running
        );

        // Full worker bootstrap preserves receipts but drops execution leases.
        // A reattached target uses a new lease; the live-worker case above must
        // also pass the unresolved receipt fence on the original lease.
        let target = if restart_worker {
            worker_target(&mut worker, &session, &agent_id, active.id())
        } else {
            (lease.clone(), leased.clone(), worker_run.clone())
        };
        binding.leased_agent_id = target.1.id.clone();
        binding.execution_lease_id = target.0.id.clone();
        binding.active_worker_provider_run_id = Some(target.2.clone());
        runtime
            .owned
            .agent_store
            .bind_remote_execution(&agent_id, binding.clone())
            .unwrap();
        let response = runtime
            .owned
            .workflow_update_queued_prompt(UpdateQueuedWorkflowPromptRequest {
                session_id: session.clone(),
                queue_item_ref: id.clone(),
                prompt: None,
                queue_ref: None,
                retry_delivery: true,
            })
            .unwrap();
        assert!(matches!(
            response,
            LocalDaemonResponse::QueuedWorkflowPromptUpdated { .. }
        ));
        let retry = runtime
            .owned
            .prepare_notification_injection(&session, &id)
            .unwrap()
            .unwrap();
        runtime
            .owned
            .mark_notification_submit(&session, &retry, Some(&binding))
            .unwrap();
        let (_, dispatch) = RemoteLeaseRuntime::new(&mut worker)
            .prepare_leased_prompt_steer(
                &target.1.id,
                &retry.dispatch.prompt_id,
                active.id(),
                retry.queued.prompt().unwrap(),
                "",
                Vec::new(),
                None,
            )
            .expect("explicit retry must pass the worker's persisted Dispatching receipt fence");
        assert!(dispatch.is_some());
        assert_ne!(original.dispatch.prompt_id, retry.dispatch.prompt_id);
        assert_eq!(
            retry.queued.id(),
            id,
            "retry retains its durable queue identity"
        );
        RemoteLeaseRuntime::new(&mut worker)
            .reserve_leased_prompt_steer(
                &target.1.id,
                &retry.dispatch.prompt_id,
                active.id(),
                &target.2,
            )
            .unwrap();
        RemoteLeaseRuntime::new(&mut worker)
            .mark_leased_prompt_steer_accepted(
                &target.1.id,
                &retry.dispatch.prompt_id,
                active.id(),
                &target.2,
            )
            .unwrap();
        let accepted = RemoteLeaseRuntime::new(&mut worker)
            .reconcile_leased_prompt_steer_receipt(
                &target.1.id,
                &retry.dispatch.prompt_id,
                active.id(),
                &target.2,
                &target.0.id,
            )
            .unwrap();
        assert!(
            runtime
                .owned
                .settle_notification_remote_receipt(&session, &id, &receipt)
                .is_err(),
            "a late receipt for the old attempt cannot settle the retry"
        );
        runtime
            .owned
            .settle_notification_remote_receipt(&session, &id, &accepted)
            .unwrap();
        assert_eq!(
            queued_item(&runtime, &session, &id).status(),
            WorkflowQueuedPromptStatus::Completed
        );
        let old = RemoteLeaseRuntime::new(&mut worker)
            .reconcile_leased_prompt_steer_receipt(
                &leased.id,
                &original.dispatch.prompt_id,
                active.id(),
                &worker_run,
                &lease.id,
            )
            .unwrap();
        assert_eq!(
            old.phase,
            LeasedPromptReceiptPhase::SteerDispatching,
            "explicit retry does not rewrite the unresolved original receipt"
        );
    }
}

#[tokio::test]
async fn explicit_retry_settles_only_its_current_local_send_attempt() {
    let (_root, runtime, session, id) = held_notification().await;
    let mut old_ids = vec![id.clone()];
    for _ in 0..2 {
        runtime
            .owned
            .resolve_held_notification_delivery(&session, &id, true)
            .unwrap()
            .unwrap();
        let retry = runtime
            .owned
            .prepare_notification_injection(&session, &id)
            .unwrap()
            .unwrap();
        runtime
            .owned
            .mark_notification_submit(&session, &retry, None)
            .unwrap();
        let finished = |prompt_id: String| crate::provider::FinishedProviderPromptSubmitJob {
            session_id: session.clone(),
            provider_run_id: retry.dispatch.provider_run_id.clone(),
            agent_id: retry.dispatch.agent_id.clone(),
            prompt_id,
            result: Ok(crate::provider::ProviderPromptSubmitAcknowledgement {
                resume_state: crate::provider::ProviderResumeState::default(),
            }),
            settlement_retry_attempt: 0,
        };
        for old in &old_ids {
            assert!(finish_structured_notification_submit(
                &runtime.owned.durable_state_store,
                &runtime.owned.session_store,
                &finished(old.clone()),
            )
            .unwrap());
            assert_eq!(
                queued_item(&runtime, &session, &id).status(),
                WorkflowQueuedPromptStatus::Running,
                "late acknowledgement for an older attempt cannot settle the current send"
            );
        }
        old_ids.push(retry.dispatch.prompt_id.clone());
    }
    let item = queued_item(&runtime, &session, &id);
    let current = item.publication_invocation().unwrap().caller["notification_send_id"]
        .as_str()
        .unwrap();
    let saved = &item.publication_invocation().unwrap().caller["notification_steer"];
    let finished = crate::provider::FinishedProviderPromptSubmitJob {
        session_id: session.clone(),
        provider_run_id: saved["provider_run_id"].as_str().unwrap().into(),
        agent_id: saved["agent_id"].as_str().unwrap().into(),
        prompt_id: current.into(),
        result: Ok(crate::provider::ProviderPromptSubmitAcknowledgement {
            resume_state: Default::default(),
        }),
        settlement_retry_attempt: 0,
    };
    assert!(finish_structured_notification_submit(
        &runtime.owned.durable_state_store,
        &runtime.owned.session_store,
        &finished,
    )
    .unwrap());
    assert_eq!(
        queued_item(&runtime, &session, &id).status(),
        WorkflowQueuedPromptStatus::Completed
    );
}
