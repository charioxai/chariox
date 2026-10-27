use super::*;
use crate::local::CreateAgentWorkflowRequest;
use crate::session::{WorkflowOriginReason, WorkflowOriginSurface};

#[tokio::test]
async fn a_trigger_gives_the_agent_one_visible_workflow_with_its_origin() {
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let session = app
        .sessions_mut()
        .create_session(CreateSessionRequest::new(
            "workspace-agent-flow",
            "worktree-agent-flow",
        ))
        .expect("session should be created");
    let session_id = session.id().to_string();
    let agent = spawn_test_agent(&mut app, &session_id, "builder", "dev-stub");
    let router = CommandRouter::with_interactive_capacity(Arc::new(Mutex::new(app)), 4);

    let create = |agent_id: &str| {
        LocalDaemonRequest::CreateAgentWorkflow(CreateAgentWorkflowRequest {
            session_id: session_id.clone(),
            agent_id: agent_id.to_string(),
            reason: WorkflowOriginReason::Trigger,
            surface: WorkflowOriginSurface::Web,
            alias: None,
        })
    };
    let request = create(agent.id());
    let (workflow, endpoint) = match router
        .dispatch(
            KernelCommand::from_local_request("agent-workflow", None, None, &request),
            request,
        )
        .await
        .expect("the agent's workflow should be created")
    {
        LocalDaemonResponse::AgentWorkflowCreated {
            workflow, endpoint, ..
        } => (workflow, endpoint),
        other => panic!("unexpected response: {other:?}"),
    };
    assert_eq!(workflow.alias(), Some("builder-trigger"));
    assert_eq!(workflow.nodes().len(), 1);
    assert_eq!(workflow.endpoints().len(), 1);
    assert_eq!(endpoint.entry_node_id(), workflow.nodes()[0].id());
    let origin = workflow.origin().expect("origin is recorded");
    assert_eq!(origin.source_agent_id, agent.id());
    assert_eq!(origin.reason, WorkflowOriginReason::Trigger);
    assert_eq!(origin.surface, WorkflowOriginSurface::Web);

    // Another trigger for the same agent gets its own ordinary alias.
    let again = create(agent.id());
    match router
        .dispatch(
            KernelCommand::from_local_request("agent-workflow-again", None, None, &again),
            again,
        )
        .await
        .expect("a second workflow for the agent should be created")
    {
        LocalDaemonResponse::AgentWorkflowCreated { workflow, .. } => {
            assert_eq!(workflow.alias(), Some("builder-trigger-2"))
        }
        other => panic!("unexpected response: {other:?}"),
    }

    let unknown = create("agent-missing");
    assert!(matches!(
        router
            .dispatch(
                KernelCommand::from_local_request("agent-workflow-missing", None, None, &unknown),
                unknown,
            )
            .await,
        Err(DaemonError::AgentNotFound { .. })
    ));
}
