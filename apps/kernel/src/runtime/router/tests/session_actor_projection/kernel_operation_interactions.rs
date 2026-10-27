use super::*;
use crate::runtime::command::{KernelCallerKind, KernelCommandSource};
use crate::session::{RuntimeSession, DEFAULT_LOCAL_USER_ID};

fn setup() -> (CommandRouter, String) {
    setup_with(|_| {})
}
fn setup_with(prepare: impl FnOnce(&mut RuntimeSession)) -> (CommandRouter, String) {
    let app = DaemonApp::bootstrap(DaemonConfig::for_tests()).unwrap();
    let mut session = RuntimeSession::new(
        format!("kernel-decision-{:016x}", rand::random::<u64>()),
        None,
        "workspace",
        "worktree",
        "machine",
        "kernel",
    );
    prepare(&mut session);
    let session_id = session.id().to_owned();
    app.sessions_mut().restore_session(session);
    (
        CommandRouter::with_interactive_capacity(Arc::new(Mutex::new(app)), 1),
        session_id,
    )
}
fn decision(id: &str, operation: &str) -> RuntimeInteraction {
    RuntimeInteraction::for_kernel_operation(
        id,
        operation,
        "Install App?",
        "Review this release",
        vec![
            RuntimeInteractionChoice::new("deny", "Cancel", "deny", None),
            RuntimeInteractionChoice::new("allow", "Install", "allow", None),
        ],
    )
}
fn reply(session: &str, interaction: &str) -> LocalDaemonRequest {
    LocalDaemonRequest::RespondToInteraction(crate::local::RespondToInteractionRequest {
        session_id: session.into(),
        interaction_id: interaction.into(),
        choice_id: "allow".into(),
        custom_reply: None,
        passkey: None,
        passkey_remember_minutes: None,
    })
}

#[tokio::test]
async fn two_kernel_decisions_project_without_agents_and_use_the_existing_terminal_reply() {
    let (router, session) = setup();
    let before = router.terminal_session_change_sequence(&session);
    let (first, second) = tokio::join!(
        router.runtime_state.create_kernel_operation_interaction(
            &session,
            DEFAULT_LOCAL_USER_ID,
            decision("first", "operation-one")
        ),
        router.runtime_state.create_kernel_operation_interaction(
            &session,
            DEFAULT_LOCAL_USER_ID,
            decision("second", "operation-two")
        ),
    );
    let first = first.unwrap();
    let second = second.unwrap();
    let snapshot = router
        .runtime_state
        .session_snapshot(&session)
        .await
        .unwrap();
    assert!(snapshot.agents().is_empty());
    assert!(snapshot.focused_agent_id().is_none());
    assert_eq!(snapshot.active_interactions().len(), 2);
    assert!(router.terminal_session_change_sequence(&session) > before);
    let request = reply(&session, "first");
    let mut command = KernelCommand::from_local_request("answer-first", None, None, &request);
    command.caller.connection_class = Some(crate::local::KernelConnectionClass::Terminal);
    assert!(matches!(
        router.dispatch(command, request).await.unwrap(),
        LocalDaemonResponse::InteractionResponded { .. }
    ));
    assert_eq!(first.await.unwrap().choice_id.as_deref(), Some("allow"));
    assert_eq!(
        router
            .runtime_state
            .session_snapshot(&session)
            .await
            .unwrap()
            .active_interactions()
            .len(),
        1
    );
    router
        .runtime_state
        .timeout_runtime_interaction(&session, "second")
        .await
        .unwrap();
    let timed_out = second.await.unwrap();
    assert_eq!(timed_out.status, "timed_out");
    assert!(timed_out.choice_id.is_none());
}

#[tokio::test]
async fn kernel_decisions_reject_agent_resolution_other_users_and_remote_kernels() {
    let (router, session) = setup();
    let receiver = router
        .runtime_state
        .create_kernel_operation_interaction(
            &session,
            DEFAULT_LOCAL_USER_ID,
            decision("owned", "operation"),
        )
        .await
        .unwrap();
    assert!(router
        .runtime_state
        .resolve_runtime_interaction(&session, "owned", "allow", None)
        .await
        .is_err());
    assert!(router
        .runtime_state
        .resolve_terminal_runtime_interaction(&session, "owned", "allow", None, Some("other-user"))
        .await
        .is_err());
    let request = reply(&session, "owned");
    let mut command =
        KernelCommand::from_local_request("remote-kernel-answer", None, None, &request);
    command.source = KernelCommandSource::RelayPeer;
    command.caller.caller_kind = KernelCallerKind::RemoteKernel;
    command.caller.user_id = Some(DEFAULT_LOCAL_USER_ID.into());
    assert!(router.dispatch(command, request).await.is_err());
    assert_eq!(
        router
            .runtime_state
            .session_snapshot(&session)
            .await
            .unwrap()
            .active_interactions()
            .len(),
        1
    );
    router
        .runtime_state
        .resolve_terminal_runtime_interaction(
            &session,
            "owned",
            "allow",
            None,
            Some(DEFAULT_LOCAL_USER_ID),
        )
        .await
        .unwrap();
    assert_eq!(receiver.await.unwrap().choice_id.as_deref(), Some("allow"));
    assert!(router
        .runtime_state
        .resolve_terminal_runtime_interaction(
            &session,
            "owned",
            "allow",
            None,
            Some(DEFAULT_LOCAL_USER_ID)
        )
        .await
        .is_err());
}

