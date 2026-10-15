//! Home-side recorded worker events, with no local provider process or run.
use super::*;
use crate::session::{
    DurablePromptDeliveryPhase, NativeInteractionOrigin, PromptQueueItem, PromptStatus,
};
use crate::transport::relay_peer::RemoteNativeInteractionContext;

async fn home(current_worker: Option<&str>, prompt: bool) -> Fixture {
    let worktree = crate::test_support::TestWorktree::new("approval-remote-home");
    let mut app = DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests()).unwrap();
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(worktree.session_request())
        .unwrap();
    let attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(crate::attachment::AttachRequest::new(
            session.id(),
            "remote-test",
            crate::attachment::ClientCapabilityLevel::FullTerminal,
        ))
        .unwrap();
    app.agents
        .bind_remote_execution(
            agent.id(),
            crate::agent::RemoteAgentBinding {
                worker_kernel_id: "worker".into(),
                worker_machine_id: "machine-worker".into(),
                execution_lease_id: "execution-lease".into(),
                leased_agent_id: "leased-agent".into(),
                active_worker_provider_run_id: current_worker.map(str::to_owned),
                relay_url: None,
                relay_token: None,
                relay_peer_protocol_version: Some(
                    crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION,
                ),
            },
        )
        .unwrap();
    let runtime = owned_runtime_state(&Arc::new(Mutex::new(app))).await;
    if prompt {
        runtime
            .owned
            .prompt_state_owner
            .activate_prompt(
                &session,
                PromptQueueItem::new(
                    "home-A",
                    attachment.id(),
                    agent.id(),
                    "remote A",
                    PromptStatus::Running,
                ),
            )
            .unwrap();
        runtime
            .owned
            .prompt_state_owner
            .mark_active_prompt_delivery(
                &session,
                agent.id(),
                "home-A",
                DurablePromptDeliveryPhase::Dispatching,
                None,
                None,
            )
            .unwrap();
    }
    assert!(runtime
        .owned
        .provider_store
        .get_run_for_agent(session.id(), agent.id())
        .is_none());
    Fixture {
        runtime,
        session: session.id().into(),
        agent: agent.id().into(),
        attachment: attachment.id().into(),
        run: "worker-run".into(),
        _worktree: worktree,
    }
}
fn context(f: &Fixture, prompt: bool) -> RemoteNativeInteractionContext {
    RemoteNativeInteractionContext {
        home_session_id: f.session.clone(),
        home_agent_id: f.agent.clone(),
        leased_agent_id: "leased-agent".into(),
        worker_provider_run_id: f.run.clone(),
        home_prompt_id: prompt.then(|| "home-A".into()),
    }
}
async fn register(
    f: &Fixture,
    prompt: bool,
) -> tokio::sync::oneshot::Receiver<PendingInteractionResolution> {
    let origin = if prompt {
        NativeInteractionOrigin::Prompt {
            provider_run_id: f.run.clone(),
            prompt_id: "home-A".into(),
        }
    } else {
        NativeInteractionOrigin::ProviderStartup {
            provider_run_id: f.run.clone(),
        }
    };
    f.runtime
        .create_runtime_interaction_with_forwarding(
            &f.session,
            native_approval(f, Some(origin)),
            Some(&context(f, prompt)),
        )
        .await
        .unwrap()
}
fn bind(f: &Fixture, id: Option<&str>) {
    f.runtime
        .owned
        .agent_store
        .set_remote_execution_active_worker_provider_run_id(&f.agent, id.map(str::to_owned))
        .unwrap();
}
fn assert_pending(
    f: &Fixture,
    receiver: &mut tokio::sync::oneshot::Receiver<PendingInteractionResolution>,
) {
    f.runtime.owned.withdraw_stale_agent_interactions();
    assert!(matches!(
        receiver.try_recv(),
        Err(tokio::sync::oneshot::error::TryRecvError::Empty)
    ));
    assert_eq!(
        f.runtime
            .owned
            .session_snapshot(&f.session)
            .unwrap()
            .active_interactions()
            .len(),
        1
    );
}
#[tokio::test]
async fn approval_lifetime_remote_prompt_before_ack_survives_binding_and_answers() {
    for previous in [None, Some("previous-worker-run")] {
        let f = home(previous, true).await;
        let mut rx = register(&f, true).await;
        assert_pending(&f, &mut rx);
        bind(&f, Some(&f.run));
        let session = f
            .runtime
            .owned
            .session_store
            .get_session(&f.session)
            .unwrap();
        f.runtime
            .owned
            .prompt_state_owner
            .mark_active_prompt_delivery(
                &session,
                &f.agent,
                "home-A",
                DurablePromptDeliveryPhase::Delivered,
                Some(f.run.clone()),
                None,
            )
            .unwrap();
        assert_pending(&f, &mut rx);
        f.runtime
            .resolve_runtime_interaction(&f.session, &f.id(), "allow", None)
            .await
            .unwrap();
        assert_eq!(rx.try_recv().unwrap().choice_id.as_deref(), Some("allow"));
    }
}
#[tokio::test]
async fn approval_lifetime_remote_startup_before_binding_is_answerable() {
    let f = home(None, false).await;
    let mut rx = register(&f, false).await;
    assert_pending(&f, &mut rx);
    f.runtime
        .resolve_runtime_interaction(&f.session, &f.id(), "allow", None)
        .await
        .unwrap();
    assert_eq!(rx.try_recv().unwrap().status, "answered");
}
// MP-08 / MP-10 / MP-11: Historical projections cannot deny a new launch approval.
#[tokio::test]
async fn approval_lifetime_remote_startup_before_binding_with_ended_history_is_answerable() {
    for observe_binding in [false, true] {
        let f = home(None, false).await;
        project_worker_run(&f, "previous-worker-run");
        let mut old = f
            .runtime
            .owned
            .provider_store
            .get_latest_run_for_agent(&f.session, &f.agent)
            .unwrap();
        old.mark_ended();
        f.runtime
            .owned
            .provider_store
            .write()
            .insert_run_for_test(old);
        let mut rx = register(&f, false).await;
        assert_pending(&f, &mut rx);
        if observe_binding {
            bind(&f, Some(&f.run));
            project_worker_run(&f, &f.run);
            assert_pending(&f, &mut rx);
        }
        f.runtime
            .resolve_runtime_interaction(&f.session, &f.id(), "allow", None)
            .await
            .unwrap();
        assert_eq!(rx.try_recv().unwrap().status, "answered");
    }
}

