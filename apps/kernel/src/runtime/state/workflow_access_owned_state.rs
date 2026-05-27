//! Workflow ownership and revision guards.

use super::*;

impl KernelRuntimeOwnedState {
    pub(super) fn deny_owner(
        user_id: &str,
        owner_user_id: &str,
        resource: String,
        operation: &'static str,
    ) -> DaemonError {
        DaemonError::OwnershipAccessDenied {
            user_id: user_id.to_string(),
            owner_user_id: owner_user_id.to_string(),
            resource,
            operation,
        }
    }

    pub(super) fn ensure_workflow_node_editor(
        &self,
        session_id: &str,
        workflow_ref: &str,
        node_id: &str,
        user_id: &str,
        operation: &'static str,
    ) -> Result<(), DaemonError> {
        let sessions = self.session_store.read();
        let session = sessions.get_session(session_id)?;
        let workflow = sessions.resolve_workflow_ref(session_id, workflow_ref)?;
        let node = workflow
            .node(node_id)
            .ok_or_else(|| DaemonError::WorkflowNodeNotFound {
                session_id: session_id.to_string(),
                workflow_id: workflow.id().to_string(),
                node_id: node_id.to_string(),
            })?;
        let full_collaboration = session
            .collaboration_level_for_user(user_id)
            .is_some_and(|level| level.can_prompt_agent_directly());
        if node.owner_user_id() == user_id
            || node.created_by_user_id() == user_id
            || full_collaboration
        {
            Ok(())
        } else {
            Err(Self::deny_owner(
                user_id,
                node.created_by_user_id(),
                format!("workflow node `{node_id}`"),
                operation,
            ))
        }
    }

    pub(super) fn ensure_workflow_endpoint_owner(
        &self,
        session_id: &str,
        workflow_ref: &str,
        endpoint_ref: &str,
        user_id: &str,
        operation: &'static str,
    ) -> Result<(), DaemonError> {
        let sessions = self.session_store.read();
        let endpoint =
            sessions.resolve_workflow_endpoint_ref(session_id, workflow_ref, endpoint_ref)?;
        if endpoint.owner_user_id() == user_id {
            Ok(())
        } else {
            Err(Self::deny_owner(
                user_id,
                endpoint.owner_user_id(),
                format!("workflow endpoint `{endpoint_ref}`"),
                operation,
            ))
        }
    }

    pub(super) fn ensure_workflow_revision(
        &self,
        session_id: &str,
        workflow_ref: &str,
        expected_revision: Option<u64>,
    ) -> Result<(), DaemonError> {
        let Some(expected_revision) = expected_revision else {
            return Ok(());
        };
        let sessions = self.session_store.read();
        let workflow = sessions.resolve_workflow_ref(session_id, workflow_ref)?;
        let current_revision = workflow.revision();
        if current_revision == expected_revision {
            Ok(())
        } else {
            Err(DaemonError::WorkflowRevisionConflict {
                session_id: session_id.to_string(),
                workflow_id: workflow.id().to_string(),
                expected_revision,
                current_revision,
            })
        }
    }

    pub(super) fn ensure_workflow_edge_incident_to_owner(
        &self,
        session_id: &str,
        workflow_ref: &str,
        edge_id: &str,
        user_id: &str,
        operation: &'static str,
    ) -> Result<(), DaemonError> {
        let sessions = self.session_store.read();
        let session = sessions.get_session(session_id)?;
        let workflow = sessions.resolve_workflow_ref(session_id, workflow_ref)?;
        let edge = workflow
            .edge(edge_id)
            .ok_or_else(|| DaemonError::WorkflowEdgeNotFound {
                session_id: session_id.to_string(),
                workflow_id: workflow.id().to_string(),
                edge_id: edge_id.to_string(),
            })?;
        let full_collaboration = session
            .collaboration_level_for_user(user_id)
            .is_some_and(|level| level.can_prompt_agent_directly());
        let can_edit_endpoint = |node_id: &str| {
            workflow.node(node_id).is_some_and(|node| {
                node.owner_user_id() == user_id || node.created_by_user_id() == user_id
            })
        };
        if full_collaboration
            || can_edit_endpoint(edge.from_node_id())
            || can_edit_endpoint(edge.to_node_id())
            || edge.created_by_user_id() == user_id
        {
            Ok(())
        } else {
            Err(Self::deny_owner(
                user_id,
                edge.created_by_user_id(),
                format!("workflow edge `{edge_id}`"),
                operation,
            ))
        }
    }
}
