//! Self-hosted mode: the kernels are the kernels this gateway paired with, and
//! one is online when it answers a status request over its relay link. This
//! needs no inventory permission on the relay, so a scoped relay works with the
//! CLIENT token given at pairing alone.
use std::sync::atomic::Ordering;
use std::time::Duration;

use chariox_relay::protocol::RelayKernelPresence;
use futures_util::future::join_all;
use serde_json::Value;

use super::directory::is_gateway_ready;
use super::error::GatewayError;
use super::link::{Link, LinkFailure};
use super::names;
use super::paired::PairedKernel;
use super::{map_failure, KernelGateway, REQUEST_COUNTER};
use crate::local::{
    GatewayKernel, GatewayKernelStatus, GatewayKernelVia, LocalDaemonRequest, RelayStatusRequest,
};

const PROBE_DEADLINE: Duration = Duration::from_secs(5);

/// What a paired kernel says about itself in its status answer.
pub(super) struct TargetStatus {
    pub(super) capabilities: Vec<String>,
    pub(super) kernel_alias: Option<String>,
    pub(super) machine_alias: Option<String>,
}

impl TargetStatus {
    /// Takes the names the kernel gives itself: the pairing link only carried its ids.
    pub(super) fn apply_to(self, presence: &mut RelayKernelPresence) {
        presence.capabilities = self.capabilities;
        presence.kernel_alias = self.kernel_alias.or(presence.kernel_alias.take());
        presence.machine_alias = self.machine_alias.or(presence.machine_alias.take());
    }
}

/// The status of a kernel (capabilities such as `kernel_gateway_v1`, and its aliases).
pub(super) async fn status_of(link: &Link) -> Result<TargetStatus, LinkFailure> {
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
    let answer = serde_json::from_slice::<Value>(&answer).unwrap_or(Value::Null);
    let status = |key: &str| {
        answer
            .pointer(&format!("/RelayStatus/status/{key}"))
            .cloned()
    };
    let text = |key: &str| {
        status(key)
            .and_then(|value| value.as_str().map(str::to_owned))
            .filter(|value| !value.trim().is_empty())
    };
    Ok(TargetStatus {
        capabilities: status("capabilities")
            .and_then(|value| serde_json::from_value(value).ok())
            .unwrap_or_default(),
        kernel_alias: text("daemon_alias"),
        machine_alias: text("machine_alias"),
    })
}

/// The one paired kernel `name` means; unknown and ambiguous are errors.
fn named(paired: &[PairedKernel], name: &str) -> Result<RelayKernelPresence, GatewayError> {
    let known: Vec<&RelayKernelPresence> = paired.iter().map(|target| &target.presence).collect();
    names::find(&known, name).cloned()
}

impl KernelGateway {
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

    pub(crate) async fn kernels(&self) -> Result<Vec<GatewayKernel>, GatewayError> {
        if !self.is_self_hosted() {
            return self.directory.kernels().await;
        }
        let probed = join_all(
            self.paired
                .all()?
                .into_iter()
                .map(|target| self.probe(target)),
        )
        .await;
        let known: Vec<&RelayKernelPresence> =
            probed.iter().map(|probed| &probed.presence).collect();
        Ok(names::grouped(
            probed
                .iter()
                .map(|probed| GatewayKernel {
                    name: names::display_name(&probed.presence, &known),
                    label: names::label(&probed.presence),
                    kernel_id: probed.presence.kernel_id.clone(),
                    machine_id: probed.presence.machine_id.clone(),
                    machine_alias: probed.presence.machine_alias.clone(),
                    status: probed.status,
                    last_seen_ms: probed.last_seen_ms,
                    gateway_ready: is_gateway_ready(&probed.presence),
                    via: GatewayKernelVia::SelfHosted,
                })
                .collect(),
        ))
    }

