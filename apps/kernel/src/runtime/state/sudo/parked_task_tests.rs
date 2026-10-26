//! MP-08 / MP-09 / MP-10 / MP-11 G02: Stop uses the parked task ledger.
use super::goal_tests::blocked_task;
use super::tests::fixture_with_options;
use super::*;
use crate::durable_state::agent_lifecycle::ExecutionState;

#[tokio::test]
async fn stop_cancels_parked_task_through_prompt_route() {
    let f = fixture_with_options(None, true);
    let original = blocked_task(&f, false);
    let request = LocalDaemonRequest::CancelActivePrompt(CancelActivePromptRequest {
        session_id: f.request.session_id.clone(),
        attachment_id: f.request.attachment_id.clone(),
        target_agent_id: f.request.target_agent_id.clone(),
        expected_prompt_id: None,
    });
    let command = crate::runtime::command::KernelCommand::from_local_request(
        "stop-parked",
        None,
        None,
        &request,
    );
    let result = f.router.dispatch(command, request).await.unwrap();
    let LocalDaemonResponse::AgentTaskUpdated { task, .. } = result else {
        panic!("expected task cancellation")
    };
    assert_eq!(task.task_id, original.task_id);
    assert_eq!(task.state, ExecutionState::Cancelled);
}

#[tokio::test]
async fn prompt_specific_stop_does_not_cancel_parked_task() {
    let f = fixture_with_options(None, true);
    let original = blocked_task(&f, false);
    let request = LocalDaemonRequest::CancelActivePrompt(CancelActivePromptRequest {
        session_id: f.request.session_id.clone(),
        attachment_id: f.request.attachment_id.clone(),
        target_agent_id: f.request.target_agent_id.clone(),
        expected_prompt_id: Some("stale-prompt".into()),
    });
    let command = crate::runtime::command::KernelCommand::from_local_request(
        "stop-stale",
        None,
        None,
        &request,
    );
    assert!(f.router.dispatch(command, request).await.is_err());
    assert_eq!(
        f.state
            .owned
            .durable_state_store
            .agent_tasks(Some(&f.request.session_id), None)
            .unwrap()[0]
            .revision,
        original.revision
    );
}
