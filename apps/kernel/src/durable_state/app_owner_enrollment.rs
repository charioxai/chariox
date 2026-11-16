//! Cloud enrollment hands the App records of the local owner to the enrolled
//! owner in one transaction on the sole writer, so an App set up before
//! enrollment keeps its installations, automations, routes and queued events.
use super::{DurableKernelStateStore, DurableWriterRequest};
use crate::error::DaemonError;
use rusqlite::{Connection, TransactionBehavior};
use std::sync::mpsc;

/// Every table that keys App records by `owner_id`. A new App table with an
/// owner column is classified here or in the enrollment test.
pub(crate) const APP_OWNER_TABLES: &[&str] = &[
    "app_automations",
    "app_connection_grants",
    "app_deployment_consents",
    "app_dev_identities",
    "app_file_exports",
    "app_file_grants",
    "app_file_picks",
    "app_host_actions",
    "app_inbox",
    "app_inbox_routes",
    "app_installation_deployments",
    "app_installation_operations",
    "app_installation_stage_trust",
    "app_installations",
    "app_logs",
    "app_outbox",
    "app_outbox_replay_floors",
    "app_publisher_decisions",
    "app_publisher_keys",
    "app_publisher_operations",
    "app_release_versions",
    "app_restore_receipts",
    "app_validations",
    "app_wakes",
    "app_worker_lifecycle",
];

pub(super) struct AppOwnerEnrollmentRequest {
    old: String,
    new: String,
    response: mpsc::Sender<rusqlite::Result<()>>,
}
impl std::fmt::Debug for AppOwnerEnrollmentRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("AppOwnerEnrollmentRequest(..)")
    }
}

impl DurableKernelStateStore {
    /// Re-owns every App record of `old` to `new`, all or nothing. Idempotent:
    /// a second pass finds nothing owned by `old`.
    pub(crate) fn enroll_app_owner(&self, old: &str, new: &str) -> Result<(), DaemonError> {
        let (response, receiver) = mpsc::channel();
        self.writer
            .enqueue(DurableWriterRequest::AppOwnerEnrollment(Box::new(
                AppOwnerEnrollmentRequest {
                    old: old.into(),
                    new: new.into(),
                    response,
                },
            )))?;
        crate::durable_state::await_writer_response(receiver)
            .map_err(|error| DaemonError::LocalTransport {
                operation: "app.owner.enroll",
                message: error.to_string(),
            })?
            .map_err(|error| DaemonError::LocalTransport {
                operation: "app.owner.enroll",
                message: error.to_string(),
            })
    }
}

pub(super) fn execute(connection: &mut Connection, request: AppOwnerEnrollmentRequest) {
    let result = (|| {
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        for table in APP_OWNER_TABLES {
            // A record the enrolled owner already holds under the same key wins;
            // the local one stays under `local` rather than being overwritten.
            transaction.execute(
                &format!("UPDATE OR IGNORE {table} SET owner_id=?1 WHERE owner_id=?2"),
                [&request.new, &request.old],
            )?;
        }
        transaction.commit()
    })();
    let _ = request.response.send(result);
}
