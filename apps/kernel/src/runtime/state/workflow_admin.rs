//! Workflow administrative mutations.
//!
//! Owns workflow CRUD, endpoint edits, watchdog updates, and queue-facing commands that alter
//! workflow definitions rather than executing an individual node.

use super::*;

impl KernelRuntimeOwnedState {
    pub(super) fn workflow_session(
        &self,
        session_id: &str,
    ) -> Result<crate::session::RuntimeSession, DaemonError> {
        self.session_snapshot(session_id)
    }

    pub(super) fn workflow_create_workflow(
        &self,
        request: crate::local::CreateWorkflowRequest,
        controlled_by_metaagent_id: Option<&str>,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let workflow = self
            .session_store
            .write()
            .create_workflow_controlled_by_metaagent(
                &request.session_id,
                request.alias,
                controlled_by_metaagent_id.map(str::to_string),
            )?;
        let session = self.workflow_session(&request.session_id)?;
        Ok(LocalDaemonResponse::WorkflowCreated { workflow, session })
    }

    pub(super) fn workflow_apply_design_op(
        &self,
        request: crate::local::ApplyWorkflowDesignOpRequest,
        caller_user_id: &str,
        caller_metaagent_id: Option<&str>,
    ) -> Result<crate::session::RuntimeSession, DaemonError> {
        let node_owner_user_id = self.ensure_workflow_design_op_authorized(
            &request.session_id,
            &request.op,
            caller_user_id,
            caller_metaagent_id,
        )?;
        self.session_store
            .write()
            .apply_workflow_design_op_with_authority(
                &request.session_id,
                request.op,
                caller_user_id.to_string(),
                node_owner_user_id,
                caller_metaagent_id.map(str::to_string),
            )?;
        self.workflow_session(&request.session_id)
    }

