use super::*;
use crate::session::{RuntimeInteractionChoice, RuntimeInteractionLevel};

fn interaction(timeout_sec: Option<u64>) -> RuntimeInteraction {
    RuntimeInteraction::new(
        "interaction-1",
        "agent-1",
        RuntimeInteractionKind::Permission,
        RuntimeInteractionLevel::Warning,
        None,
        "Approve?",
        Vec::<RuntimeInteractionChoice>::new(),
        None,
        timeout_sec,
        None,
    )
}

#[test]
fn remote_response_timeout_outlives_the_user_interaction_deadline() {
    assert_eq!(
        remote_native_interaction_response_timeout(&interaction(Some(300)), 60_000),
        Duration::from_secs(315)
    );
    assert_eq!(
        remote_native_interaction_response_timeout(&interaction(Some(5)), 60_000),
        Duration::from_secs(20)
    );
}

#[test]
fn remote_response_timeout_uses_relay_default_without_interaction_deadline() {
    assert_eq!(
        remote_native_interaction_response_timeout(&interaction(None), 42_000),
        Duration::from_secs(42)
    );
}

#[tokio::test]
async fn native_interaction_bridge_releases_runtime_after_last_router_drops() {
    let config = crate::config::DaemonConfig::for_tests();
    let app = Arc::new(tokio::sync::Mutex::new(
        crate::app::DaemonApp::bootstrap(config.clone()).unwrap(),
    ));
    let weak_app = Arc::downgrade(&app);
    let router = crate::runtime::router::CommandRouter::with_interactive_capacity_from_app(app, 1);
    let clone = router.clone();
    assert!(crate::app::DaemonApp::bootstrap(config.clone()).is_err());
    drop(router);
    assert!(weak_app.upgrade().is_some());
    assert!(crate::app::DaemonApp::bootstrap(config.clone()).is_err());
    drop(clone);
    assert!(
        weak_app.upgrade().is_none(),
        "permission bridge must not retain its own runtime"
    );
    assert!(crate::app::DaemonApp::bootstrap(config).is_ok());
}

async fn permission_queue_fixture() -> (
    crate::runtime::router::CommandRouter,
    String,
    String,
    std::path::PathBuf,
) {
    let root = crate::test_support::test_workspace_root().join(format!(
        "native-permission-queue-{:016x}",
        rand::random::<u64>()
    ));
    std::fs::create_dir_all(&root).unwrap();
    let mut app =
        crate::app::DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests()).unwrap();
    let worktree = root.to_string_lossy().into_owned();
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(crate::session::CreateSessionRequest::new(
            &worktree, &worktree,
        ))
        .unwrap();
    let router = crate::runtime::router::CommandRouter::with_interactive_capacity_from_app(
        Arc::new(tokio::sync::Mutex::new(app)),
        1,
    );
    (router, session.id().into(), agent.id().into(), root)
}

fn queued_permission(id: &str, agent: &str, timeout: u64) -> RuntimeInteraction {
    RuntimeInteraction::new(
        id,
        agent,
        RuntimeInteractionKind::Permission,
        RuntimeInteractionLevel::Warning,
        Some("Approve documentation fetch?".into()),
        "Fetch the requested public documentation",
        vec![
            RuntimeInteractionChoice::new("allow", "Allow once", "allow", None),
            RuntimeInteractionChoice::new("deny", "Deny", "deny", None),
        ],
        None,
        Some(timeout),
        Some("deny".into()),
    )
}