#[tokio::test]
async fn approval_lifetime_remote_startup_during_replacement_dispatch_expires_on_cancel() {
    let f = home(Some("previous-worker-run"), true).await;
    let mut rx = register(&f, false).await;
    assert_pending(&f, &mut rx);
    f.runtime
        .owned
        .cancel_active_prompt_only(&f.session, &f.agent)
        .unwrap();
    f.assert_withdrawn(&mut rx).await;
}
#[tokio::test]
async fn approval_lifetime_remote_old_callback_is_refused_during_followup_dispatch() {
    let f = home(None, true).await;
    let old_context = context(&f, true);
    let interaction = native_approval(
        &f,
        Some(NativeInteractionOrigin::Prompt {
            provider_run_id: f.run.clone(),
            prompt_id: "worker-A".into(),
        }),
    );
    f.runtime
        .owned
        .cancel_active_prompt_only(&f.session, &f.agent)
        .unwrap();
    let session = f
        .runtime
        .owned
        .session_store
        .get_session(&f.session)
        .unwrap();
    f.runtime
        .owned
        .prompt_state_owner
        .activate_prompt(
            &session,
            PromptQueueItem::new(
                "home-B",
                &f.attachment,
                &f.agent,
                "B",
                PromptStatus::Running,
            ),
        )
        .unwrap();
    f.runtime
        .owned
        .prompt_state_owner
        .mark_active_prompt_delivery(
            &session,
            &f.agent,
            "home-B",
            DurablePromptDeliveryPhase::Dispatching,
            None,
            None,
        )
        .unwrap();
    let resolution = crate::runtime::native_interaction_bridge::forward_relay_native_interaction(
        &f.runtime,
        old_context,
        interaction,
    )
    .await
    .unwrap();
    assert_eq!(resolution.status, "timed_out");
    assert!(resolution.choice_id.is_none() && resolution.reply.is_none());
    assert!(f
        .runtime
        .resolve_runtime_interaction(&f.session, &f.id(), "allow", None)
        .await
        .is_err());
}
#[tokio::test]
async fn approval_lifetime_remote_binding_mismatch_or_lease_replacement_refuses_late_answer() {
    for change_lease in [false, true] {
        let f = home(Some("worker-run"), false).await;
        let mut rx = register(&f, false).await;
        assert_pending(&f, &mut rx);
        if change_lease {
            let mut remote = f
                .runtime
                .owned
                .agent_store
                .get_agent(&f.agent)
                .unwrap()
                .remote_execution()
                .unwrap()
                .clone();
            remote.execution_lease_id = "replacement-lease".into();
            f.runtime
                .owned
                .agent_store
                .bind_remote_execution(&f.agent, remote)
                .unwrap();
        } else {
            bind(&f, Some("replacement-worker-run"));
        }
        assert!(f
            .runtime
            .resolve_runtime_interaction(&f.session, &f.id(), "allow", None)
            .await
            .is_err());
        f.assert_withdrawn(&mut rx).await;
    }
}
#[tokio::test]
async fn approval_lifetime_remote_projected_exit_refuses_answer_without_raw_worker_run() {
    let f = home(Some("worker-run"), false).await;
    let mut rx = register(&f, false).await;
    let request = crate::provider::LaunchProviderRequest::new(
        &f.session,
        "dev-stub",
        "claude-code",
        "default",
        "sonnet",
    )
    .with_agent_id(&f.agent);
    let mut run = crate::provider::RuntimeProviderRun::new(
        crate::provider::projected_leased_provider_run_id("leased-agent", &f.run),
        &request,
        crate::provider::ProviderLaunchResult {
            endpoint_mode: crate::provider::AgentEndpointMode::Managed,
            process_label: "recorded-exit".into(),
            pty_target: None,
            pty_program: None,
            pty_args: Vec::new(),
            pty_env: Default::default(),
            pty_env_remove: Vec::new(),
            working_directory: None,
            structured_endpoint: None,
        },
    );
    run.mark_ended();
    f.runtime
        .owned
        .provider_store
        .write()
        .insert_run_for_test(run);
    assert!(f.runtime.owned.provider_store.get_run(&f.run).is_err());
    assert!(f
        .runtime
        .resolve_runtime_interaction(&f.session, &f.id(), "allow", None)
        .await
        .is_err());
    f.assert_withdrawn(&mut rx).await;
}

