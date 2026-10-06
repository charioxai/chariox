//! Captured-run controls. Trigger admission remains owned by existing services.
use super::*;
use crate::local::{
    ControlRoomWorkflowRunsRequest, RoomWorkflowRunAction, RoomWorkflowRunControlOutcome,
    RoomWorkflowRunControlResult,
};

impl KernelRuntimeState {
    pub(super) async fn execute_room_workflow_control_request(
        &self,
        request: ControlRoomWorkflowRunsRequest,
        caller_user_id: &str,
    ) -> (
        Result<LocalDaemonResponse, DaemonError>,
        Option<crate::session::RuntimeSession>,
    ) {
        if let Err(error) = self.authorize_current_external_command() {
            return (Err(error), None);
        }
        let initial = match self.owned.session_snapshot(&request.session_id) {
            Ok(session) => session,
            Err(error) => return (Err(error), None),
        };
        if initial.workflow(&request.workflow_id).is_none() || request.run_ids.len() > 1024 {
            return (
                Err(DaemonError::LocalTransport {
                    operation: "control room workflow runs",
                    message: "workflow is not in this room or run selection is too large".into(),
                }),
                None,
            );
        }
        let mut results = Vec::new();
        let mut seen = std::collections::BTreeSet::new();
        for run_id in &request.run_ids {
            if !seen.insert(run_id) {
                continue;
            }
            // Resolve exact ids, never aliases/latest. Archive lookup distinguishes terminal
            // since-click runs from invalid selections without retargeting new admissions.
            let run = self
                .owned
                .session_store
                .get_session(&request.session_id)
                .ok()
                .and_then(|session| {
                    session
                        .workflow_runs()
                        .iter()
                        .find(|run| run.id() == run_id)
                        .cloned()
                })
                .or_else(|| {
                    self.owned
                        .durable_state_store
                        .resolve_workflow_run(initial.host_daemon_id(), initial.id(), run_id)
                        .ok()
                        .flatten()
                });
            let mut result = RoomWorkflowRunControlResult {
                run_id: run_id.clone(),
                outcome: RoomWorkflowRunControlOutcome::Failed,
                error: None,
            };
            match run {
                Some(run) if run.id() == run_id && run.workflow_id() == request.workflow_id => {
                    if run.status().is_terminal()
                        || (request.action == RoomWorkflowRunAction::Pause
                            && run.status() == crate::session::WorkflowRunStatus::Paused)
                    {
                        result.outcome = RoomWorkflowRunControlOutcome::Unchanged;
                    } else {
                        let outcome = match request.action {
                            RoomWorkflowRunAction::Pause => self
                                .execute_workflow_interrupt_run(&request.session_id, run_id, true)
                                .await
                                .map(|_| ()),
                            RoomWorkflowRunAction::Stop => self
                                .execute_workflow_interrupt_run(&request.session_id, run_id, false)
                                .await
                                .map(|_| ()),
                            RoomWorkflowRunAction::Resume => self
                                .execute_workflow_resume_run_request(
                                    crate::local::ResumeWorkflowRunRequest {
                                        session_id: request.session_id.clone(),
                                        workflow_run_ref: run_id.clone(),
                                    },
                                )
                                .await
                                .0
                                .map(|_| ()),
                        };
                        match outcome {
                            Ok(()) => result.outcome = RoomWorkflowRunControlOutcome::Applied,
                            Err(DaemonError::InvalidWorkflowRunState { status, .. })
                                if status.is_terminal() =>
                            {
                                result.outcome = RoomWorkflowRunControlOutcome::Unchanged;
                            }
                            Err(error) => result.error = Some(error.to_string()),
                        }
                    }
                }
                _ => result.error = Some("Run does not belong to this room and workflow".into()),
            }
            results.push(result);
        }
        match self.owned.session_snapshot(&request.session_id) {
            Ok(session) => {
                let inventory = crate::runtime::projection::RoomWorkflowInventory::project(
                    &session,
                    caller_user_id,
                );
                (
                    Ok(LocalDaemonResponse::RoomWorkflowRunsControlled { results, inventory }),
                    Some(session),
                )
            }
            Err(error) => (Err(error), None),
        }
    }
}
