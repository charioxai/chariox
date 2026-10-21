//! Self-hosted mode: the machines are the kernels this gateway paired with, and
//! one is online when it answers a status request over its relay link. This
//! needs no inventory permission on the relay, so a scoped relay works with the
//! CLIENT token given at pairing alone.
use std::sync::atomic::Ordering;
use std::time::Duration;

use chariox_relay::protocol::RelayKernelPresence;
use futures_util::future::join_all;
use serde_json::Value;

use super::directory::{display_name, is_gateway_ready, names_of};
use super::error::GatewayError;
use super::link::{Link, LinkFailure};
use super::paired::PairedMachine;
use super::{MachineGateway, REQUEST_COUNTER};
use crate::local::{
    GatewayMachine, GatewayMachineStatus, GatewayMachineVia, LocalDaemonRequest, RelayStatusRequest,
};

const PROBE_DEADLINE: Duration = Duration::from_secs(5);

/// What a kernel advertises (for example `machine_gateway_v1`), from its status answer.
pub(super) async fn capabilities_of(link: &Link) -> Result<Vec<String>, LinkFailure> {
    let request = serde_json::to_vec(
        &serde_json::json!({"request": LocalDaemonRequest::RelayStatus(RelayStatusRequest)}),
    )
    .expect("relay status serializes");
    let request_id = format!(
        "gw-status-{}-{}",
        crate::session::unix_epoch_ms(),
        REQUEST_COUNTER.fetch_add(1, Ordering::Relaxed)
    );
    let answer = link.request(request_id, &request, PROBE_DEADLINE).await?;
    Ok(serde_json::from_slice::<Value>(&answer)
        .ok()
        .and_then(|value| value.pointer("/RelayStatus/status/capabilities").cloned())
        .and_then(|capabilities| serde_json::from_value(capabilities).ok())
        .unwrap_or_default())
}

/// The one paired kernel `name` means; unknown and ambiguous are errors.
fn named(paired: &[PairedMachine], name: &str) -> Result<RelayKernelPresence, GatewayError> {
    let matches: Vec<_> = paired
        .iter()
        .filter(|machine| names_of(&machine.presence).iter().any(|n| n == name))
        .collect();
    match matches.as_slice() {
        [] => Err(GatewayError::Unknown { name: name.into() }),
        [one] => Ok(one.presence.clone()),
        several => Err(GatewayError::Ambiguous {
            name: name.into(),
            candidates: several
                .iter()
                .map(|machine| machine.presence.kernel_id.clone())
                .collect(),
        }),
    }
}

impl MachineGateway {
    pub(super) fn is_self_hosted(&self) -> bool {
        self.config.snapshot().cloud_relay.is_none()
    }

    /// The one kernel `name` means; unknown and ambiguous are errors. Whether it is
    /// online is found out by talking to it, with a definitive code when it is not.
    pub(super) async fn resolve(&self, name: &str) -> Result<RelayKernelPresence, GatewayError> {
        if !self.is_self_hosted() {
            return self.directory.resolve(name).await;
        }
        named(&self.paired.all()?, name)
    }

    pub(crate) async fn machines(&self) -> Result<Vec<GatewayMachine>, GatewayError> {
        if !self.is_self_hosted() {
            return self.directory.machines().await;
        }
        let paired = self.paired.all()?;
        Ok(join_all(paired.into_iter().map(|machine| self.probe(machine))).await)
    }

    async fn probe(&self, machine: PairedMachine) -> GatewayMachine {
        let mut presence = machine.presence;
        let name = display_name(&presence);
        let answered = match self.link_for(&name, &presence).await {
            Ok(link) => capabilities_of(&link).await.ok(),
            Err(_) => None,
        };
        let now = crate::session::unix_epoch_ms();
        let online = answered.is_some();
        if let Some(capabilities) = answered {
            self.last_answer
                .lock()
                .expect("last answer poisoned")
                .insert(presence.kernel_id.clone(), now);
            if capabilities != presence.capabilities {
                presence.capabilities = capabilities;
                // The pairing stays valid whether or not the update could be stored.
                let _ = self.paired.save(PairedMachine {
                    presence: presence.clone(),
                    relay_token: machine.relay_token,
                    paired_at_ms: machine.paired_at_ms,
                });
            }
        }
        let last_seen_ms = self
            .last_answer
            .lock()
            .expect("last answer poisoned")
            .get(&presence.kernel_id)
            .copied()
            .unwrap_or(0);
        GatewayMachine {
            name,
            kernel_id: presence.kernel_id.clone(),
            machine_id: presence.machine_id.clone(),
            machine_alias: presence.machine_alias.clone(),
            status: if online {
                GatewayMachineStatus::Online
            } else {
                GatewayMachineStatus::Offline
            },
            last_seen_ms,
            gateway_ready: is_gateway_ready(&presence),
            via: GatewayMachineVia::SelfHosted,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn paired(kernel_id: &str, alias: Option<&str>, machine_alias: Option<&str>) -> PairedMachine {
        PairedMachine {
            presence: RelayKernelPresence {
                kernel_id: kernel_id.into(),
                machine_id: format!("machine-{kernel_id}"),
                machine_alias: machine_alias.map(Into::into),
                relay_alias: None,
                kernel_alias: alias.map(Into::into),
                available_providers: Vec::new(),
                provider_accounts: Vec::new(),
                capabilities: Vec::new(),
                accepting_remote_leases: false,
                leased_agent_count: 0,
                local_session_count: 0,
                public_key: format!("key-{kernel_id}"),
            },
            relay_token: None,
            paired_at_ms: 1,
        }
    }

    // EA2-08: a name means one paired kernel; being online is found out by talking to it.
    #[test]
    fn a_name_means_one_paired_kernel_and_every_other_case_is_a_typed_error() {
        let machines = vec![
            paired("k-b3", Some("b3"), Some("ovh")),
            paired("k-x", None, Some("shared")),
            paired("k-y", Some("y"), Some("shared")),
        ];
        assert_eq!(named(&machines, "b3").unwrap().kernel_id, "k-b3");
        assert_eq!(named(&machines, "k-x").unwrap().kernel_id, "k-x");
        assert_eq!(
            named(&machines, "nope").unwrap_err().code(),
            "machine_unknown"
        );
        let ambiguous = named(&machines, "shared").unwrap_err();
        assert_eq!(ambiguous.code(), "machine_ambiguous");
        assert!(ambiguous.message().contains("k-x") && ambiguous.message().contains("k-y"));
    }
}