#[tokio::test]
async fn native_permission_queue_waits_for_the_existing_subject_decision() {
    let (router, session, agent, root) = permission_queue_fixture().await;
    let state = router.runtime_state();
    let first = state
        .create_runtime_interaction(&session, queued_permission("first", &agent, 10))
        .await
        .unwrap();
    let pending_state = state.clone();
    let pending_session = session.clone();
    let second = tokio::spawn(async move {
        request_runtime_interaction_with_timeout(
            &pending_state,
            &pending_session,
            queued_permission("second", &agent, 5),
            "native_permission_queue_test",
            None,
        )
        .await
    });
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(
        !second.is_finished(),
        "MP-10/MP-11 parallel permission must wait for the current decision"
    );
    state
        .resolve_runtime_interaction(&session, "first", "allow", None)
        .await
        .unwrap();
    assert_eq!(first.await.unwrap().choice_id.as_deref(), Some("allow"));
    let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
    loop {
        if state
            .resolve_runtime_interaction(&session, "second", "deny", None)
            .await
            .is_ok()
        {
            break;
        }
        assert!(tokio::time::Instant::now() < deadline);
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let resolution = second.await.unwrap().unwrap();
    assert_eq!(resolution.choice_id.as_deref(), Some("deny"));
    std::fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn native_permission_queue_counts_waiting_time_against_its_deadline() {
    let (router, session, agent, root) = permission_queue_fixture().await;
    let state = router.runtime_state();
    let _first = state
        .create_runtime_interaction(&session, queued_permission("first", &agent, 10))
        .await
        .unwrap();
    let started = tokio::time::Instant::now();
    let resolution = request_runtime_interaction_with_timeout(
        &state,
        &session,
        queued_permission("second", &agent, 1),
        "native_permission_queue_test",
        None,
    )
    .await
    .unwrap();
    assert_eq!(resolution.status, "timed_out");
    assert!(started.elapsed() < Duration::from_millis(1500));
    state
        .resolve_runtime_interaction(&session, "first", "deny", None)
        .await
        .unwrap();
    std::fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn native_permission_queue_cancellation_keeps_the_existing_decision() {
    let (router, session, agent, root) = permission_queue_fixture().await;
    let state = router.runtime_state();
    let first = state
        .create_runtime_interaction(&session, queued_permission("shared", &agent, 10))
        .await
        .unwrap();
    let waiting_state = state.clone();
    let waiting_session = session.clone();
    let waiting = tokio::spawn(async move {
        request_runtime_interaction_with_timeout(
            &waiting_state,
            &waiting_session,
            queued_permission("shared", &agent, 5),
            "native_permission_queue_test",
            None,
        )
        .await
    });
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(!waiting.is_finished());
    waiting.abort();
    assert!(waiting.await.unwrap_err().is_cancelled());
    tokio::task::yield_now().await;
    state
        .resolve_runtime_interaction(&session, "shared", "allow", None)
        .await
        .unwrap();
    assert_eq!(first.await.unwrap().choice_id.as_deref(), Some("allow"));
    std::fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn native_permission_queue_does_not_retry_foreign_session_admission() {
    let (router, session, agent, root) = permission_queue_fixture().await;
    let state = router.runtime_state();
    let started = tokio::time::Instant::now();
    let result = request_runtime_interaction_with_timeout(
        &state,
        &format!("{session}-foreign"),
        queued_permission("foreign", &agent, 30),
        "native_permission_queue_test",
        None,
    )
    .await;
    assert!(result.is_err());
    assert!(started.elapsed() < Duration::from_millis(500));
    std::fs::remove_dir_all(root).unwrap();
}

async fn forwarded_permission_fixture() -> (
    crate::runtime::router::CommandRouter,
    RemoteNativeInteractionContext,
    RuntimeInteraction,
    std::path::PathBuf,
) {
    let (router, session, agent, root) = permission_queue_fixture().await;
    let context = RemoteNativeInteractionContext {
        home_session_id: session.clone(),
        home_agent_id: agent.clone(),
        leased_agent_id: "leased-agent".into(),
        worker_provider_run_id: "worker-run".into(),
        home_prompt_id: None,
    };
    router
        .runtime_state()
        .with_app_side_effect(move |app| {
            app.agents
                .bind_remote_execution(
                    &agent,
                    crate::agent::RemoteAgentBinding {
                        worker_kernel_id: "worker".into(),
                        worker_machine_id: "worker-machine".into(),
                        execution_lease_id: "execution-lease".into(),
                        leased_agent_id: "leased-agent".into(),
                        active_worker_provider_run_id: None,
                        relay_url: None,
                        relay_token: None,
                        relay_peer_protocol_version: Some(
                            crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION,
                        ),
                    },
                )
                .unwrap();
        })
        .await;
    let interaction = queued_permission(
        &format!("reconnecting-{}", context.home_session_id),
        "worker-agent",
        2,
    )
    .with_native_origin(Some(
        crate::session::NativeInteractionOrigin::ProviderStartup {
            provider_run_id: "worker-run".into(),
        },
    ));
    (router, context, interaction, root)
}

async fn wait_for_forwarded_card(state: &KernelRuntimeState, session: &str) -> RuntimeInteraction {
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            let session = session.to_owned();
            let card = state
                .with_app_side_effect(move |app| {
                    app.sessions()
                        .get_session(&session)
                        .unwrap()
                        .active_interactions()
                        .first()
                        .cloned()
                })
                .await;
            if let Some(card) = card {
                return card;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("MP-08 forwarded permission should be visible")
}

// MP-08 / MP-10 / MP-11: the first home response can be lost after the
// person answers. Reconnect must replay that answer, never register again.
#[tokio::test]
async fn native_permission_reconnect_replays_the_real_answer_without_a_second_card() {
    for decision in ["allow", "deny"] {
        let (router, context, interaction, root) = forwarded_permission_fixture().await;
        let state = router.runtime_state();
        let pending_state = state.clone();
        let pending_context = context.clone();
        let pending_interaction = interaction.clone();
        let first = tokio::spawn(async move {
            forward_relay_native_interaction(&pending_state, pending_context, pending_interaction)
                .await
        });
        let before = wait_for_forwarded_card(&state, &context.home_session_id).await;
        state
            .resolve_runtime_interaction(&context.home_session_id, interaction.id(), decision, None)
            .await
            .unwrap();
        let lost_response = first.await.unwrap().unwrap();
        assert_eq!(lost_response.choice_id.as_deref(), Some(decision));
        let recovered = tokio::time::timeout(
            Duration::from_millis(300),
            forward_relay_native_interaction(&state, context.clone(), interaction.clone()),
        )
        .await
        .expect("MP-08 reconnect must replay the human decision without asking again")
        .unwrap();
        assert_eq!(recovered, lost_response);
        let snapshot = state
            .with_app_side_effect(move |app| {
                app.sessions()
                    .get_session(&context.home_session_id)
                    .unwrap()
            })
            .await;
        assert!(snapshot.active_interactions().is_empty());
        assert_eq!(before.id(), interaction.id());
        std::fs::remove_dir_all(root).unwrap();
    }
}

#[tokio::test]
async fn native_permission_reconnect_keeps_the_original_card_and_timeout() {
    let (router, context, interaction, root) = forwarded_permission_fixture().await;
    let state = router.runtime_state();
    let pending_state = state.clone();
    let pending_context = context.clone();
    let pending_interaction = interaction.clone();
    let started = tokio::time::Instant::now();
    let first = tokio::spawn(async move {
        forward_relay_native_interaction(&pending_state, pending_context, pending_interaction).await
    });
    let before = wait_for_forwarded_card(&state, &context.home_session_id).await;
    tokio::time::sleep(Duration::from_millis(1200)).await;
    let retry_state = state.clone();
    let retry_context = context.clone();
    let second = tokio::spawn(async move {
        forward_relay_native_interaction(&retry_state, retry_context, interaction).await
    });
    assert_eq!(
        wait_for_forwarded_card(&state, &context.home_session_id).await,
        before
    );
    let original_timeout = first.await.unwrap().unwrap();
    assert_eq!(original_timeout.status, "answered");
    assert_eq!(original_timeout.choice_id.as_deref(), Some("deny"));
    let recovered = tokio::time::timeout(Duration::from_millis(400), second)
        .await
        .expect("MP-08 reconnect must use the original timeout, never another permission budget")
        .unwrap()
        .unwrap();
    assert_eq!(recovered, original_timeout);
    assert!(started.elapsed() < Duration::from_millis(2600));
    std::fs::remove_dir_all(root).unwrap();
}

// MP-08 / MP-10 / MP-11: a provider login reuses one interaction id for every
// phase. Once the worker retires a decided phase, the next phase is a new
// decision, never a mismatched or replayed transport retry.
#[tokio::test]
async fn provider_login_phases_reuse_one_id_without_replaying_the_previous_answer() {
    for unchanged in [false, true] {
        let (router, context, _, root) = forwarded_permission_fixture().await;
        let state = router.runtime_state();
        let id = "provider-auth-recovery:login-1";
        let phase = |title: &str| {
            let origin = crate::session::NativeInteractionOrigin::ProviderStartup {
                provider_run_id: "worker-run".into(),
            };
            RuntimeInteraction::new(
                id,
                "worker-agent",
                RuntimeInteractionKind::Permission,
                RuntimeInteractionLevel::Warning,
                Some(title.into()),
                "Complete the provider login",
                vec![
                    RuntimeInteractionChoice::new("allow", "Continue", "allow", None),
                    RuntimeInteractionChoice::new("deny", "Cancel", "deny", None),
                ],
                None,
                Some(30),
                None,
            )
            .with_native_origin(Some(origin))
        };
        let first = phase("Unlock Chariox Vault");
        let second = if unchanged {
            first.clone()
        } else {
            phase("Authenticate provider account")
        };
        for (interaction, decision) in [(first, "allow"), (second, "deny")] {
            let (call_state, call_context, call) =
                (state.clone(), context.clone(), interaction.clone());
            let pending = tokio::spawn(async move {
                forward_relay_native_interaction(&call_state, call_context, call).await
            });
            let card = wait_for_forwarded_card(&state, &context.home_session_id).await;
            assert_eq!(card.title(), interaction.title());
            state
                .resolve_runtime_interaction(&context.home_session_id, id, decision, None)
                .await
                .unwrap();
            let resolved = pending.await.unwrap().unwrap();
            assert_eq!(resolved.choice_id.as_deref(), Some(decision));
            // The worker retires the decided phase before it asks the next one.
            state
                .update_forwarded_provider_login_interaction(
                    &context.home_session_id,
                    &context.home_agent_id,
                    id,
                    None,
                )
                .unwrap();
        }
        std::fs::remove_dir_all(root).unwrap();
    }
}

#[tokio::test]
async fn native_permission_reconnect_keeps_card_when_reply_waiter_is_cancelled() {
    let (router, context, interaction, root) = forwarded_permission_fixture().await;
    let state = router.runtime_state();
    let pending_state = state.clone();
    let pending_context = context.clone();
    let pending_interaction = interaction.clone();
    let first = tokio::spawn(async move {
        forward_relay_native_interaction(&pending_state, pending_context, pending_interaction).await
    });
    let before = wait_for_forwarded_card(&state, &context.home_session_id).await;
    first.abort();
    assert!(first.await.unwrap_err().is_cancelled());
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(
        wait_for_forwarded_card(&state, &context.home_session_id).await,
        before
    );
    state
        .resolve_runtime_interaction(&context.home_session_id, interaction.id(), "allow", None)
        .await
        .unwrap();
    let recovered = forward_relay_native_interaction(&state, context, interaction)
        .await
        .unwrap();
    assert_eq!(recovered.choice_id.as_deref(), Some("allow"));
    std::fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn native_permission_reconnect_rejects_a_different_run_using_the_same_id() {
    let (router, context, interaction, root) = forwarded_permission_fixture().await;
    let state = router.runtime_state();
    let pending_state = state.clone();
    let pending_context = context.clone();
    let pending_interaction = interaction.clone();
    let first = tokio::spawn(async move {
        forward_relay_native_interaction(&pending_state, pending_context, pending_interaction).await
    });
    let before = wait_for_forwarded_card(&state, &context.home_session_id).await;
    let mut replacement = context.clone();
    replacement.worker_provider_run_id = "replacement-run".into();
    let changed = interaction.clone().with_native_origin(Some(
        crate::session::NativeInteractionOrigin::ProviderStartup {
            provider_run_id: "replacement-run".into(),
        },
    ));
    assert!(
        forward_relay_native_interaction(&state, replacement, changed)
            .await
            .is_err()
    );
    assert_eq!(
        wait_for_forwarded_card(&state, &context.home_session_id).await,
        before
    );
    state
        .resolve_runtime_interaction(&context.home_session_id, interaction.id(), "deny", None)
        .await
        .unwrap();
    assert_eq!(
        first.await.unwrap().unwrap().choice_id.as_deref(),
        Some("deny")
    );
    std::fs::remove_dir_all(root).unwrap();
}
