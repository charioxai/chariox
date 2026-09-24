use super::*;
use crate::runtime::command::{KernelCallerKind, KernelCommandSource};
use crate::session::{RuntimeSession, DEFAULT_LOCAL_USER_ID};

fn setup() -> (CommandRouter, String) {
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).unwrap();
    let session = RuntimeSession::new(
        format!("kernel-decision-{:016x}", rand::random::<u64>()),
        None,
        "workspace",
        "worktree",
        "machine",
        "kernel",
    );
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
    let command = KernelCommand::from_local_request("answer-first", None, None, &request);
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
