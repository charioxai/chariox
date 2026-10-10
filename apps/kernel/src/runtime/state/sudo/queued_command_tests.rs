//! MP-08 / MP-10 / MP-11 A04: a queued sudo command keeps its submitting turn.
use super::tests::{fixture_with_options, popup};
use super::window_tests::{approve, open, start_continuation};
use super::*;
use crate::runtime::command::{KernelCaller, KernelCommand, KernelCommandSource};
use crate::runtime::session_actor::{FocusedAgentProjection, SessionRuntime};

fn spawn(session_id: &str, alias: &str, kernel_ref: Option<&str>) -> LocalDaemonRequest {
    LocalDaemonRequest::SpawnAgent(SpawnAgentRequest {
        session_id: session_id.into(),
        alias: Some(alias.into()),
        provider: Some("dev-stub".into()),
        account_profile: None,
        model: None,
        effort: None,
        execution_mode: None,
        permission_level: None,
        worktree_id: None,
        kernel_ref: kernel_ref.map(Into::into),
        slice_ref: None,
        worktree_placement: None,
        metaagent: false,
    })
}

#[tokio::test]
async fn queued_sudo_session_command_cannot_run_as_a_later_continuation() {
    assert_queued_command(true, true).await;
}

#[tokio::test]
async fn queued_sudo_session_command_can_run_in_its_submitting_turn() {
    assert_queued_command(false, true).await;
}

#[tokio::test]
async fn queued_sudo_session_command_without_run_cannot_borrow_the_current_turn() {
    assert_queued_command(false, false).await;
}

async fn assert_queued_command(end_turn: bool, include_run: bool) {
    let f = fixture_with_options(None, true);
    let window = open(&f, None).await;
    let request = spawn(&window.session_id, "sudo-queued", None);
    // The owner approves this exact operation for the elevated turn.
    let confirmation = tokio::spawn({
        let state = f.state.clone();
        let window = window.clone();
        let request = request.clone();
        async move { state.confirm_sudo_scope(&window, &request).await }
    });
    approve(&f, &popup(&f.state).await, None).await.unwrap();
    confirmation.await.unwrap().unwrap();
    let probe = Arc::new(tokio::sync::Notify::new());
    let mut lane_state = f.state.clone();
    lane_state.observe_app_lock_wait_for_test(probe.clone());
    let app = f.app.lock().await;
    let runtime = SessionRuntime::with_queue_limit_and_focus_projection(
        lane_state.clone(),
        4,
        FocusedAgentProjection::default(),
        lane_state.owned.session_projection.clone(),
        lane_state.owned.agent_runtime_projection.clone(),
        app.terminal_stream_store(),
    );
    // Earlier session work holds the lane while it waits on the app lock.
    let blocker = spawn(&window.session_id, "lane-blocker", Some("missing-worker"));
    let earlier = tokio::spawn({
        let runtime = runtime.clone();
        let command = KernelCommand::from_local_request("earlier", None, None, &blocker);
        async move { runtime.dispatch_session_command(command, blocker).await }
    });
    tokio::time::timeout(Duration::from_secs(3), probe.notified())
        .await
        .unwrap();
    // The MCP call from the elevated turn is admitted and queues behind it.
    let mut command = KernelCommand::from_local_request(
        format!("{}:queued", window.entry_id),
        window.prompt_id.clone(),
        Some(window.entry_id.clone()),
        &request,
    );
    command.caller = KernelCaller::for_source(&KernelCommandSource::LocalIpc)
        .with_connection_class(KernelConnectionClass::KernelAgent);
    command.provider_run_id = if include_run {
        window.provider_run_id.clone()
    } else {
        None
    };
    command.caller.caller_id = window.entry_id.clone();
    command.caller.user_id = Some(window.owner_user_id.clone());
    command.caller.metaagent_id = Some(window.agent_id.clone());
    let queued = tokio::spawn({
        let runtime = runtime.clone();
        let request = request.clone();
        async move { runtime.dispatch_session_command(command, request).await }
    });
    tokio::time::timeout(Duration::from_secs(3), async {
        while runtime.lane_capacity(&window.session_id).await != Some(3) {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    if end_turn {
        // The elevated turn ends and a continuation of the same task starts.
        f.state
            .owned
            .prompt_state_owner
            .cancel_active_prompt_only(&session_of(&f, &window), &window.agent_id)
            .unwrap();
        f.state.sweep_sudo();
        let continuation = start_continuation(&f, &window);
        assert_ne!(Some(continuation), window.prompt_id);
    }
    drop(app);
    let _ = tokio::time::timeout(Duration::from_secs(5), earlier)
        .await
        .unwrap();
    let result = tokio::time::timeout(Duration::from_secs(5), queued)
        .await
        .unwrap()
        .unwrap();
    let expected_error = if end_turn {
        Some("original provider turn ended")
    } else if !include_run {
        Some("no running-turn binding")
    } else {
        None
    };
    if let Some(error) = expected_error {
        assert!(
            result
                .as_ref()
                .is_err_and(|e| e.to_string().contains(error)),
            "a queued command acquired another turn's authority: {result:?}"
        );
    } else {
        assert!(
            matches!(result, Ok(LocalDaemonResponse::AgentSpawned { .. })),
            "{result:?}"
        );
    }
    assert_eq!(
        f.state
            .owned
            .agent_store
            .list_agents()
            .iter()
            .any(|agent| agent.alias() == Some("sudo-queued")),
        expected_error.is_none()
    );
    f.state
        .revoke_sudo(Some("local"), Some(&window.entry_id), "fixture_cleanup")
        .unwrap();
}

fn session_of(
    f: &super::tests::Fixture,
    window: &KernelSudoTurn,
) -> crate::session::RuntimeSession {
    f.state
        .owned
        .session_store
        .get_session(&window.session_id)
        .unwrap()
}
