//! MP-08/MP-11: prompt lanes must retain the submitting sudo turn.
use super::tests::{fixture_with_options, popup, Fixture};
use super::window_tests::{approve, open, start_continuation};
use super::*;
use crate::runtime::agent_actor::AgentRuntime;
use crate::runtime::command::{KernelCaller, KernelCommand, KernelCommandSource};
use crate::runtime::session_actor::FocusedAgentProjection;

#[tokio::test]
async fn queued_sudo_agent_submit_rejects_a_later_continuation() {
    assert_queued_prompt(false, true, true, false).await;
}

#[tokio::test]
async fn queued_sudo_agent_submit_accepts_its_submitting_turn() {
    assert_queued_prompt(false, false, true, false).await;
}

#[tokio::test]
async fn queued_sudo_agent_submit_rejects_a_missing_binding() {
    assert_queued_prompt(false, false, false, false).await;
}

#[tokio::test]
async fn queued_sudo_agent_cancel_rejects_a_later_continuation() {
    assert_queued_prompt(true, true, true, false).await;
}

#[tokio::test]
async fn queued_sudo_agent_cancel_accepts_its_submitting_turn() {
    assert_queued_prompt(true, false, true, false).await;
}

#[tokio::test]
async fn queued_sudo_agent_cancel_rejects_a_missing_binding() {
    assert_queued_prompt(true, false, false, false).await;
}

#[tokio::test]
async fn queued_sudo_agent_cold_submit_rechecks_after_waiting_for_app_lock() {
    assert_queued_prompt(false, true, true, true).await;
}

fn replace_turn(f: &Fixture, window: &KernelSudoTurn) {
    let session = f
        .state
        .owned
        .session_store
        .get_session(&window.session_id)
        .unwrap();
    f.state
        .owned
        .prompt_state_owner
        .cancel_active_prompt_only(&session, &window.agent_id)
        .unwrap();
    f.state.sweep_sudo();
    assert_ne!(Some(start_continuation(f, window)), window.prompt_id);
}

async fn assert_queued_prompt(cancel: bool, replace: bool, include_run: bool, cold: bool) {
    let f = fixture_with_options(None, true);
    let window = open(&f, None).await;
    let target = if cold {
        crate::app::KernelSessionService::new(&mut *f.app.lock().await)
            .spawn_agent(crate::agent::CreateAgentRequest::new(
                &window.session_id,
                "dev-stub",
            ))
            .unwrap()
            .id()
            .to_owned()
    } else {
        window.agent_id.clone()
    };
    let request = if cancel {
        LocalDaemonRequest::CancelActivePrompt(CancelActivePromptRequest {
            session_id: window.session_id.clone(),
            attachment_id: f.request.attachment_id.clone(),
            target_agent_id: Some(target.clone()),
        })
    } else {
        let mut request = f.request.clone();
        request.prompt = "queued sudo regression".into();
        request.target_agent_id = Some(target.clone());
        LocalDaemonRequest::SubmitPrompt(request)
    };
    let confirmation = tokio::spawn({
        let state = f.state.clone();
        let window = window.clone();
        let request = request.clone();
        async move { state.confirm_sudo_scope(&window, &request).await }
    });
    approve(&f, &popup(&f.state).await, None).await.unwrap();
    confirmation.await.unwrap().unwrap();
    let runtime = AgentRuntime::new(
        f.state.clone(),
        Default::default(),
        FocusedAgentProjection::default(),
        f.state.owned.session_projection.clone(),
        f.state.owned.agent_runtime_projection.clone(),
        f.state.owned.prompt_state_owner.clone(),
        Default::default(),
    );
    let mut command = KernelCommand::from_local_request(
        "sudo-prompt-queue",
        window.prompt_id.clone(),
        Some(window.entry_id.clone()),
        &request,
    );
    command.caller = KernelCaller::for_source(&KernelCommandSource::LocalIpc)
        .with_connection_class(KernelConnectionClass::KernelAgent);
    command.caller.caller_id = window.entry_id.clone();
    command.caller.user_id = Some(window.owner_user_id.clone());
    command.caller.metaagent_id = Some(window.agent_id.clone());
    command.provider_run_id = include_run
        .then(|| window.provider_run_id.clone())
        .flatten();
    let (capacity, lane) = runtime.paused_agent_lane_for_test(&target).await;
    let initial_capacity = capacity();
    let dispatch = async {
        match request.clone() {
            LocalDaemonRequest::SubmitPrompt(r) => {
                runtime.dispatch_prompt_submit(&command, r).await
            }
            LocalDaemonRequest::CancelActivePrompt(r) => {
                runtime.dispatch_prompt_cancel(&command, r).await
            }
            _ => unreachable!(),
        }
    };
    tokio::pin!(dispatch, lane);
    assert!(futures_util::poll!(&mut dispatch).is_pending());
    assert_eq!(
        capacity(),
        initial_capacity - 1,
        "command must be queued before replacing its turn"
    );
    let app_lock = if cold {
        let lock = f.app.lock().await;
        assert!(futures_util::poll!(&mut lane).is_pending());
        assert_eq!(
            capacity(),
            initial_capacity,
            "consumer must reach the cold-launch app lock"
        );
        Some(lock)
    } else {
        None
    };
    if replace {
        replace_turn(&f, &window);
    }
    let before = f.state.owned.session_snapshot(&window.session_id).unwrap();
    drop(app_lock);
    let result = tokio::time::timeout(Duration::from_secs(3), async {
        tokio::select! { result = &mut dispatch => result, _ = &mut lane => panic!("lane closed") }
    })
    .await
    .unwrap();
    let error = if replace {
        Some("original provider turn ended")
    } else if !include_run {
        Some("no running-turn binding")
    } else {
        None
    };
    if let Some(error) = error {
        assert!(
            result
                .as_ref()
                .is_err_and(|e| e.to_string().contains(error)),
            "queued prompt borrowed a different turn: {result:?}"
        );
        let after = f.state.owned.session_snapshot(&window.session_id).unwrap();
        assert_eq!(
            after.active_prompt_for_agent(&target),
            before.active_prompt_for_agent(&target)
        );
        assert_eq!(
            after.queued_prompts_for_agent(&target),
            before.queued_prompts_for_agent(&target)
        );
        if cold {
            assert!(
                f.state
                    .owned
                    .provider_store
                    .get_run_for_agent(&window.session_id, &target)
                    .is_none(),
                "ended submitting turn must not launch the cold provider"
            );
        }
    } else if cancel {
        assert!(
            matches!(result, Ok(LocalDaemonResponse::PromptCancelled { .. })),
            "{result:?}"
        );
    } else {
        assert!(
            matches!(result, Ok(LocalDaemonResponse::PromptSubmitted { .. })),
            "{result:?}"
        );
    }
    f.state
        .revoke_sudo(Some("local"), Some(&window.entry_id), "fixture_cleanup")
        .unwrap();
}
