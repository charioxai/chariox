//! Every workflow of a Room owner has a "run finished" source on the ordinary notification
//! router: one occurrence per run that completed or failed.
use crate::durable_state::workflow_notifications::{naming, run_finished::RUN_FINISHED_NODE};

/// The "run finished" source of one workflow of a Room, for the Room's current owner.
pub(super) fn run_finished_source(
    session: &crate::session::RuntimeSession,
    workflow: &crate::session::WorkflowDefinition,
) -> crate::local::WorkflowNotificationSource {
    use sha2::{Digest, Sha256};
    let name = naming::workflow_name(workflow);
    crate::local::WorkflowNotificationSource {
        kind: crate::local::NotificationSourceKind::WorkflowRun,
        source_id: format!(
            "run_finished_{:x}",
            Sha256::digest(
                serde_json::to_vec(&(
                    session.host_daemon_id(),
                    session.id(),
                    workflow.id(),
                    session.owner_user_id()
                ))
                .expect("identities serialize")
            )
        ),
        owner_user_id: session.owner_user_id().into(),
        kernel_id: session.host_daemon_id().into(),
        session_id: session.id().into(),
        workflow_id: workflow.id().into(),
        exit_node_id: RUN_FINISHED_NODE.into(),
        enabled: true,
        available: session.status() != crate::session::SessionStatus::Ended,
        display_name: naming::run_display_name(&name),
        name,
        exit_ordinal: 0,
    }
}