#[tokio::test]
async fn approval_lifetime_remote_cleared_binding_cannot_reopen_pre_ack_allowance() {
    let f = home(None, false).await;
    let mut rx = register(&f, false).await;
    assert_pending(&f, &mut rx);
    bind(&f, Some(&f.run));
    assert_pending(&f, &mut rx);
    bind(&f, None);
    assert!(f
        .runtime
        .resolve_runtime_interaction(&f.session, &f.id(), "allow", None)
        .await
        .is_err());
    f.assert_withdrawn(&mut rx).await;
}

// MP-08 / MP-10 / MP-11: Environment Adjust belongs to the home even when idle.
#[tokio::test]
async fn mp08_mp10_mp11_idle_worker_review_has_home_target() {
    let mut config = crate::config::DaemonConfig::for_tests();
    config.accept_remote_leases = true;
    let mut app = DaemonApp::bootstrap(config).unwrap();
    let lease = crate::app::RemoteLeaseRuntime::new(&mut app)
        .create_execution_lease("home-kernel", "home-room", "home-agent", false, "home-user")
        .unwrap();
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
        .unwrap();
    let (run, _) = crate::app::RemoteLeaseRuntime::new(&mut app)
        .submit_leased_prompt(&leased.id, "fixture task", Vec::new())
        .unwrap();
    let runtime = owned_runtime_state(&Arc::new(Mutex::new(app))).await;
    runtime
        .owned
        .complete_local_prompt_without_advance(
            &leased.backing_session_id,
            &leased.backing_agent_id,
            Some(&run),
        )
        .unwrap();
    runtime.owned.active_turns.clear(&run);
    let target = runtime
        .leased_interaction_home_target(&leased.backing_session_id, &leased.backing_agent_id, None)
        .await
        .unwrap();
    runtime
        .owned
        .provider_store
        .terminate_run_provider_only(&leased.backing_session_id, &run)
        .unwrap();
    let (_, home, context, _) = target.expect("idle kernel review must reach the home");
    assert_eq!(home, "home-kernel");
    assert_eq!(context.worker_provider_run_id, run);
    assert!(runtime
        .leased_interaction_home_target(&leased.backing_session_id, &leased.backing_agent_id, None)
        .await
        .unwrap()
        .is_none());
}