    /// Asks one paired kernel for its status over its own link and keeps what it says.
    async fn probe(&self, target: PairedKernel) -> Probed {
        let mut presence = target.presence;
        let answer = match self.link_for(&presence.kernel_id, &presence).await {
            Ok(link) => status_of(&link)
                .await
                .map_err(|failure| map_failure(&presence.kernel_id, failure)),
            Err(error) => Err(error),
        };
        let status = status_after(&answer);
        if let Ok(answered) = answer {
            self.last_answer
                .lock()
                .expect("last answer poisoned")
                .insert(presence.kernel_id.clone(), crate::session::unix_epoch_ms());
            let before = presence.clone();
            answered.apply_to(&mut presence);
            if presence != before {
                // The pairing stays valid whether or not the update could be stored.
                let _ = self.paired.save(PairedKernel {
                    presence: presence.clone(),
                    relay_token: target.relay_token,
                    paired_at_ms: target.paired_at_ms,
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
        Probed {
            presence,
            status,
            last_seen_ms,
        }
    }
}

struct Probed {
    presence: RelayKernelPresence,
    status: GatewayKernelStatus,
    last_seen_ms: u64,
}

/// A kernel that answered is online; one that refused this gateway is unpaired, not offline.
fn status_after(answer: &Result<TargetStatus, GatewayError>) -> GatewayKernelStatus {
    match answer {
        Ok(_) => GatewayKernelStatus::Online,
        Err(error) if error.code() == "kernel_unpaired" => GatewayKernelStatus::Unpaired,
        Err(_) => GatewayKernelStatus::Offline,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn paired(kernel_id: &str, alias: Option<&str>, machine_alias: Option<&str>) -> PairedKernel {
        PairedKernel {
            presence: RelayKernelPresence {
                kernel_id: kernel_id.into(),
                machine_id: format!("machine-{}", machine_alias.unwrap_or(kernel_id)),
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
        let kernels = vec![
            paired("k-b3", Some("b3"), Some("ovh")),
            paired("k-x", None, Some("shared")),
            paired("k-y", Some("y"), Some("shared")),
        ];
        assert_eq!(named(&kernels, "b3").unwrap().kernel_id, "k-b3");
        assert_eq!(named(&kernels, "k-x").unwrap().kernel_id, "k-x");
        assert_eq!(named(&kernels, "shared/y").unwrap().kernel_id, "k-y");
        assert_eq!(
            named(&kernels, "nope").unwrap_err().code(),
            "kernel_unknown"
        );
        let ambiguous = named(&kernels, "shared").unwrap_err();
        assert_eq!(ambiguous.code(), "kernel_ambiguous");
        assert!(ambiguous.message().contains("k-x") && ambiguous.message().contains("k-y"));
    }

    #[test]
    fn a_kernel_that_refused_the_gateway_is_unpaired_not_offline() {
        let name = "b3".to_string();
        let answered = Ok(TargetStatus {
            capabilities: Vec::new(),
            kernel_alias: None,
            machine_alias: None,
        });
        assert_eq!(status_after(&answered), GatewayKernelStatus::Online);
        assert_eq!(
            status_after(&Err(GatewayError::Unpaired { name: name.clone() })),
            GatewayKernelStatus::Unpaired
        );
        assert_eq!(
            status_after(&Err(GatewayError::Offline { name })),
            GatewayKernelStatus::Offline
        );
    }

    // A pairing link carries ids only; the kernel's own status gives the names `--kernel` accepts.
    #[test]
    fn the_names_a_kernel_gives_itself_replace_the_ids_the_link_carried() {
        let mut presence = paired("k-x", None, None).presence;
        TargetStatus {
            capabilities: vec!["kernel_gateway_v1".into()],
            kernel_alias: Some("dev".into()),
            machine_alias: Some("b3".into()),
        }
        .apply_to(&mut presence);
        assert_eq!(presence.kernel_alias.as_deref(), Some("dev"));
        assert_eq!(presence.machine_alias.as_deref(), Some("b3"));
        assert_eq!(presence.capabilities, ["kernel_gateway_v1"]);
        // A status that names nothing keeps what was learned before.
        TargetStatus {
            capabilities: Vec::new(),
            kernel_alias: None,
            machine_alias: None,
        }
        .apply_to(&mut presence);
        assert_eq!(presence.kernel_alias.as_deref(), Some("dev"));
        assert_eq!(presence.machine_alias.as_deref(), Some("b3"));
        assert!(presence.capabilities.is_empty());
    }
}
