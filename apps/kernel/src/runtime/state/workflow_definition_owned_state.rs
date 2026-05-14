//! Workflow definition edge and canvas topology mutations.
//!
//! This module owns workflow edge and canvas layout edits. Node, definition-level settings,
//! publication admin, endpoint admin, and run/watchdog queue admin live in separate modules.

use super::*;

impl KernelRuntimeOwnedState {
    pub(super) fn workflow_add_edge(
        &self,
        request: crate::local::AddWorkflowEdgeRequest,
        caller_user_id: &str,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        self.ensure_workflow_revision(
            &request.session_id,
            &request.workflow_ref,
            request.expected_workflow_revision,
        )?;
        let workflow = self
            .session_store
            .read()
            .resolve_workflow_ref(&request.session_id, &request.workflow_ref)?;
        let from_owner = workflow
            .node(&request.from_node_id)
            .map(|node| node.owner_user_id())
            .ok_or_else(|| DaemonError::WorkflowNodeNotFound {
                session_id: request.session_id.clone(),
                workflow_id: workflow.id().to_string(),
                node_id: request.from_node_id.clone(),
            })?;
        let to_owner = workflow
            .node(&request.to_node_id)
            .map(|node| node.owner_user_id())
            .ok_or_else(|| DaemonError::WorkflowNodeNotFound {
                session_id: request.session_id.clone(),
                workflow_id: workflow.id().to_string(),
                node_id: request.to_node_id.clone(),
            })?;
        if from_owner != caller_user_id && to_owner != caller_user_id {
            return Err(Self::deny_owner(
                caller_user_id,
                from_owner,
                format!(
                    "workflow edge `{} -> {}`",
                    request.from_node_id, request.to_node_id
                ),
                "add workflow edge",
            ));
        }
        let edge = self.session_store.write().add_workflow_edge_owned(
            &request.session_id,
            &request.workflow_ref,
            &request.from_node_id,
            &request.to_node_id,
            caller_user_id.to_string(),
            request.output_schema_ref,
            request.validation_policy,
        )?;
        let workflow = self
            .session_store
            .read()
            .resolve_workflow_ref(&request.session_id, &request.workflow_ref)?;
        let session = self.workflow_session(&request.session_id)?;
        Ok(LocalDaemonResponse::WorkflowEdgeAdded {
            edge,
            workflow,
            session,
        })
    }

    pub(super) fn workflow_remove_edge(
        &self,
        request: crate::local::RemoveWorkflowEdgeRequest,
        caller_user_id: &str,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        self.ensure_workflow_revision(
            &request.session_id,
            &request.workflow_ref,
            request.expected_workflow_revision,
        )?;
        self.ensure_workflow_edge_incident_to_owner(
            &request.session_id,
            &request.workflow_ref,
            &request.edge_id,
            caller_user_id,
            "remove workflow edge",
        )?;
        let edge = self.session_store.write().remove_workflow_edge(
            &request.session_id,
            &request.workflow_ref,
            &request.edge_id,
        )?;
        let workflow = self
            .session_store
            .read()
            .resolve_workflow_ref(&request.session_id, &request.workflow_ref)?;
        let session = self.workflow_session(&request.session_id)?;
        Ok(LocalDaemonResponse::WorkflowEdgeRemoved {
            edge,
            workflow,
            session,
        })
    }

    pub(super) fn workflow_update_canvas_layout(
        &self,
        request: crate::local::UpdateWorkflowCanvasLayoutRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let layout = self.session_store.write().update_workflow_canvas_layout(
            &request.session_id,
            &request.workflow_ref,
            request.patches,
        )?;
        let workflow = self
            .session_store
            .read()
            .resolve_workflow_ref(&request.session_id, &request.workflow_ref)?;
        let session = self.workflow_session(&request.session_id)?;
        Ok(LocalDaemonResponse::WorkflowCanvasLayoutUpdated {
            layout,
            workflow,
            session,
        })
    }
}
