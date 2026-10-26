//! MP-08 / MP-10 / MP-11: terminal updates use the same trace fence as snapshots.
use crate::app::{DaemonApp, KernelSessionReadService};
use crate::error::DaemonError;
use crate::session::WorkflowRun;

pub(super) fn drain_visible_workflow_run_updates(
    app: &DaemonApp,
    session_id: &str,
    attachment_id: &str,
    owner_user_id: &str,
) -> Result<Vec<WorkflowRun>, DaemonError> {
    let updates = app
        .terminal()
        .drain_workflow_run_updates(session_id, attachment_id);
    if updates.is_empty() {
        return Ok(updates);
    }
    let session = KernelSessionReadService::new(app).session_snapshot(session_id)?;
    let history = app.durable_state_store();
    updates
        .into_iter()
        .filter_map(|run| {
            match session.project_workflow_run_for_user(run, owner_user_id, Some(&history)) {
                Ok(Some(run)) => Some(Ok(run)),
                Ok(None) => None,
                Err(error) => Some(Err(error)),
            }
        })
        .collect()
}
