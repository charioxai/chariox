//! MP-08 / MP-10: one owned terminal stream for local and relay subscribers.
use super::*;
use crate::runtime::projection::SessionSnapshotProjection;
use crate::runtime_transport::WatchResult;

impl KernelRuntimeState {
    pub(crate) async fn watch_relay_subscription_state(
        &self,
        session_id: &str,
        attachment_id: &str,
        should_check_snapshot: bool,
        previous_snapshot: Option<SessionSnapshotProjection>,
        last_workflow_design_sequence: u64,
    ) -> WatchResult {
        // MP-08 / MP-10: provider I/O belongs to the readiness pump. Client
        // subscribers only drain the same shared stores used by that owner.
        if self
            .owned
            .ensure_attachment_in_session(session_id, attachment_id)
            .is_err()
        {
            return WatchResult::Unavailable("Current session is no longer available.".to_string());
        }
        let previous_snapshot_for_compare = previous_snapshot;
        let terminal = &self.owned.terminal_stream;
        let mut result = WatchResult::Ok {
            records: terminal.drain_output_records(session_id, attachment_id),
            notices: terminal.drain_notice_records(session_id, attachment_id),
            completions: terminal.drain_completion_records(session_id, attachment_id),
            workflow_run_updates: terminal.drain_workflow_run_updates(session_id, attachment_id),
            workflow_design_events: self.owned.workflow_design_events.events_since(
                session_id,
                last_workflow_design_sequence,
                attachment_id,
            ),
            snapshot: Box::new(None),
        };
        if !should_check_snapshot {
            return result;
        }
        let projected_snapshot = match self.session_snapshot_projection_for_attachment(
            session_id,
            attachment_id,
            self.session_projection_change_sequence(),
        ) {
            Ok(mut snapshot) => {
                snapshot.metadata.last_event_id = self.session_projection_change_sequence();
                Box::new(
                    (!previous_snapshot_for_compare
                        .as_ref()
                        .is_some_and(|previous| same_snapshot_data(previous, &snapshot)))
                    .then_some(snapshot),
                )
            }
            Err(DaemonError::SessionNotFound { .. })
            | Err(DaemonError::AttachmentNotFound { .. })
            | Err(DaemonError::AttachmentNotInSession { .. }) => {
                return WatchResult::Unavailable(
                    "Current session is no longer available.".to_string(),
                );
            }
            Err(error) => {
                crate::logging::warn_with_fields(
                    "daemon.runtime_transport",
                    "kernel event loop failed to build owned session snapshot",
                    serde_json::json!({
                        "session_id": session_id,
                        "attachment_id": attachment_id,
                        "error": error.to_string(),
                    }),
                );
                Box::new(None)
            }
        };
        if let WatchResult::Ok { snapshot, .. } = &mut result {
            *snapshot = projected_snapshot;
        }
        result
    }
}

fn same_snapshot_data(a: &SessionSnapshotProjection, b: &SessionSnapshotProjection) -> bool {
    // MP-08 / MP-10: generation time and the global wake cursor are not
    // session data. Preserve version/data changes while avoiding full-snapshot
    // traffic and DOM work on every reconciliation of an unchanged session.
    a.metadata.projection_version == b.metadata.projection_version
        && a.room_workflows == b.room_workflows
        && a.session == b.session
        && a.provider_run == b.provider_run
        && a.agent_activity == b.agent_activity
}
