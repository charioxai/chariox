//! Workflow turn acknowledgement and output validation request handlers.

use super::*;

impl KernelRuntimeOwnedState {
    pub(super) fn workflow_validate_handoff(
        &self,
        request: crate::local::ValidateWorkflowHandoffRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let warning = crate::transport::runtime_tools::validate_workflow_handoff_schema(
            &request.handoff_schema_ref,
            &request.handoff_json,
        )
        .err();
        Ok(LocalDaemonResponse::WorkflowHandoffValidated {
            valid: warning.is_none(),
            warning,
        })
    }

    pub(super) fn workflow_ack_turn(
        &self,
        request: crate::local::AckWorkflowTurnRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let workflow_run_id = self
            .session_store
            .read()
            .resolve_workflow_run_ref(&request.session_id, &request.workflow_run_ref)?
            .id()
            .to_string();
        let workflow_run = self.session_store.write().ack_workflow_turn(
            &request.session_id,
            &workflow_run_id,
            &request.workflow_node_run_id,
            &request.delivery_token,
        )?;
        let event = crate::session::WorkflowRuntimeToolCallEvent::new(
            crate::transport::runtime_tools::ACK_WORKFLOW_TURN_TOOL.to_string(),
            serde_json::json!({"delivery_token": request.delivery_token}).to_string(),
            Some(
                serde_json::json!({
                    "workflow_run_id": workflow_run.id(),
                    "workflow_node_run_id": request.workflow_node_run_id,
                    "state": "acknowledged",
                    "next_action": "Continue this same workflow turn. This acknowledgement is not the final answer. If this turn requires final workflow run output, call validate_and_submit_workflow_run_output before stopping; otherwise emit the required final fenced json block before stopping.",
                })
                .to_string(),
            ),
            true,
        );
        let _ = self
            .session_store
            .write()
            .record_workflow_runtime_tool_call(
                &request.session_id,
                &request.workflow_node_run_id,
                event,
            );
        let workflow_run = self
            .session_store
            .read()
            .resolve_workflow_run_ref(&request.session_id, &workflow_run_id)?;
        let session = self.workflow_session(&request.session_id)?;
        Ok(LocalDaemonResponse::WorkflowTurnAcknowledged {
            workflow_run,
            session,
        })
    }
}