fn project_worker_run(f: &Fixture, worker_run: &str) {
    use crate::provider::{
        AgentEndpointMode, LaunchProviderRequest, ProviderLaunchResult, RuntimeProviderRun,
    };
    let request = LaunchProviderRequest::new(&f.session, "codex", "codex", "default", "fixture")
        .with_agent_id(&f.agent);
    let mut run = RuntimeProviderRun::new(
        crate::provider::projected_leased_provider_run_id("leased-agent", worker_run),
        &request,
        ProviderLaunchResult {
            endpoint_mode: AgentEndpointMode::External,
            process_label: "projection fixture".into(),
            pty_target: None,
            pty_program: None,
            pty_args: vec![],
            pty_env: Default::default(),
            pty_env_remove: vec![],
            working_directory: None,
            structured_endpoint: None,
        },
    );
    run.mark_running();
    // Fixtures represent distinct launches even when constructed in one millisecond.
    if let Some(previous) = f
        .runtime
        .owned
        .provider_store
        .get_latest_run_for_agent(&f.session, &f.agent)
    {
        let mut value = serde_json::to_value(&run).unwrap();
        let later = previous.last_activity_at_ms().max(previous.started_at_ms()) + 1;
        value["started_at_ms"] = serde_json::json!(later);
        value["last_activity_at_ms"] = serde_json::json!(later);
        run = serde_json::from_value(value).unwrap();
    }
    f.runtime
        .owned
        .provider_store
        .write()
        .insert_run_for_test(run);
}

#[tokio::test]
async fn mp08_mp10_mp11_idle_home_review_expires_on_projected_run_replacement() {
    let f = home(None, false).await;
    project_worker_run(&f, &f.run);
    let mut receiver = register(&f, false).await;
    assert_pending(&f, &mut receiver);
    project_worker_run(&f, "replacement");
    f.runtime.owned.withdraw_stale_agent_interactions();
    f.assert_withdrawn(&mut receiver).await;
}

// MP-08 / MP-10 / MP-11: Idle kernel choices require a live projection,
// while provider-native startup permissions retain their pre-ACK allowance.
async fn register_idle_review(
    f: &Fixture,
) -> tokio::sync::oneshot::Receiver<PendingInteractionResolution> {
    let interaction = crate::session::RuntimeInteraction::new(
        f.id(),
        &f.agent,
        crate::session::RuntimeInteractionKind::Choice,
        crate::session::RuntimeInteractionLevel::Info,
        None,
        "Review setup",
        vec![crate::session::RuntimeInteractionChoice::new(
            "allow", "Allow", "allow", None,
        )],
        None,
        Some(900),
        None,
    )
    .with_native_origin(Some(NativeInteractionOrigin::ProviderStartup {
        provider_run_id: f.run.clone(),
    }));
    f.runtime
        .create_runtime_interaction_with_forwarding(
            &f.session,
            interaction,
            Some(&context(f, false)),
        )
        .await
        .unwrap()
}

#[tokio::test]
async fn mp08_mp10_mp11_idle_home_review_requires_live_projection() {
    let f = home(None, false).await;
    let mut receiver = register_idle_review(&f).await;
    f.assert_withdrawn(&mut receiver).await;
}

