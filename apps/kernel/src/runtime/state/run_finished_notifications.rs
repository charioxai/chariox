//! Every workflow of a Room owner has a "run finished" source on the ordinary notification
//! router: one occurrence per run that completed or failed.
use super::*;
use crate::durable_state::workflow_notifications::{
    naming, run_finished::RUN_FINISHED_NODE, NotificationOperation, NotificationOutcome,
    SourceAdmission,
};

impl KernelRuntimeOwnedState {
    pub(super) fn ensure_run_finished_notification_source(
        &self,
        session: &crate::session::RuntimeSession,
        workflow: &crate::session::WorkflowDefinition,
    ) -> Result<crate::local::WorkflowNotificationSource, DaemonError> {
        use sha2::{Digest, Sha256};
        let name = naming::workflow_name(workflow);
        let source = crate::local::WorkflowNotificationSource {
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
        };
        match self
            .durable_state_store
            .notify(NotificationOperation::Register(SourceAdmission {
                source,
                workflow_json: String::new(),
            }))? {
            NotificationOutcome::Source(source) => Ok(source),
            _ => Err(crate::durable_state::workflow_notifications::error(
                "unexpected notification registration result",
            )),
        }
    }
}
