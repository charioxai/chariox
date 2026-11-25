//! Row 582: a machine this kernel acquired waits for its owner's logins (`awaiting_context`). The
//! kernel asks the connected service for the transfer ticket and starts its own transfer. It rides
//! the read of the owner's machines that `cloud_machine_notices` already makes while a machine can
//! change (every 30 s): there is no timer of its own, and a read that missed the state sees it on
//! the next one.
use std::collections::BTreeSet;
use std::sync::{Arc, Mutex};

use crate::error::DaemonError;
use crate::local::{ControlPlaneMachineRow, ControlPlaneRequest, ControlPlaneResponse};
use crate::managed_context::outbound_service::ManagedContextTransferTicket;
use crate::runtime::state::KernelRuntimeState;

/// A kernel that advertises this starts the install of its owner's logins on a machine it acquires.
pub(crate) const AGENT_MACHINE_INSTALL_CAPABILITY: &str = "agent_machine_install_v1";

const AWAITING_CONTEXT: &str = "awaiting_context";

/// Starts the transfer a ticket names, from this kernel as its source. Handed over at spawn by the
/// router, which owns the relay state and the transfer store.
pub(crate) type InstallStarter =
    Arc<dyn Fn(ManagedContextTransferTicket) -> Result<(), DaemonError> + Send + Sync>;

/// The allocations this process already asked about while they wait.
#[derive(Default)]
pub(crate) struct InstallAttempts(Mutex<BTreeSet<String>>);

impl InstallAttempts {
    /// The allocations in `awaiting_context` not yet tried, now marked tried. A machine that left
    /// the state is forgotten, so a machine restarted into it is tried again.
    pub(crate) fn due(&self, machines: &[ControlPlaneMachineRow]) -> Vec<String> {
        let mut tried = self
            .0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let waiting: BTreeSet<&str> = machines
            .iter()
            .filter(|row| row.state == AWAITING_CONTEXT)
            .map(|row| row.allocation_ref.as_str())
            .collect();
        tried.retain(|allocation| waiting.contains(allocation.as_str()));
        waiting
            .into_iter()
            .filter(|allocation| tried.insert((*allocation).to_string()))
            .map(str::to_string)
            .collect()
    }

    /// A try that failed for a reason the next read may not share: ask again then.
    pub(crate) fn forget(&self, allocation_ref: &str) {
        self.0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .remove(allocation_ref);
    }
}

/// The service answers only the kernel that acquired the machine: any other kernel is told once
/// that the allocation is not found, and is not asked again until the machine leaves the state.
pub(crate) fn refusal_is_final(error: &DaemonError) -> bool {
    error.to_string().contains("not found")
}

impl KernelRuntimeState {
    /// Called with every read of the owner's machines.
    pub(crate) async fn install_awaiting_machines(
        &self,
        machines: &[ControlPlaneMachineRow],
        api_url: &str,
        account_id: &str,
        credential: &str,
    ) {
        let Some(starter) = self.agent_install_starter() else {
            return;
        };
        for allocation_ref in self.agent_install_attempts().due(machines) {
            match fetch_ticket(api_url, account_id, credential, &allocation_ref).await {
                Ok(ticket) => {
                    if let Err(error) = starter(ticket) {
                        crate::logging::warn_with_fields(
                            "cloud.agent_machine_install",
                            "could not start the transfer of the owner's logins; trying again at the next read",
                            serde_json::json!({ "allocation_ref": allocation_ref, "error": error.to_string() }),
                        );
                        self.agent_install_attempts().forget(&allocation_ref);
                    }
                }
                Err(error) if refusal_is_final(&error) => {}
                Err(error) => {
                    crate::logging::warn_with_fields(
                        "cloud.agent_machine_install",
                        "could not read the transfer ticket; trying again at the next read",
                        serde_json::json!({ "allocation_ref": allocation_ref, "error": error.to_string() }),
                    );
                    self.agent_install_attempts().forget(&allocation_ref);
                }
            }
        }
    }
}

async fn fetch_ticket(
    api_url: &str,
    account_id: &str,
    credential: &str,
    allocation_ref: &str,
) -> Result<ManagedContextTransferTicket, DaemonError> {
    let response = crate::runtime::cloud_api_client::post_cloud_json::<ControlPlaneResponse>(
        api_url.to_string(),
        "/control-plane",
        serde_json::json!({
            "accountId": account_id,
            "kernelCredential": credential,
            "request": ControlPlaneRequest::ContextTransfer { allocation_ref: allocation_ref.to_string() },
        }),
    )
    .await?;
    match response {
        ControlPlaneResponse::ContextTransfer { ticket } => Ok(*ticket),
        _ => Err(DaemonError::LocalTransport {
            operation: "agent_machine_install",
            message: "The connected service answered the transfer ticket with another response"
                .into(),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(allocation: &str, state: &str) -> ControlPlaneMachineRow {
        ControlPlaneMachineRow {
            allocation_ref: allocation.into(),
            name: allocation.into(),
            state: state.into(),
            revision: 1,
            since: "2026-11-25T00:00:00Z".into(),
            kernel_id: None,
            machine_id: None,
            available_actions: Vec::new(),
            reason: None,
        }
    }

    #[test]
    fn only_machines_waiting_for_their_context_are_due_and_each_is_due_once() {
        let attempts = InstallAttempts::default();
        let machines = [
            row("a", "awaiting_context"),
            row("b", "ready"),
            row("c", "provisioning"),
            row("d", "awaiting_context"),
        ];
        assert_eq!(attempts.due(&machines), ["a", "d"]);
        assert!(
            attempts.due(&machines).is_empty(),
            "the next read asks nothing again"
        );
    }

    #[test]
    fn a_machine_that_left_the_state_is_forgotten_and_tried_again_when_it_returns() {
        let attempts = InstallAttempts::default();
        assert_eq!(attempts.due(&[row("a", "awaiting_context")]), ["a"]);
        assert!(attempts.due(&[row("a", "ready")]).is_empty());
        assert_eq!(attempts.due(&[row("a", "awaiting_context")]), ["a"]);
    }

    #[test]
    fn a_failed_try_is_asked_again_at_the_next_read() {
        let attempts = InstallAttempts::default();
        let machines = [row("a", "awaiting_context")];
        assert_eq!(attempts.due(&machines), ["a"]);
        attempts.forget("a");
        assert_eq!(attempts.due(&machines), ["a"]);
    }

    #[test]
    fn a_machine_the_service_says_is_not_ours_is_final() {
        let refused = DaemonError::LocalTransport {
            operation: "cloud",
            message: "Machine allocation not found".into(),
        };
        let down = DaemonError::LocalTransport {
            operation: "cloud",
            message: "connection refused".into(),
        };
        assert!(refusal_is_final(&refused));
        assert!(!refusal_is_final(&down));
    }

    #[test]
    fn the_context_transfer_request_has_the_wire_shape_the_service_reads() {
        assert_eq!(
            serde_json::to_value(ControlPlaneRequest::ContextTransfer {
                allocation_ref: "env-1".into()
            })
            .unwrap(),
            serde_json::json!({"action": "context_transfer", "allocation_ref": "env-1"})
        );
    }
}
