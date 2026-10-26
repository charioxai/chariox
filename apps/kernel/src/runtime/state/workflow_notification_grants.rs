//! MP-08/MP-10/MP-11: a notification subscription attached under an external
//! access grant never outlives it. Revoke, expiry, holder exit and kernel
//! restart retire it; delivery re-checks the grant before any sweep runs.
use super::*;
use crate::durable_state::workflow_notifications::{NotificationOperation, NotificationOutcome};
use crate::local::WorkflowNotificationSubscription;

impl KernelRuntimeState {
    /// The access grant behind the current command. Sudo turns act for the owner.
    pub(super) fn notification_grant(&self) -> Option<String> {
        self.external_command_authority
            .as_ref()
            .map(|authority| authority.grant_id.clone())
            .filter(|id| !id.starts_with("sudo:"))
    }

    /// A grant that ended while attaching retires what it just attached.
    pub(super) fn attached_within_grant(
        &self,
        sub: WorkflowNotificationSubscription,
    ) -> Result<WorkflowNotificationSubscription, DaemonError> {
        let Some(grant) = self.notification_grant() else {
            return Ok(sub);
        };
        if let Err(error) = self.authorize_current_external_command() {
            self.retire_notification_grants(Some(&grant));
            return Err(error);
        }
        Ok(sub)
    }

    /// `None` retires every grant-bound subscription (kernel start).
    pub(super) fn retire_notification_grants(&self, grant: Option<&str>) {
        let retire = NotificationOperation::RetireGrants {
            grant: grant.map(str::to_owned),
        };
        let retired = match self.owned.durable_state_store.notify(retire) {
            Ok(NotificationOutcome::Retired(retired)) => retired,
            Ok(_) => return,
            Err(error) => {
                // Delivery still re-checks the grant; this row cannot deliver.
                crate::logging::warn_with_fields(
                    "workflow.notifications",
                    "grant-bound notification retirement failed",
                    serde_json::json!({ "grant_id": grant, "error": error.to_string() }),
                );
                return;
            }
        };
        let Ok(handle) = tokio::runtime::Handle::try_current() else {
            return;
        };
        // Best effort: the source kernel otherwise retries until the occurrence TTL.
        for sub in retired
            .into_iter()
            .filter(|sub| sub.source_kernel_id != sub.target_kernel_id)
        {
            let runtime = self.clone();
            handle.spawn(async move {
                let _ = runtime
                    .notification_peer(
                        &sub.owner_user_id,
                        &sub.source_kernel_id,
                        RelayPeerRequest::UnsubscribeWorkflowNotifications {
                            protocol_version: super::workflow_notification_peers::VERSION,
                            subscription_id: sub.subscription_id,
                        },
                    )
                    .await;
            });
        }
    }
}

impl KernelRuntimeOwnedState {
    pub(super) fn notification_grant_live(&self, sub: &WorkflowNotificationSubscription) -> bool {
        self.with_live_notification_grant(sub, |require_live| require_live())
            .is_ok()
    }

    /// Keep grant retirement serialized with admission through the durable commit.
    pub(super) fn with_live_notification_grant<T>(
        &self,
        sub: &WorkflowNotificationSubscription,
        admit: impl FnOnce(&dyn Fn() -> Result<(), DaemonError>) -> Result<T, DaemonError>,
    ) -> Result<T, DaemonError> {
        let Some(id) = self
            .durable_state_store
            .notification_grant(&sub.owner_user_id, &sub.subscription_id)?
        else {
            return admit(&|| Ok(()));
        };
        let state = self.kernel_access.lock().expect("access state poisoned");
        let require_live = || {
            if state
                .grants
                .get(&id)
                .is_some_and(|grant| Instant::now() < grant.deadline && grant.holder.alive())
            {
                Ok(())
            } else {
                Err(crate::durable_state::workflow_notifications::error(
                    "notification grant revoked or expired",
                ))
            }
        };
        require_live()?;
        let result = admit(&require_live);
        drop(state);
        result
    }
}