#[tokio::test]
async fn mp08_mp10_mp11_idle_home_review_survives_dispatch_binding_clear() {
    let f = home(Some("worker-run"), false).await;
    project_worker_run(&f, &f.run);
    let mut receiver = register_idle_review(&f).await;
    assert_pending(&f, &mut receiver);
    bind(&f, None);
    assert_pending(&f, &mut receiver);
    f.runtime
        .resolve_runtime_interaction(&f.session, &f.id(), "allow", None)
        .await
        .unwrap();
    assert_eq!(receiver.await.unwrap().status, "answered");
}

#[tokio::test]
async fn mp08_mp10_mp11_idle_home_review_expires_on_projected_exit() {
    let f = home(None, false).await;
    project_worker_run(&f, &f.run);
    let mut receiver = register_idle_review(&f).await;
    assert_pending(&f, &mut receiver);
    let mut run = f
        .runtime
        .owned
        .provider_store
        .get_latest_run_for_agent(&f.session, &f.agent)
        .unwrap();
    run.mark_ended();
    f.runtime
        .owned
        .provider_store
        .write()
        .insert_run_for_test(run);
    f.runtime.owned.withdraw_stale_agent_interactions();
    f.assert_withdrawn(&mut receiver).await;
}

#[tokio::test]
async fn mp08_mp10_mp11_active_worker_choice_retains_dispatch_pre_ack_allowance() {
    let f = home(Some("previous-worker"), true).await;
    let interaction = crate::session::RuntimeInteraction::new(
        f.id(),
        &f.agent,
        crate::session::RuntimeInteractionKind::Choice,
        crate::session::RuntimeInteractionLevel::Info,
        None,
        "Review setup",
        vec![crate::session::RuntimeInteractionChoice::new(
            "allow", "Allow", "allow", None,
        )],
        None,
        Some(900),
        None,
    )
    .with_native_origin(Some(NativeInteractionOrigin::Prompt {
        provider_run_id: f.run.clone(),
        prompt_id: "home-A".into(),
    }));
    let mut receiver = f
        .runtime
        .create_runtime_interaction_with_forwarding(
            &f.session,
            interaction,
            Some(&context(&f, true)),
        )
        .await
        .unwrap();
    assert_pending(&f, &mut receiver);
    f.runtime
        .resolve_runtime_interaction(&f.session, &f.id(), "allow", None)
        .await
        .unwrap();
    assert_eq!(receiver.await.unwrap().status, "answered");
}

#[tokio::test]
async fn mp08_mp10_mp11_idle_home_review_expires_when_projection_replaces_bound_run() {
    let f = home(Some("worker-run"), false).await;
    project_worker_run(&f, &f.run);
    let mut receiver = register_idle_review(&f).await;
    assert_pending(&f, &mut receiver);
    project_worker_run(&f, "replacement");
    f.runtime.owned.withdraw_stale_agent_interactions();
    f.assert_withdrawn(&mut receiver).await;
}

#[tokio::test]
async fn mp08_mp10_mp11_idle_home_review_prefers_live_projection_over_ended_history() {
    let f = home(None, false).await;
    project_worker_run(&f, "old-run");
    let mut old = f
        .runtime
        .owned
        .provider_store
        .get_latest_run_for_agent(&f.session, &f.agent)
        .unwrap();
    old.mark_ended();
    f.runtime
        .owned
        .provider_store
        .write()
        .insert_run_for_test(old.clone());
    project_worker_run(&f, &f.run);
    let current = f
        .runtime
        .owned
        .provider_store
        .get_run_for_agent(&f.session, &f.agent)
        .unwrap();
    let mut value = serde_json::to_value(&old).unwrap();
    value["last_activity_at_ms"] = serde_json::json!(current.last_activity_at_ms() + 1);
    f.runtime
        .owned
        .provider_store
        .write()
        .insert_run_for_test(serde_json::from_value(value).unwrap());
    let mut receiver = register_idle_review(&f).await;
    assert_pending(&f, &mut receiver);
    f.runtime
        .resolve_runtime_interaction(&f.session, &f.id(), "allow", None)
        .await
        .unwrap();
    assert_eq!(receiver.await.unwrap().status, "answered");
}