#[tokio::test]
async fn duplicate_subject_and_foreign_timeout_cannot_replace_or_remove_a_decision() {
    let (router, session) = setup();
    let receiver = router
        .runtime_state
        .create_kernel_operation_interaction(
            &session,
            DEFAULT_LOCAL_USER_ID,
            decision("original", "operation"),
        )
        .await
        .unwrap();
    assert!(router
        .runtime_state
        .create_kernel_operation_interaction(
            &session,
            DEFAULT_LOCAL_USER_ID,
            decision("replacement", "operation")
        )
        .await
        .is_err());
    assert!(router
        .runtime_state
        .create_runtime_interaction(&session, decision("agent-bypass", "other"))
        .await
        .is_err());
    router
        .runtime_state
        .timeout_runtime_interaction("foreign-session", "original")
        .await
        .unwrap();
    assert_eq!(
        router
            .runtime_state
            .session_snapshot(&session)
            .await
            .unwrap()
            .active_interactions()
            .len(),
        1
    );
    router
        .runtime_state
        .timeout_runtime_interaction(&session, "original")
        .await
        .unwrap();
    assert_eq!(receiver.await.unwrap().status, "timed_out");
}

#[tokio::test]
async fn a_decision_nobody_waits_for_is_superseded_by_the_same_subject() {
    let (router, session) = setup();
    let abandoned = router
        .runtime_state
        .create_kernel_operation_interaction(
            &session,
            DEFAULT_LOCAL_USER_ID,
            decision("abandoned", "operation"),
        )
        .await
        .unwrap();
    // The component that asked was replaced: its receiver is gone.
    drop(abandoned);
    let replacement = router
        .runtime_state
        .create_kernel_operation_interaction(
            &session,
            DEFAULT_LOCAL_USER_ID,
            decision("replacement", "operation"),
        )
        .await
        .unwrap();
    let active = router
        .runtime_state
        .session_snapshot(&session)
        .await
        .unwrap()
        .active_interactions()
        .iter()
        .map(|interaction| interaction.id().to_owned())
        .collect::<Vec<_>>();
    assert_eq!(active, ["replacement"]);
    // A decision someone still waits for keeps blocking its subject.
    assert!(router
        .runtime_state
        .create_kernel_operation_interaction(
            &session,
            DEFAULT_LOCAL_USER_ID,
            decision("third", "operation"),
        )
        .await
        .is_err());
    drop(replacement);
}

async fn active_ids(router: &CommandRouter, session: &str) -> Vec<String> {
    router
        .runtime_state
        .session_snapshot(session)
        .await
        .unwrap()
        .active_interactions()
        .iter()
        .map(|interaction| interaction.id().to_owned())
        .collect()
}

#[tokio::test]
async fn a_decision_restored_after_a_restart_is_superseded_by_the_same_subject() {
    // Persisted with the session, but no pending entry survived the restart:
    // nobody can resolve it.
    let (router, session) = setup_with(|session| {
        session.add_active_interaction(decision("restored", "operation"));
    });
    let replacement = router
        .runtime_state
        .create_kernel_operation_interaction(
            &session,
            DEFAULT_LOCAL_USER_ID,
            decision("replacement", "operation"),
        )
        .await
        .unwrap();
    assert_eq!(active_ids(&router, &session).await, ["replacement"]);
    drop(replacement);
}

#[tokio::test]
async fn another_owners_abandoned_decision_still_blocks_the_subject() {
    let (router, session) = setup_with(|session| {
        session.add_member("bob", None, crate::session::CollaborationLevel::Full);
    });
    let abandoned = router
        .runtime_state
        .create_kernel_operation_interaction(&session, "bob", decision("bob", "operation"))
        .await
        .unwrap();
    drop(abandoned);
    assert!(router
        .runtime_state
        .create_kernel_operation_interaction(
            &session,
            DEFAULT_LOCAL_USER_ID,
            decision("alice", "operation"),
        )
        .await
        .is_err());
    assert_eq!(active_ids(&router, &session).await, ["bob"]);
}

#[tokio::test]
async fn an_owner_at_the_limit_can_replace_an_abandoned_decision() {
    crate::test_support::isolated_env_test!();
    let (router, session) = setup();
    let mut waiting = Vec::new();
    for index in 0..7 {
        waiting.push(
            router
                .runtime_state
                .create_kernel_operation_interaction(
                    &session,
                    DEFAULT_LOCAL_USER_ID,
                    decision(&format!("live-{index}"), &format!("operation-{index}")),
                )
                .await
                .unwrap(),
        );
    }
    let abandoned = router
        .runtime_state
        .create_kernel_operation_interaction(
            &session,
            DEFAULT_LOCAL_USER_ID,
            decision("abandoned", "operation"),
        )
        .await
        .unwrap();
    drop(abandoned);
    let replacement = router
        .runtime_state
        .create_kernel_operation_interaction(
            &session,
            DEFAULT_LOCAL_USER_ID,
            decision("replacement", "operation"),
        )
        .await
        .unwrap();
    assert!(active_ids(&router, &session)
        .await
        .contains(&"replacement".to_owned()));
    drop((replacement, waiting));
}

#[tokio::test]
async fn kernel_registration_rejects_custom_or_automatic_approval() {
    let (router, session) = setup();
    for field in ["default_on_timeout", "custom_choice"] {
        let mut wire = serde_json::to_value(decision(field, field)).unwrap();
        wire[field] = if field == "default_on_timeout" {
            serde_json::json!("allow")
        } else {
            serde_json::json!({"id":"custom","label":"Custom"})
        };
        let interaction = serde_json::from_value(wire).unwrap();
        assert!(router
            .runtime_state
            .create_kernel_operation_interaction(&session, DEFAULT_LOCAL_USER_ID, interaction)
            .await
            .is_err());
    }
    assert!(router
        .runtime_state
        .session_snapshot(&session)
        .await
        .unwrap()
        .active_interactions()
        .is_empty());
}
