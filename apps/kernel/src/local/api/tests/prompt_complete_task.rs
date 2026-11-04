//! MP-08 / MP-09 / MP-10 / MP-11: a client that ends a turn with `CompletePrompt` ends its task too.
use super::*;
use crate::durable_state::agent_lifecycle::ExecutionState;

#[test]
fn completing_a_prompt_settles_its_kernel_task() {
    let worktree = crate::test_support::TestWorktree::new("complete-prompt-task");
    let mut config = DaemonConfig::for_tests();
    config.room_agent_tools = true;
    let harness = LocalRouterTestHarness::with_config(config);
    let (session, agent) = match harness
        .dispatch(LocalDaemonRequest::CreateSession(
            worktree
                .session_request()
                .with_agent_defaults(crate::session::SessionAgentDefaults::new("dev-stub")),
        ))
        .unwrap()
    {
        LocalDaemonResponse::SessionCreated { session, agent, .. } => (session, agent),
        other => panic!("unexpected response: {other:?}"),
    };
    harness.launch_workflow_test_provider(session.id(), agent.id());
    let attachment = match harness
        .dispatch(LocalDaemonRequest::AttachToSession(
            AttachToSessionRequest {
                session_id: session.id().into(),
                client_id: "complete-prompt".into(),
                capability_level: ClientCapabilityLevel::FullTerminal,
            },
        ))
        .unwrap()
    {
        LocalDaemonResponse::SessionAttached { attachment } => attachment,
        other => panic!("unexpected response: {other:?}"),
    };
    let prompt = match harness
        .dispatch(LocalDaemonRequest::SubmitPrompt(SubmitPromptRequest {
            session_id: session.id().into(),
            attachment_id: attachment.id().into(),
            target_agent_id: Some(agent.id().into()),
            prompt: "one turn".into(),
            attachments: vec![],
        }))
        .unwrap()
    {
        LocalDaemonResponse::PromptSubmitted {
            outcome: PromptSubmissionOutcome::Started { prompt },
            ..
        } => prompt,
        other => panic!("unexpected response: {other:?}"),
    };
    let task_state = || {
        harness.with_app(|app| {
            app.durable_state_store()
                .agent_turn_task(session.id(), agent.id(), prompt.id())
                .unwrap()
                .map(|task| task.state)
        })
    };
    assert_eq!(task_state(), Some(ExecutionState::Working));
    harness
        .dispatch(LocalDaemonRequest::CompletePrompt(CompletePromptRequest {
            session_id: session.id().into(),
        }))
        .unwrap();
    let unfinished = harness.with_app(|app| {
        app.durable_state_store()
            .agent_tasks(Some(session.id()), Some(agent.id()))
            .unwrap()
            .into_iter()
            .filter(|task| !matches!(task.state, ExecutionState::Done | ExecutionState::Cancelled))
            .count()
    });
    assert_eq!(
        (task_state(), unfinished),
        (Some(ExecutionState::Done), 0),
        "the completed turn must not leave its task working"
    );
}