    fn ensure_workflow_design_op_authorized(
        &self,
        session_id: &str,
        op: &crate::local::WorkflowDesignOp,
        caller_user_id: &str,
        caller_metaagent_id: Option<&str>,
    ) -> Result<Option<String>, DaemonError> {
        use crate::local::WorkflowDesignOp;

        let workflow_ref = match op {
            WorkflowDesignOp::WorkflowCreate { .. } => None,
            WorkflowDesignOp::WorkflowUpdate { workflow_id, .. }
            | WorkflowDesignOp::WorkflowRemove { workflow_id }
            | WorkflowDesignOp::SchemaAdd { workflow_id, .. }
            | WorkflowDesignOp::SchemaUpdate { workflow_id, .. }
            | WorkflowDesignOp::SchemaRemove { workflow_id, .. }
            | WorkflowDesignOp::NodeAdd { workflow_id, .. }
            | WorkflowDesignOp::NodeUpdate { workflow_id, .. }
            | WorkflowDesignOp::NodeMove { workflow_id, .. }
            | WorkflowDesignOp::NodeRemove { workflow_id, .. }
            | WorkflowDesignOp::EdgeAdd { workflow_id, .. }
            | WorkflowDesignOp::EdgeUpdate { workflow_id, .. }
            | WorkflowDesignOp::EdgeRemove { workflow_id, .. }
            | WorkflowDesignOp::EndpointAdd { workflow_id, .. }
            | WorkflowDesignOp::EndpointUpdate { workflow_id, .. }
            | WorkflowDesignOp::EndpointMove { workflow_id, .. }
            | WorkflowDesignOp::EndpointRemove { workflow_id, .. } => Some(workflow_id.as_str()),
        };
        if let (Some(metaagent_id), Some(workflow_ref)) = (caller_metaagent_id, workflow_ref) {
            self.ensure_workflow_controlled_by_metaagent(
                session_id,
                workflow_ref,
                metaagent_id,
                "apply workflow design op",
            )?;
        }

        match op {
            WorkflowDesignOp::NodeAdd { node, .. } => {
                let agent = self
                    .agent_store
                    .get_session_agents(session_id)
                    .into_iter()
                    .find(|agent| agent.id() == node.agent_id)
                    .ok_or_else(|| DaemonError::AgentNotFound {
                        agent_id: node.agent_id.clone(),
                    })?;
                if agent.is_metaagent() {
                    return Err(DaemonError::LocalTransport {
                        operation: "workflow.node.add",
                        message: "metaagents cannot be added as workflow nodes".to_string(),
                    });
                }
                if let Some(metaagent_id) = caller_metaagent_id {
                    if agent.controlled_by_metaagent_id() != Some(metaagent_id) {
                        return Err(DaemonError::LocalTransport {
                            operation: "workflow.node.add",
                            message: format!(
                                "agent `{}` is not controlled by metaagent `{metaagent_id}`",
                                node.agent_id
                            ),
                        });
                    }
                }
                Ok(Some(agent.owner_user_id().to_string()))
            }
            WorkflowDesignOp::NodeUpdate {
                workflow_id,
                node_id,
                ..
            }
            | WorkflowDesignOp::NodeRemove {
                workflow_id,
                node_id,
            } => {
                self.ensure_workflow_node_editor(
                    session_id,
                    workflow_id,
                    node_id,
                    caller_user_id,
                    "apply workflow node design op",
                )?;
                Ok(None)
            }
            WorkflowDesignOp::EdgeAdd { workflow_id, edge } => {
                let workflows = self.session_store.read();
                let workflow = workflows.resolve_workflow_ref(session_id, workflow_id)?;
                let from_owner = workflow
                    .node(&edge.from_node_id)
                    .map(|node| node.owner_user_id())
                    .ok_or_else(|| DaemonError::WorkflowNodeNotFound {
                        session_id: session_id.to_string(),
                        workflow_id: workflow.id().to_string(),
                        node_id: edge.from_node_id.clone(),
                    })?;
                let to_owner = workflow
                    .node(&edge.to_node_id)
                    .map(|node| node.owner_user_id())
                    .ok_or_else(|| DaemonError::WorkflowNodeNotFound {
                        session_id: session_id.to_string(),
                        workflow_id: workflow.id().to_string(),
                        node_id: edge.to_node_id.clone(),
                    })?;
                if from_owner != caller_user_id && to_owner != caller_user_id {
                    return Err(Self::deny_owner(
                        caller_user_id,
                        from_owner,
                        format!(
                            "workflow edge `{} -> {}`",
                            edge.from_node_id, edge.to_node_id
                        ),
                        "add workflow edge",
                    ));
                }
                Ok(None)
            }
            WorkflowDesignOp::EdgeUpdate {
                workflow_id,
                edge_id,
                ..
            }
            | WorkflowDesignOp::EdgeRemove {
                workflow_id,
                edge_id,
            } => {
                self.ensure_workflow_edge_incident_to_owner(
                    session_id,
                    workflow_id,
                    edge_id,
                    caller_user_id,
                    "apply workflow edge design op",
                )?;
                Ok(None)
            }
            WorkflowDesignOp::EndpointAdd {
                workflow_id,
                endpoint,
                ..
            } => {
                self.ensure_workflow_node_editor(
                    session_id,
                    workflow_id,
                    &endpoint.entry_node_id,
                    caller_user_id,
                    "create workflow endpoint",
                )?;
                Ok(None)
            }
            WorkflowDesignOp::EndpointUpdate {
                workflow_id,
                endpoint_id,
                patch,
            } => {
                self.ensure_workflow_endpoint_owner(
                    session_id,
                    workflow_id,
                    endpoint_id,
                    caller_user_id,
                    "update workflow endpoint",
                )?;
                if let Some(entry_node_id) = patch.entry_node_id.as_deref() {
                    self.ensure_workflow_node_editor(
                        session_id,
                        workflow_id,
                        entry_node_id,
                        caller_user_id,
                        "bind workflow endpoint",
                    )?;
                }
                Ok(None)
            }
            WorkflowDesignOp::EndpointRemove {
                workflow_id,
                endpoint_id,
            } => {
                self.ensure_workflow_endpoint_owner(
                    session_id,
                    workflow_id,
                    endpoint_id,
                    caller_user_id,
                    "apply workflow endpoint design op",
                )?;
                Ok(None)
            }
            WorkflowDesignOp::WorkflowCreate { .. }
            | WorkflowDesignOp::WorkflowUpdate { .. }
            | WorkflowDesignOp::WorkflowRemove { .. }
            | WorkflowDesignOp::SchemaAdd { .. }
            | WorkflowDesignOp::SchemaUpdate { .. }
            | WorkflowDesignOp::SchemaRemove { .. }
            | WorkflowDesignOp::NodeMove { .. }
            | WorkflowDesignOp::EndpointMove { .. } => Ok(None),
        }
    }

    pub(super) fn workflow_alias_workflow(
        &self,
        request: crate::local::AliasWorkflowRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        self.ensure_workflow_revision(
            &request.session_id,
            &request.workflow_ref,
            request.expected_workflow_revision,
        )?;
        let workflow = self.session_store.write().assign_workflow_alias(
            &request.session_id,
            &request.workflow_ref,
            request.alias,
        )?;
        let session = self.workflow_session(&request.session_id)?;
        Ok(LocalDaemonResponse::WorkflowAliased { workflow, session })
    }
}
