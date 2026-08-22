use super::*;

struct TestWorkflowEndpoint {
    workflow: crate::session::WorkflowDefinition,
    endpoint: crate::session::WorkflowEndpointDefinition,
}

fn workflow_with_endpoint(
    service: &mut SessionService,
    session_id: &str,
    alias: &str,
    agent_id: &str,
) -> TestWorkflowEndpoint {
    let workflow = service
        .create_workflow(session_id, Some(alias.to_string()))
        .expect("workflow should be created");
    let node = service
        .add_workflow_node(session_id, workflow.id(), agent_id)
        .expect("workflow node should be added");
    let endpoint = service
        .create_workflow_endpoint(
            session_id,
            workflow.id(),
            node.id(),
            Some("entry".to_string()),
        )
        .expect("workflow endpoint should be created");
    TestWorkflowEndpoint { workflow, endpoint }
}

mod console;
mod endpoint_instances;
mod lifecycle;
mod prompt_queues;
mod publication_observability;
mod watchdog_budgets;
mod watchdog_policies;
