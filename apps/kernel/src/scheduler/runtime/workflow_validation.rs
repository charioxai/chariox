use super::*;

pub fn validate_workflow_agents(
    app: &DaemonApp,
    session_id: &str,
    workflow: &WorkflowDefinition,
) -> Result<(), DaemonError> {
    let agents = app
        .agents()
        .get_session_agents(session_id)
        .into_iter()
        .collect::<Vec<_>>();
    let agent_ids = agents
        .iter()
        .map(|agent| agent.id().to_string())
        .collect::<BTreeSet<_>>();
    for node in workflow.nodes() {
        if !agent_ids.contains(node.agent_id()) {
            return Err(DaemonError::WorkflowNodeAgentMissing {
                session_id: session_id.to_string(),
                workflow_id: workflow.id().to_string(),
                node_id: node.id().to_string(),
                agent_id: node.agent_id().to_string(),
            });
        }
        let Some(agent) = agents.iter().find(|agent| agent.id() == node.agent_id()) else {
            continue;
        };
        let capabilities =
            workflow_node_control_capabilities(app, session_id, node.agent_id(), agent.provider());
        if !capabilities.supports_control_operation(ControlOperation::AckWorkflowTurn) {
            return Err(DaemonError::WorkflowNodeControlUnsupported {
                session_id: session_id.to_string(),
                workflow_id: workflow.id().to_string(),
                node_id: node.id().to_string(),
                agent_id: node.agent_id().to_string(),
                operation: "ack_workflow_turn",
            });
        }
        let requires_validation = workflow
            .edges()
            .iter()
            .any(|edge| edge.from_node_id() == node.id() && edge.handoff_schema_ref().is_some());
        if requires_validation
            && !capabilities.supports_control_operation(ControlOperation::ValidateWorkflowHandoff)
        {
            return Err(DaemonError::WorkflowNodeControlUnsupported {
                session_id: session_id.to_string(),
                workflow_id: workflow.id().to_string(),
                node_id: node.id().to_string(),
                agent_id: node.agent_id().to_string(),
                operation: "validate_workflow_handoff",
            });
        }
    }
    Ok(())
}

fn workflow_node_control_capabilities(
    app: &DaemonApp,
    session_id: &str,
    agent_id: &str,
    provider: &str,
) -> RuntimeProviderRun {
    if let Some(run) = app.providers().get_run_for_agent(session_id, agent_id) {
        return run;
    }

    RuntimeProviderRun::from_control_capability_inference(
        format!("inferred-{session_id}-{agent_id}"),
        session_id.to_string(),
        Some(agent_id.to_string()),
        provider.to_string(),
    )
}
