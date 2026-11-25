//! Reads the owner's machines from the connected service while one of them can still report an
//! event, and raises each event in the person's Inbox (`person_inbox::cloud_machine`). The service
//! never pushes to an idle kernel, so the kernel reads. With nothing that can report an event the
//! loop waits for a machine request through this kernel or a change of Cloud enrollment instead of
//! reading.
use std::time::Duration;

use tokio::sync::Notify;

use crate::error::DaemonError;
use crate::local::{ControlPlaneRequest, ControlPlaneResponse};
use crate::runtime::person_inbox::cloud_machine::needs_watching;
use crate::runtime::state::KernelRuntimeState;

/// How often the owner's machines are read while one can report an event (Miguel 11-25 03:0xZ: ok).
const READ_INTERVAL: Duration = Duration::from_secs(30);
/// A failed read waits twice as long each time, up to this.
const MAX_FAILURE_DELAY: Duration = Duration::from_secs(300);

static WAKE: Notify = Notify::const_new();

/// The kernel's Cloud enrollment changed (linked, re-linked, unlinked): the owner's machines are
/// read again, or the watcher goes back to waiting when there is no enrollment.
pub(crate) fn enrollment_changed() {
    WAKE.notify_one();
}

/// A request through this kernel that can change the owner's machines. Held for the length of the
/// request; the owner's machines are read after it ends.
pub(crate) struct MachinesMayHaveChanged;

/// Reads of the machine list, a machine or a job change nothing and wake nothing.
pub(crate) fn after(request: &ControlPlaneRequest) -> Option<MachinesMayHaveChanged> {
    use ControlPlaneRequest::*;
    matches!(
        request,
        Acquire { .. }
            | AcquireJob { .. }
            | Release { .. }
            | Start { .. }
            | Stop { .. }
            | StartSavedState { .. }
            | DeleteSavedState { .. }
    )
    .then_some(MachinesMayHaveChanged)
}

impl Drop for MachinesMayHaveChanged {
    fn drop(&mut self) {
        WAKE.notify_one();
    }
}

fn failure_delay(failures: u32) -> Duration {
    READ_INTERVAL
        .saturating_mul(2u32.saturating_pow(failures.saturating_sub(1)))
        .min(MAX_FAILURE_DELAY)
}

impl KernelRuntimeState {
    pub(crate) async fn run_cloud_machine_notices(&self) {
        let mut failures = 0u32;
        loop {
            match self.read_cloud_machines().await {
                Ok(Watch::Poll) => {
                    failures = 0;
                    self.wait_or_machine_request(READ_INTERVAL).await;
                }
                Ok(Watch::Idle) => {
                    failures = 0;
                    WAKE.notified().await;
                }
                Err(error) => {
                    failures = failures.saturating_add(1);
                    if failures == 1 {
                        crate::logging::warn_with_fields(
                            "cloud.machine_notices",
                            "could not read the owner's machines; retrying with a longer wait each time",
                            serde_json::json!({ "error": error.to_string() }),
                        );
                    }
                    self.wait_or_machine_request(failure_delay(failures)).await;
                }
            }
        }
    }

    async fn wait_or_machine_request(&self, delay: Duration) {
        tokio::select! {
            _ = tokio::time::sleep(delay) => {}
            _ = WAKE.notified() => {}
        }
    }

    async fn read_cloud_machines(&self) -> Result<Watch, DaemonError> {
        let config = self.config_snapshot().await;
        let Some(profile) = crate::runtime::cloud_relay_control::active_cloud_profile(&config)
        else {
            return Ok(Watch::Idle);
        };
        let Some(credential) = profile
            .kernel_credential
            .as_deref()
            .filter(|c| !c.is_empty())
        else {
            return Ok(Watch::Idle);
        };
        let response = crate::runtime::cloud_api_client::post_cloud_json::<ControlPlaneResponse>(
            profile.api_url.clone(),
            "/control-plane",
            serde_json::json!({
                "accountId": profile.account_id,
                "kernelCredential": credential,
                "request": ControlPlaneRequest::List {},
            }),
        )
        .await?;
        let ControlPlaneResponse::Machines { machines, .. } = response else {
            return Err(DaemonError::LocalTransport {
                operation: "cloud_machine_notices",
                message: "The connected service answered the machine list with another response"
                    .into(),
            });
        };
        self.reconcile_cloud_machine_notices(&machines)?;
        Ok(
            if needs_watching(&machines, crate::session::unix_epoch_ms()) {
                Watch::Poll
            } else {
                Watch::Idle
            },
        )
    }
}

enum Watch {
    Poll,
    Idle,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn failed_reads_wait_longer_each_time_up_to_the_cap() {
        let waits: Vec<_> = (1..=6).map(|n| failure_delay(n).as_secs()).collect();
        assert_eq!(waits, [30, 60, 120, 240, 300, 300]);
    }
}
