//! Isolated kernel drills with a dev-stub provider; no provider credentials.
use super::*;
use crate::session::{
    RuntimeInteraction, RuntimeInteractionChoice, RuntimeInteractionKind, RuntimeInteractionLevel,
};

struct Fixture {
    runtime: KernelRuntimeState,
    session: String,
    agent: String,
    run: String,
    attachment: String,
    _worktree: crate::test_support::TestWorktree,
}
impl Fixture {
    async fn new() -> Self {
        let worktree = crate::test_support::TestWorktree::new("approval-lifetime");
        let mut app = DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests()).unwrap();
        let (session, agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(worktree.session_request())
            .unwrap();
        let attachment = crate::app::KernelSessionService::new(&mut app)
            .attach(crate::attachment::AttachRequest::new(
                session.id(),
                "approval-test",
                crate::attachment::ClientCapabilityLevel::FullTerminal,
            ))
            .unwrap();
        let run = app
            .launch_provider(
                crate::provider::LaunchProviderRequest::new(
                    session.id(),
                    "dev-stub",
                    "claude-code",
                    "default",
                    "sonnet",
                )
                .with_agent_id(agent.id()),
            )
            .unwrap();
        app.submit_prompt(
            session.id(),
            attachment.id(),
            Some(agent.id()),
            "request approval",
            Vec::new(),
        )
        .unwrap();
        let runtime = owned_runtime_state(&Arc::new(Mutex::new(app))).await;
        Self {
            runtime,
            session: session.id().into(),
            agent: agent.id().into(),
            run: run.id().into(),
            attachment: attachment.id().into(),
            _worktree: worktree,
        }
    }
    fn id(&self) -> String {
        format!("approval-{}", self.session)
    }
    async fn approval(&self) -> tokio::sync::oneshot::Receiver<PendingInteractionResolution> {
        self.runtime
            .create_runtime_interaction(
                &self.session,
                RuntimeInteraction::new(
                    self.id(),
                    &self.agent,
                    RuntimeInteractionKind::Permission,
                    RuntimeInteractionLevel::Warning,
                    Some("App binding approval".into()),
                    "Allow binding?",
                    vec![RuntimeInteractionChoice::new(
                        "allow", "Allow", "allow", None,
                    )],
                    None,
                    None,
                    Some("allow".into()),
                ),
            )
            .await
            .unwrap()
    }
    async fn assert_withdrawn(
        &self,
        receiver: &mut tokio::sync::oneshot::Receiver<PendingInteractionResolution>,
    ) {
        assert!(self
            .runtime
            .owned
            .session_snapshot(&self.session)
            .unwrap()
            .active_interactions()
            .is_empty());
        let result = receiver.try_recv().unwrap();
        assert_eq!(result.status, "timed_out");
        assert!(result.reply.is_none());
        assert!(
            result.choice_id.is_none(),
            "withdrawal must never apply a timeout default"
        );
        assert!(self
            .runtime
            .resolve_runtime_interaction(&self.session, &self.id(), "allow", None)
            .await
            .is_err());
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = self
            .runtime
            .owned
            .withdraw_agent_interactions(&self.session, None);
        let _ = self
            .runtime
            .owned
            .provider_store
            .terminate_run_provider_only(&self.session, &self.run);
    }
}

#[tokio::test]
async fn approval_lifetime_cancel_withdraws_and_refuses_late_answer() {
    let f = Fixture::new().await;
    let mut receiver = f.approval().await;
    f.runtime
        .owned
        .cancel_active_prompt_only(&f.session, &f.agent)
        .unwrap();
    f.assert_withdrawn(&mut receiver).await;
    let entries = f
        .runtime
        .owned
        .operational_history_store
        .load_session_history_entries(&f.session, Some(&f.agent))
        .unwrap();
    assert!(entries.iter().any(|entry| entry.text.contains("withdrawn")));
}

#[tokio::test]
async fn approval_lifetime_cancellation_request_withdraws_before_abort_ack() {
    let f = Fixture::new().await;
    let mut receiver = f.approval().await;
    f.runtime
        .owned
        .cancel_local_prompt(&f.session, &f.agent, &f.attachment)
        .unwrap();
    f.assert_withdrawn(&mut receiver).await;
}

