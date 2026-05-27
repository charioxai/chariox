//! Workflow endpoint ownership mutations.
//!
//! This module owns endpoint creation, aliasing, and binding for workflow definitions.

use super::*;

impl KernelRuntimeOwnedState {
    pub(super) fn workflow_create_endpoint(
        &self,
        request: crate::local::CreateWorkflowEndpointRequest,
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
            &request.entry_node_id,
            caller_user_id,
            "create workflow endpoint",
        )?;
        let endpoint = self.session_store.write().create_workflow_endpoint(
            &request.session_id,
            &request.workflow_ref,
            &request.entry_node_id,
            request.alias,
        )?;
        let endpoint = self.session_store.write().set_workflow_endpoint_owner(
            &request.session_id,
            &request.workflow_ref,
            endpoint.id(),
            caller_user_id.to_string(),
        )?;
        let workflow = self
            .session_store
            .read()
            .resolve_workflow_ref(&request.session_id, &request.workflow_ref)?;
        let session = self.workflow_session(&request.session_id)?;
        Ok(LocalDaemonResponse::WorkflowEndpointCreated {
            endpoint,
            workflow,
            session,
        })
    }

    pub(super) fn workflow_alias_endpoint(
        &self,
        request: crate::local::AliasWorkflowEndpointRequest,
        caller_user_id: &str,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        self.ensure_workflow_revision(
            &request.session_id,
            &request.workflow_ref,
            request.expected_workflow_revision,
        )?;
        self.ensure_workflow_endpoint_owner(
            &request.session_id,
            &request.workflow_ref,
            &request.endpoint_ref,
            caller_user_id,
            "alias workflow endpoint",
        )?;
        let endpoint = self.session_store.write().assign_workflow_endpoint_alias(
            &request.session_id,
            &request.workflow_ref,
            &request.endpoint_ref,
            request.alias,
        )?;
        let workflow = self
            .session_store
            .read()
            .resolve_workflow_ref(&request.session_id, &request.workflow_ref)?;
        let session = self.workflow_session(&request.session_id)?;
        Ok(LocalDaemonResponse::WorkflowEndpointAliased {
            endpoint,
            workflow,
            session,
        })
    }

    pub(super) fn workflow_bind_endpoint(
        &self,
        request: crate::local::BindWorkflowEndpointRequest,
        caller_user_id: &str,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        self.ensure_workflow_revision(
            &request.session_id,
            &request.workflow_ref,
            request.expected_workflow_revision,
        )?;
        self.ensure_workflow_endpoint_owner(
            &request.session_id,
            &request.workflow_ref,
            &request.endpoint_ref,
            caller_user_id,
            "bind workflow endpoint",
        )?;
        self.ensure_workflow_node_editor(
            &request.session_id,
            &request.workflow_ref,
            &request.entry_node_id,
            caller_user_id,
            "bind workflow endpoint",
        )?;
        let endpoint = self.session_store.write().bind_workflow_endpoint(
            &request.session_id,
            &request.workflow_ref,
            &request.endpoint_ref,
            &request.entry_node_id,
        )?;
        let workflow = self
            .session_store
            .read()
            .resolve_workflow_ref(&request.session_id, &request.workflow_ref)?;
        let session = self.workflow_session(&request.session_id)?;
        Ok(LocalDaemonResponse::WorkflowEndpointBound {
            endpoint,
            workflow,
            session,
        })
    }
}
