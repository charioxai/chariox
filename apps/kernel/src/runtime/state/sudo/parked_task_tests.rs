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

#[tokio::test]
async fn owner_stop_keeps_existing_attachment_membership_authority() {
    let f = fixture_with_options(None, true);
    let original = blocked_task(&f, false);
    let attachment = {
        let mut app = f.app.lock().await;
        crate::app::KernelSessionService::new(&mut app)
            .attach(crate::attachment::AttachRequest::for_user(
                &f.request.session_id,
                "member-terminal",
                crate::attachment::ClientCapabilityLevel::FullTerminal,
                "room-member",
            ))
            .unwrap()
    };
    let request = LocalDaemonRequest::CancelActivePrompt(CancelActivePromptRequest {
        session_id: f.request.session_id.clone(),
        attachment_id: attachment.id().into(),
        target_agent_id: f.request.target_agent_id.clone(),
        expected_prompt_id: None,
    });
    let session = f
        .state
        .owned
        .session_store
        .get_session(&f.request.session_id)
        .unwrap();
    let agent = f.request.target_agent_id.as_deref().unwrap();
    f.state
        .owned
        .prompt_state_owner
        .submit_prepared_prompt_with_queue_policy(
            &session,
            PromptQueueItem::new(
                "active-parity",
                attachment.id(),
                agent,
                "Current ordinary work",
                PromptStatus::Queued,
            ),
            false,
            false,
        )
        .unwrap();
    let command = crate::runtime::command::KernelCommand::from_local_request(
        "owner-stop-active-member-terminal",
        None,
        None,
        &request,
    );
    assert!(matches!(
        f.router.dispatch(command, request.clone()).await.unwrap(),
        LocalDaemonResponse::PromptCancelled { .. }
    ));
    f.state
        .owned
        .prompt_state_owner
        .cancel_active_prompt_only(&session, agent);
    // The normal prompt route authorizes the caller against the agent owner;
    // membership of the attachment is checked separately in that same room.
    let command = crate::runtime::command::KernelCommand::from_local_request(
        "owner-stop-member-terminal",
        None,
        None,
        &request,
    );
    let response = f.router.dispatch(command, request).await.unwrap();
    let LocalDaemonResponse::AgentTaskUpdated { task, .. } = response else {
        panic!()
    };
    assert_eq!(task.task_id, original.task_id);
    assert_eq!(task.state, ExecutionState::Cancelled);
}