#[tokio::test]
async fn approval_lifetime_completion_and_followup_do_not_retain_approval() {
    let f = Fixture::new().await;
    let mut receiver = f.approval().await;
    f.runtime
        .owned
        .complete_local_prompt_without_advance(&f.session, &f.agent, Some(&f.run))
        .unwrap();
    f.assert_withdrawn(&mut receiver).await;
    f.runtime
        .app
        .lock()
        .await
        .submit_prompt(
            &f.session,
            &f.attachment,
            Some(&f.agent),
            "new turn",
            Vec::new(),
        )
        .unwrap();
    assert!(f
        .runtime
        .resolve_runtime_interaction(&f.session, &f.id(), "allow", None)
        .await
        .is_err());
    // A fresh approval belongs to the new turn and remains answerable.
    let mut next = f.approval().await;
    f.runtime
        .resolve_runtime_interaction(&f.session, &f.id(), "allow", None)
        .await
        .unwrap();
    assert_eq!(next.try_recv().unwrap().status, "answered");
}

#[tokio::test]
async fn approval_lifetime_agent_delete_withdraws_even_without_a_turn() {
    let f = Fixture::new().await;
    f.runtime
        .owned
        .complete_local_prompt_without_advance(&f.session, &f.agent, Some(&f.run))
        .unwrap();
    let mut receiver = f.approval().await;
    f.runtime
        .owned
        .destroy_agent(&f.agent, crate::session::DEFAULT_LOCAL_USER_ID)
        .unwrap();
    f.assert_withdrawn(&mut receiver).await;
}

#[tokio::test]
async fn approval_lifetime_session_delete_closes_responder_and_refuses_answer() {
    let f = Fixture::new().await;
    let mut receiver = f.approval().await;
    f.runtime
        .owned
        .delete_session_ref(&f.session, None)
        .unwrap();
    let result = receiver.try_recv().unwrap();
    assert_eq!(result.status, "timed_out");
    assert!(result.choice_id.is_none());
    assert!(f
        .runtime
        .resolve_runtime_interaction(&f.session, &f.id(), "allow", None)
        .await
        .is_err());
}

#[tokio::test]
async fn approval_lifetime_provider_exit_withdraws() {
    let f = Fixture::new().await;
    let mut receiver = f.approval().await;
    f.runtime
        .owned
        .provider_store
        .terminate_run_provider_only(&f.session, &f.run)
        .unwrap();
    f.runtime
        .reconcile_provider_run_exit(&f.session, &f.run)
        .await
        .unwrap();
    f.assert_withdrawn(&mut receiver).await;
}

#[tokio::test]
async fn approval_lifetime_late_answer_is_refused_before_projection_catches_up() {
    let f = Fixture::new().await;
    let mut receiver = f.approval().await;
    let session = f
        .runtime
        .owned
        .session_store
        .get_session(&f.session)
        .unwrap();
    // Reproduce the interval between the authoritative mutation and its mirror.
    f.runtime
        .owned
        .prompt_state_owner
        .cancel_active_prompt_only(&session, &f.agent)
        .unwrap();
    assert!(f
        .runtime
        .resolve_runtime_interaction(&f.session, &f.id(), "allow", None)
        .await
        .is_err());
    f.assert_withdrawn(&mut receiver).await;
}

#[tokio::test]
async fn approval_lifetime_long_running_turn_survives_65_seconds() {
    let f = Fixture::new().await;
    let origin = f
        .runtime
        .capture_native_interaction_origin(&f.session, &f.agent, &f.run);
    let mut receiver = f
        .runtime
        .create_runtime_interaction(&f.session, native_approval(&f, origin))
        .await
        .unwrap();
    tokio::time::sleep(std::time::Duration::from_secs(66)).await;
    f.runtime.pump_transport_runtime().await;
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
    f.runtime
        .resolve_runtime_interaction(&f.session, &f.id(), "allow", None)
        .await
        .unwrap();
    assert_eq!(
        receiver.try_recv().unwrap().choice_id.as_deref(),
        Some("allow")
    );
}

#[tokio::test]
async fn approval_lifetime_timeout_after_turn_end_cannot_apply_allow_default() {
    let f = Fixture::new().await;
    let mut receiver = f.approval().await;
    let session = f
        .runtime
        .owned
        .session_store
        .get_session(&f.session)
        .unwrap();
    f.runtime
        .owned
        .prompt_state_owner
        .cancel_active_prompt_only(&session, &f.agent)
        .unwrap();
    f.runtime
        .timeout_runtime_interaction(&f.session, &f.id())
        .await
        .unwrap();
    f.assert_withdrawn(&mut receiver).await;
}

#[tokio::test]
async fn approval_lifetime_native_turn_end_withdraws_without_prompt_owner() {
    let f = Fixture::new().await;
    f.runtime
        .owned
        .complete_local_prompt_without_advance(&f.session, &f.agent, Some(&f.run))
        .unwrap();
    f.runtime
        .owned
        .active_turns
        .start(crate::app::ActiveTurnState::new(
            f.session.clone(),
            f.agent.clone(),
            "native-turn".into(),
            f.run.clone(),
        ));
    let mut receiver = f.approval().await;
    f.runtime.owned.active_turns.clear(&f.run);
    f.runtime.owned.withdraw_stale_agent_interactions();
    f.assert_withdrawn(&mut receiver).await;
}

