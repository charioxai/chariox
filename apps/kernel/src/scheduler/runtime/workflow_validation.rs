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
        crate::provider::adapter_key_for_provider(provider).to_string(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::CreateAgentRequest;
    use crate::app::KernelSessionService;
    use crate::session::{
        CreateSessionRequest, WorkflowDefinition, WorkflowEdgeDefinition, WorkflowNodeDefinition,
    };
    use crate::{DaemonApp, DaemonConfig};

    #[test]
    fn validates_alias_provider_nodes_before_lazy_launch() {
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
        let (session, _initial_agent) = KernelSessionService::new(&mut app)
            .create_session(CreateSessionRequest::new("workspace", "worktree"))
            .expect("session should be created");
        let claude_agent = KernelSessionService::new(&mut app)
            .spawn_agent(
                CreateAgentRequest::new(session.id(), "claude-p")
                    .with_alias("filter")
                    .with_model("sonnet"),
            )
            .expect("claude-p agent should spawn");
        let codex_agent = KernelSessionService::new(&mut app)
            .spawn_agent(
                CreateAgentRequest::new(session.id(), "codex")
                    .with_alias("finisher")
                    .with_model("gpt-5"),
            )
            .expect("codex agent should spawn");

        let mut workflow = WorkflowDefinition::new("workflow-1", Some("alias-provider".into()));
        workflow.add_node(WorkflowNodeDefinition::new(
            "node-filter",
            claude_agent.id(),
        ));
        workflow.add_node(WorkflowNodeDefinition::new(
            "node-finisher",
            codex_agent.id(),
        ));
        workflow.add_edge(WorkflowEdgeDefinition::new(
            "edge-filter-finisher",
            "node-filter",
            "node-finisher",
            Some("schema-filtered".into()),
            None,
        ));

        validate_workflow_agents(&app, session.id(), &workflow)
            .expect("claude-p should infer claude workflow controls before launch");
    }
}
