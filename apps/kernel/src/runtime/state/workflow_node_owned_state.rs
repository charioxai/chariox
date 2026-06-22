//! Workflow definition node mutations.

use super::*;

impl KernelRuntimeOwnedState {
    pub(super) fn workflow_add_node(
        &self,
        request: crate::local::AddWorkflowNodeRequest,
        caller_user_id: &str,
        caller_metaagent_id: Option<&str>,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        self.ensure_workflow_revision(
            &request.session_id,
            &request.workflow_ref,
            request.expected_workflow_revision,
        )?;
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
                        request.agent_id
                    ),
                });
            }
        }
        let node = self.session_store.write().add_workflow_node_owned(
            &request.session_id,
            &request.workflow_ref,
            &request.agent_id,
            agent.owner_user_id().to_string(),
            caller_user_id.to_string(),
            request.agent_id.clone(),
        )?;
        let workflow = self
            .session_store
            .read()
            .resolve_workflow_ref(&request.session_id, &request.workflow_ref)?;
        let session = self.workflow_session(&request.session_id)?;
        Ok(LocalDaemonResponse::WorkflowNodeAdded {
            node,
            workflow,
            session,
        })
    }

    pub(super) fn workflow_remove_node(
        &self,
        request: crate::local::RemoveWorkflowNodeRequest,
        caller_user_id: &str,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        self.ensure_workflow_revision(
            &request.session_id,
            &request.workflow_ref,
            request.expected_workflow_revision,
        )?;
        self.ensure_workflow_node_editor(
            &request.session_id,
            &request.workflow_ref,
            &request.node_id,
            caller_user_id,
            "remove workflow node",
        )?;
        let node = self.session_store.write().remove_workflow_node(
            &request.session_id,
            &request.workflow_ref,
            &request.node_id,
        )?;
        let workflow = self
            .session_store
            .read()
            .resolve_workflow_ref(&request.session_id, &request.workflow_ref)?;
        let session = self.workflow_session(&request.session_id)?;
        Ok(LocalDaemonResponse::WorkflowNodeRemoved {
            node,
            workflow,
            session,
        })
    }

    pub(super) fn workflow_update_node_instructions(
        &self,
        request: crate::local::UpdateWorkflowNodeInstructionsRequest,
        caller_user_id: &str,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        self.ensure_workflow_revision(
            &request.session_id,
            &request.workflow_ref,
            request.expected_workflow_revision,
        )?;
        self.ensure_workflow_node_editor(
            &request.session_id,
            &request.workflow_ref,
            &request.node_id,
            caller_user_id,
            "update workflow node instructions",
        )?;
        let node = self
            .session_store
            .write()
            .update_workflow_node_instructions(
                &request.session_id,
                &request.workflow_ref,
                &request.node_id,
                request.instructions,
            )?;
        let workflow = self
            .session_store
            .read()
            .resolve_workflow_ref(&request.session_id, &request.workflow_ref)?;
        let session = self.workflow_session(&request.session_id)?;
        Ok(LocalDaemonResponse::WorkflowNodeInstructionsUpdated {
            node,
            workflow,
            session,
        })
    }

    pub(super) fn workflow_set_node_can_complete_run(
        &self,
        request: crate::local::SetWorkflowNodeCanCompleteRunRequest,
        caller_user_id: &str,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        self.ensure_workflow_revision(
            &request.session_id,
            &request.workflow_ref,
            request.expected_workflow_revision,
        )?;
        self.ensure_workflow_node_editor(
            &request.session_id,
            &request.workflow_ref,
            &request.node_id,
            caller_user_id,
            "set workflow node completion policy",
        )?;
        let node = self
            .session_store
            .write()
            .set_workflow_node_can_complete_run(
                &request.session_id,
                &request.workflow_ref,
                &request.node_id,
                request.can_complete_workflow_run,
            )?;
        let workflow = self
            .session_store
            .read()
            .resolve_workflow_ref(&request.session_id, &request.workflow_ref)?;
        let session = self.workflow_session(&request.session_id)?;
        Ok(LocalDaemonResponse::WorkflowNodeCanCompleteRunUpdated {
            node,
            workflow,
            session,
        })
    }

    pub(super) fn workflow_set_node_can_emit_intermediate_output(
        &self,
        request: crate::local::SetWorkflowNodeCanEmitIntermediateOutputRequest,
        caller_user_id: &str,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        self.ensure_workflow_revision(
            &request.session_id,
            &request.workflow_ref,
            request.expected_workflow_revision,
        )?;
        self.ensure_workflow_node_editor(
            &request.session_id,
            &request.workflow_ref,
            &request.node_id,
            caller_user_id,
            "set workflow node intermediate output policy",
        )?;
        let node = self
            .session_store
            .write()
            .set_workflow_node_can_emit_intermediate_output(
                &request.session_id,
                &request.workflow_ref,
                &request.node_id,
                request.can_emit_intermediate_workflow_run_output,
            )?;
        let workflow = self
            .session_store
            .read()
            .resolve_workflow_ref(&request.session_id, &request.workflow_ref)?;
        let session = self.workflow_session(&request.session_id)?;
        Ok(
            LocalDaemonResponse::WorkflowNodeCanEmitIntermediateOutputUpdated {
                node,
                workflow,
                session,
            },
        )
    }

    pub(super) fn workflow_set_node_wait_for_all_inputs(
        &self,
        request: crate::local::SetWorkflowNodeWaitForAllInputsRequest,
        caller_user_id: &str,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        self.ensure_workflow_revision(
            &request.session_id,
            &request.workflow_ref,
            request.expected_workflow_revision,
        )?;
        self.ensure_workflow_node_editor(
            &request.session_id,
            &request.workflow_ref,
            &request.node_id,
            caller_user_id,
            "set workflow node input synchronization policy",
        )?;
        let node = self
            .session_store
            .write()
            .set_workflow_node_wait_for_all_inputs(
                &request.session_id,
                &request.workflow_ref,
                &request.node_id,
                request.wait_for_all_inputs,
            )?;
        let workflow = self
            .session_store
            .read()
            .resolve_workflow_ref(&request.session_id, &request.workflow_ref)?;
        let session = self.workflow_session(&request.session_id)?;
        Ok(LocalDaemonResponse::WorkflowNodeWaitForAllInputsUpdated {
            node,
            workflow,
            session,
        })
    }

    pub(super) fn workflow_set_node_intermediate_output_schema(
        &self,
        request: crate::local::SetWorkflowNodeIntermediateOutputSchemaRequest,
        caller_user_id: &str,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        self.ensure_workflow_revision(
            &request.session_id,
            &request.workflow_ref,
            request.expected_workflow_revision,
        )?;
        self.ensure_workflow_node_editor(
            &request.session_id,
            &request.workflow_ref,
            &request.node_id,
            caller_user_id,
            "set workflow node intermediate output schema",
        )?;
        let node = self
            .session_store
            .write()
            .set_workflow_node_intermediate_output_schema_ref(
                &request.session_id,
                &request.workflow_ref,
                &request.node_id,
                request.intermediate_output_schema_ref,
            )?;
        let workflow = self
            .session_store
            .read()
            .resolve_workflow_ref(&request.session_id, &request.workflow_ref)?;
        let session = self.workflow_session(&request.session_id)?;
        Ok(
            LocalDaemonResponse::WorkflowNodeIntermediateOutputSchemaUpdated {
                node,
                workflow,
                session,
            },
        )
    }

    pub(super) fn workflow_set_node_max_turns(
        &self,
        request: crate::local::SetWorkflowNodeMaxTurnsRequest,
        caller_user_id: &str,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        self.ensure_workflow_revision(
            &request.session_id,
            &request.workflow_ref,
            request.expected_workflow_revision,
        )?;
        self.ensure_workflow_node_editor(
            &request.session_id,
            &request.workflow_ref,
            &request.node_id,
            caller_user_id,
            "set workflow node max turns",
        )?;
        let node = self.session_store.write().set_workflow_node_max_turns(
            &request.session_id,
            &request.workflow_ref,
            &request.node_id,
            request.max_turns,
        )?;
        let workflow = self
            .session_store
            .read()
            .resolve_workflow_ref(&request.session_id, &request.workflow_ref)?;
        let session = self.workflow_session(&request.session_id)?;
        Ok(LocalDaemonResponse::WorkflowNodeMaxTurnsUpdated {
            node,
            workflow,
            session,
        })
    }
}