#[tokio::test]
async fn approval_lifetime_withdrawal_wakes_and_clears_both_terminal_projections() {
    let f = Fixture::new().await;
    let mut receiver = f.approval().await;
    let second = {
        let mut app = f.runtime.app.lock().await;
        crate::app::KernelSessionService::new(&mut app)
            .attach(crate::attachment::AttachRequest::new(
                &f.session,
                "approval-test-second",
                crate::attachment::ClientCapabilityLevel::FullTerminal,
            ))
            .unwrap()
    };
    let router =
        crate::runtime::router::CommandRouter::with_interactive_capacity(f.runtime.app.clone(), 1);
    let mut snapshots = Vec::new();
    for attachment in [&f.attachment, &second.id().to_owned()] {
        let crate::runtime_transport::WatchResult::Ok { snapshot, .. } = router
            .relay_watch_subscription_state(&f.session, attachment, true, None, 0)
            .await
        else {
            panic!("terminal should attach");
        };
        let snapshot = snapshot.as_ref().clone().unwrap();
        assert_eq!(snapshot.session.active_interactions().len(), 1);
        snapshots.push((attachment.clone(), snapshot));
    }
    let before = router.session_projection_change_sequence();
    f.runtime
        .owned
        .cancel_active_prompt_only(&f.session, &f.agent)
        .unwrap();
    tokio::time::timeout(
        std::time::Duration::from_secs(1),
        router.wait_for_session_projection_change_after(before),
    )
    .await
    .unwrap();
    for (attachment, previous) in snapshots {
        let crate::runtime_transport::WatchResult::Ok { snapshot, .. } = router
            .relay_watch_subscription_state(&f.session, &attachment, true, Some(previous), 0)
            .await
        else {
            panic!("terminal should receive update");
        };
        assert!(snapshot
            .as_ref()
            .as_ref()
            .unwrap()
            .session
            .active_interactions()
            .is_empty());
    }
    f.assert_withdrawn(&mut receiver).await;
}

#[tokio::test]
async fn approval_lifetime_kernel_shutdown_closes_pending_approval() {
    let f = Fixture::new().await;
    let mut receiver = f.approval().await;
    f.runtime.shutdown_cleanup().await.unwrap();
    f.assert_withdrawn(&mut receiver).await;
}

fn native_approval(
    f: &Fixture,
    origin: Option<crate::session::NativeInteractionOrigin>,
) -> RuntimeInteraction {
    RuntimeInteraction::new(
        f.id(),
        &f.agent,
        RuntimeInteractionKind::Permission,
        RuntimeInteractionLevel::Warning,
        None,
        "Allow?",
        vec![RuntimeInteractionChoice::new(
            "allow", "Allow", "allow", None,
        )],
        None,
        None,
        Some("allow".into()),
    )
    .with_native_origin(origin)
}

async fn delayed_native_approval(followup: bool) {
    let f = Fixture::new().await;
    let bridge =
        crate::runtime::native_interaction_bridge::install_provider_native_interaction_bridge(
            f.runtime.clone(),
            &f.runtime.owned.provider_store,
        )
        .unwrap();
    let origin = bridge
        .capture_turn_origin(&f.session, &f.agent, &f.run)
        .unwrap();
    let interaction = native_approval(&f, Some(origin));
    let (release, wait) = std::sync::mpsc::channel();
    let session = f.session.clone();
    let callback = tokio::task::spawn_blocking(move || {
        wait.recv().unwrap();
        bridge.request_blocking(&session, interaction)
    });
    f.runtime
        .owned
        .cancel_active_prompt_only(&f.session, &f.agent)
        .unwrap();
    if followup {
        f.runtime
            .app
            .lock()
            .await
            .submit_prompt(
                &f.session,
                &f.attachment,
                Some(&f.agent),
                "follow-up while callback A is delayed",
                Vec::new(),
            )
            .unwrap();
    }
    release.send(()).unwrap();
    let resolution = tokio::time::timeout(std::time::Duration::from_secs(2), callback)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(resolution.status, "timed_out");
    assert!(resolution.choice_id.is_none() && resolution.reply.is_none());
    assert!(f
        .runtime
        .owned
        .session_snapshot(&f.session)
        .unwrap()
        .active_interactions()
        .is_empty());
    assert!(f
        .runtime
        .resolve_runtime_interaction(&f.session, &f.id(), "allow", None)
        .await
        .is_err());
    if followup {
        let mut fresh = f.approval().await;
        f.runtime
            .resolve_runtime_interaction(&f.session, &f.id(), "allow", None)
            .await
            .unwrap();
        assert_eq!(fresh.try_recv().unwrap().reply.as_deref(), Some("allow"));
    }
}

