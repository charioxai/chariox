// MP-08 / MP-10 / MP-11: admission rejection must not strand a home prompt.
use super::*;
use crate::app::RemoteLeaseRuntime;
use crate::transport::relay_peer::{
    LeasedPromptReceiptPhase, RelayAgentExecutionProfile, RemoteGitTurnContext,
};

#[tokio::test]
async fn leased_admission_replay_failure_has_a_definitive_rejection_receipt() {
    let mut config = crate::DaemonConfig::for_tests();
    config.accept_remote_leases = true;
    let mut app = DaemonApp::bootstrap(config).unwrap();
    let lease = RemoteLeaseRuntime::new(&mut app)
        .create_execution_lease("home", "room", "agent", false, "owner")
        .unwrap();
    let leased = RemoteLeaseRuntime::new(&mut app)
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
    let context = RemoteGitTurnContext {
        home_session_id: "room".into(),
        home_agent_id: "agent".into(),
        home_prompt_id: "home-prompt-replay".into(),
        home_turn_id: "home-prompt-replay".into(),
        source_attachment_id: None,
        workspace_live_sync_mode: None,
        prompt_origin: None,
        external_provider: None,
        external_provider_session_id: None,
        external_provider_turn_id: None,
        prompt_summary: "receipt regression".into(),
    };
    RemoteLeaseRuntime::new(&mut app)
        .submit_leased_prompt_with_workflow_context(
            &leased.id,
            "already settled backing turn",
            Vec::new(),
            None,
            Some(context.clone()),
            Vec::new(),
            None,
            crate::extension::RemoteExtensionManifest::default(),
        )
        .unwrap();
    // A completion can remove the backing prompt before the submission retry
    // reaches the worker. This is before any new provider input is sent.
    app.prompt_owner_cancel_active_prompt_only(
        &leased.backing_session_id,
        &leased.backing_agent_id,
    )
    .unwrap();
    let app = Arc::new(Mutex::new(app));
    let runtime = owned_runtime_state(&app).await;
    let result = runtime
        .submit_relay_leased_prompt(
            &leased.id,
            RelayAgentExecutionProfile {
                provider: leased.provider.clone(),
                account_profile: leased.account_profile.clone(),
                model: leased.model.clone(),
                effort: leased.effort.clone(),
            },
            "retry exact home prompt",
            "",
            Vec::new(),
            None,
            Some(context),
            Vec::new(),
            None,
            crate::extension::RemoteExtensionManifest::default(),
            None,
        )
        .await;
    assert!(
        matches!(
            &result,
            Err(DaemonError::LocalTransport {
                operation: "replay active leased prompt submission",
                ..
            })
        ),
        "expected exact replay rejection, got {result:?}"
    );
    let receipt = runtime
        .query_relay_leased_prompt_receipt(&leased.id, "home-prompt-replay")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        receipt.phase,
        LeasedPromptReceiptPhase::SteerRejected,
        "a proven pre-admission replay failure must not leave Dispatching forever"
    );
    assert!(receipt.worker_provider_run_id.is_empty());
    assert!(receipt.target_home_prompt_id.is_none());
    assert_eq!(
        receipt.execution_lease_id.as_deref(),
        Some(lease.id.as_str())
    );
}

