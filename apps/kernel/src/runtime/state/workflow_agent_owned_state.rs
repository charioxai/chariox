//! Workflows generated from an agent. A Freeform agent gains a workflow only
//! when it gets a trigger or a deployment: one visible node for that agent,
//! one entry endpoint, and the recorded origin. Binding Apps or Extensions to
//! the agent never comes here.

use super::*;

impl KernelRuntimeOwnedState {
    pub(super) fn workflow_create_from_agent(
        &self,
        request: crate::local::CreateAgentWorkflowRequest,
        caller_user_id: &str,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let Some(agent) = self
            .agent_store
            .get_session_agents(&request.session_id)
            .into_iter()
            .find(|agent| agent.id() == request.agent_id)
        else {
            return Err(DaemonError::AgentNotFound {
                agent_id: request.agent_id,
            });
        };
        if agent.is_metaagent() || agent.owner_user_id() != caller_user_id {
            return Err(DaemonError::LocalTransport {
                operation: "workflow.create_from_agent",
                message: "only the owner of an ordinary agent can give it a trigger or deployment"
                    .to_string(),
            });
        }
        let alias = request.alias.or_else(|| {
            let name = agent.alias().unwrap_or(agent.id());
            let reason = match request.reason {
                crate::session::WorkflowOriginReason::Trigger => "trigger",
                crate::session::WorkflowOriginReason::Deploy => "deploy",
            };
            Some(format!("{name}-{reason}"))
        });
        let workflow = self
            .session_store
            .write()
            .create_workflow_controlled_by_metaagent(&request.session_id, alias, None)?;
        self.session_store.write().set_workflow_origin(
            &request.session_id,
            workflow.id(),
            crate::session::WorkflowOrigin {
                source_agent_id: agent.id().to_string(),
                reason: request.reason,
                surface: request.surface,
                created_at_ms: crate::session::unix_epoch_ms(),
            },
        )?;
        let node = self.session_store.write().add_workflow_node_owned(
            &request.session_id,
            workflow.id(),
            agent.id(),
            agent.owner_user_id().to_string(),
            caller_user_id.to_string(),
            agent.id().to_string(),
        )?;
        let endpoint = self.session_store.write().create_workflow_endpoint(
            &request.session_id,
            workflow.id(),
            node.id(),
            None,
        )?;
        let endpoint = self.session_store.write().set_workflow_endpoint_owner(
            &request.session_id,
            workflow.id(),
            endpoint.id(),
            caller_user_id.to_string(),
        )?;
        let workflow = self
            .session_store
            .read()
            .resolve_workflow_ref(&request.session_id, workflow.id())?;
        let session = self.workflow_session(&request.session_id)?;
        Ok(LocalDaemonResponse::AgentWorkflowCreated {
            workflow,
            endpoint,
            session,
        })
    }
}