#[tokio::test]
async fn approval_lifetime_delayed_real_bridge_after_cancel_is_refused() {
    delayed_native_approval(false).await;
}

#[tokio::test]
async fn approval_lifetime_delayed_real_bridge_cannot_attach_to_followup() {
    delayed_native_approval(true).await;
}

#[tokio::test]
async fn approval_lifetime_delayed_local_and_relay_requests_cannot_attach_to_followup() {
    let f = Fixture::new().await;
    let origin = f
        .runtime
        .capture_native_interaction_origin(&f.session, &f.agent, &f.run)
        .unwrap();
    let home_prompt_id = match &origin {
        crate::session::NativeInteractionOrigin::Prompt { prompt_id, .. } => prompt_id.clone(),
        _ => panic!("expected kernel prompt"),
    };
    let local = crate::local::RequestNativeProviderTurnInteractionRequest::allow_deny(
        &f.session,
        &f.agent,
        f.id(),
        None,
        "Approve?",
        None,
        origin.clone(),
    );
    let interaction = native_approval(&f, Some(origin));
    let context = crate::transport::relay_peer::RemoteNativeInteractionContext {
        home_session_id: f.session.clone(),
        home_agent_id: f.agent.clone(),
        leased_agent_id: "lease-test".into(),
        worker_provider_run_id: f.run.clone(),
        home_prompt_id: Some(home_prompt_id),
    };
    f.runtime
        .owned
        .complete_local_prompt_without_advance(&f.session, &f.agent, Some(&f.run))
        .unwrap();
    f.runtime
        .app
        .lock()
        .await
        .submit_prompt(
            &f.session,
            &f.attachment,
            Some(&f.agent),
            "follow-up",
            Vec::new(),
        )
        .unwrap();
    let response =
        crate::runtime::native_interaction_bridge::execute_native_provider_interaction_request(
            &f.runtime, local,
        )
        .await
        .unwrap();
    match response {
        crate::local::LocalDaemonResponse::NativeProviderInteractionResolved { resolution } => {
            assert_eq!(resolution.status, "timed_out");
            assert!(resolution.choice_id.is_none() && resolution.reply.is_none());
        }
        _ => panic!("unexpected response"),
    }
    let resolution = crate::runtime::native_interaction_bridge::forward_relay_native_interaction(
        &f.runtime,
        context,
        interaction,
    )
    .await
    .unwrap();
    assert_eq!(resolution.status, "timed_out");
    assert!(resolution.choice_id.is_none() && resolution.reply.is_none());
    assert!(f
        .runtime
        .owned
        .session_snapshot(&f.session)
        .unwrap()
        .active_interactions()
        .is_empty());
}

#[tokio::test]
async fn approval_lifetime_unbound_native_request_is_refused() {
    let f = Fixture::new().await;
    let bridge =
        crate::runtime::native_interaction_bridge::install_provider_native_interaction_bridge(
            f.runtime.clone(),
            &f.runtime.owned.provider_store,
        )
        .unwrap();
    let interaction = native_approval(&f, None);
    let session = f.session.clone();
    let result =
        tokio::task::spawn_blocking(move || bridge.request_blocking(&session, interaction))
            .await
            .unwrap()
            .unwrap();
    assert_eq!(result.status, "timed_out");
    assert!(result.reply.is_none());
}

#[tokio::test]
async fn approval_lifetime_explicit_startup_scope_survives_until_provider_exit() {
    let f = Fixture::new().await;
    f.runtime
        .owned
        .complete_local_prompt_without_advance(&f.session, &f.agent, Some(&f.run))
        .unwrap();
    let interaction = native_approval(
        &f,
        Some(crate::session::NativeInteractionOrigin::ProviderStartup {
            provider_run_id: f.run.clone(),
        }),
    );
    let mut receiver = f
        .runtime
        .create_runtime_interaction(&f.session, interaction)
        .await
        .unwrap();
    assert!(matches!(
        receiver.try_recv(),
        Err(tokio::sync::oneshot::error::TryRecvError::Empty)
    ));
    f.runtime
        .owned
        .provider_store
        .terminate_run_provider_only(&f.session, &f.run)
        .unwrap();
    f.runtime
        .reconcile_provider_run_exit(&f.session, &f.run)
        .await
        .unwrap();
    f.assert_withdrawn(&mut receiver).await;
}

#[path = "approval_lifetime_remote.rs"]
mod remote;