#[tokio::test]
async fn leased_admission_commits_before_async_io_and_dispatch_failure_settles_failed() {
    let mut config = crate::DaemonConfig::for_tests();
    config.accept_remote_leases = true;
    let mut app = DaemonApp::bootstrap(config).unwrap();
    let lease = RemoteLeaseRuntime::new(&mut app)
        .create_execution_lease("home", "room", "agent", false, "owner")
        .unwrap();
    let leased = RemoteLeaseRuntime::new(&mut app)
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
    let prepared = RemoteLeaseRuntime::new(&mut app)
        .prepare_leased_prompt_submission(
            &leased.id,
            "not admitted yet",
            "",
            Vec::new(),
            None,
            None,
            Vec::new(),
            None,
            crate::extension::RemoteExtensionManifest::default(),
        )
        .unwrap();
    let crate::app::PreparedLeasedProviderRun::LaunchRequired(request) = prepared.provider_run
    else {
        panic!("fresh worker should launch its provider")
    };
    let run_id = app
        .launch_leased_provider(request)
        .unwrap()
        .id()
        .to_string();
    let app = Arc::new(Mutex::new(app));
    let runtime = owned_runtime_state(&app).await;
    // Hold the ordinary provider lane while admitting the new prompt. No
    // provider I/O may happen under the worker admission/app lock.
    let lane = runtime.provider_runtime_lanes.acquire(&run_id).await;
    let (_, outcome) = tokio::time::timeout(
        std::time::Duration::from_secs(1),
        runtime.submit_relay_leased_prompt(
            &leased.id,
            RelayAgentExecutionProfile {
                provider: leased.provider.clone(),
                account_profile: leased.account_profile.clone(),
                model: leased.model.clone(),
                effort: leased.effort.clone(),
            },
            "new admitted turn",
            "",
            Vec::new(),
            None,
            Some(RemoteGitTurnContext {
                home_session_id: "room".into(),
                home_agent_id: "agent".into(),
                home_prompt_id: "home-dispatch-failure".into(),
                home_turn_id: "home-dispatch-failure".into(),
                source_attachment_id: None,
                workspace_live_sync_mode: None,
                prompt_origin: None,
                external_provider: None,
                external_provider_session_id: None,
                external_provider_turn_id: None,
                prompt_summary: "admission regression".into(),
            }),
            Vec::new(),
            None,
            crate::extension::RemoteExtensionManifest::default(),
            None,
        ),
    )
    .await
    .expect("admission must not wait on provider I/O")
    .unwrap();
    let crate::session::PromptSubmissionOutcome::Started { prompt } = outcome else {
        panic!("idle turn should start")
    };
    assert_ne!(
        prompt.durable_delivery_phase(),
        Some(crate::session::DurablePromptDeliveryPhase::Delivered),
        "an admission receipt must not assert provider delivery before the provider lane runs"
    );
    let receipt = runtime
        .with_app_side_effect(|app| {
            RemoteLeaseRuntime::new(app).leased_prompt_receipt(&leased.id, "home-dispatch-failure")
        })
        .await
        .unwrap()
        .unwrap();
    assert_eq!(receipt.phase, LeasedPromptReceiptPhase::Active);
    assert_eq!(receipt.worker_provider_run_id, run_id);
    // An injected provider input error uses the same dispatcher finalizer;
    // the held lane prevents any synthetic provider from sending input first.
    let failed = runtime
        .fail_prompt_dispatch(
            crate::app::KernelPromptDispatch {
                session_id: leased.backing_session_id.clone(),
                provider_run_id: run_id.clone(),
                agent_id: leased.backing_agent_id.clone(),
                prompt_id: prompt.id().to_string(),
                target_active_prompt_id: None,
                source_attachment_id: leased.backing_attachment_id.clone(),
                prompt: prompt.prompt().to_string(),
                hidden_system_context: String::new(),
                attachments: Vec::new(),
                prompt_origin: crate::session::PromptOrigin::Chariox,
                external_provider: None,
                external_provider_session_id: None,
                external_provider_turn_id: None,
                steering: false,
            },
            DaemonError::LocalTransport {
                operation: "injected leased provider input failure",
                message: "supplementary receipt regression".into(),
            },
        )
        .await;
    assert!(failed.is_err());
    drop(lane);
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(2);
    loop {
        let snapshot = runtime
            .owned
            .session_snapshot(&leased.backing_session_id)
            .unwrap();
        if snapshot
            .active_prompt_for_agent(&leased.backing_agent_id)
            .is_none()
        {
            let activity = runtime.agent_activity_for_session(&snapshot);
            let settled = activity[&leased.backing_agent_id]
                .last_completed_turn
                .as_ref()
                .unwrap();
            assert_eq!(settled.prompt_id, prompt.id());
            assert_eq!(
                settled.settlement_status,
                crate::git_observer::CompletedTurnSettlementStatus::Failed
            );
            break;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "a shared dispatch error must settle the admitted turn"
        );
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
}
