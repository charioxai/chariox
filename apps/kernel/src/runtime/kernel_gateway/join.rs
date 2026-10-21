//! Pairing the gateway with a self-hosted kernel through the terminal pairing
//! link that kernel made. The gateway acts as a terminal: it asks the kernel,
//! over the operator relay, to admit the gateway key, then remembers the kernel.
use std::sync::atomic::Ordering;
use std::time::Duration;

use chariox_relay::protocol::RelayKernelPresence;
use serde_json::Value;

use super::error::GatewayError;
use super::link::{Link, LinkParams};
use super::paired::PairedKernel;
use super::self_hosted::status_of;
use super::{map_failure, map_open_failure, KernelGateway, REQUEST_COUNTER};
use crate::local::{
    GatewayKernel, GatewayKernelStatus, GatewayKernelVia, JoinKernelLinkRequest,
    JoinTerminalPairingLinkRequest, LocalDaemonRequest, TerminalType,
};

/// A scoped relay's pairing link carries this instead of a transport token.
const CLIENT_TOKEN_REQUIRED: &str = "operator-client-token-required";
const JOIN_DEADLINE: Duration = Duration::from_secs(30);

fn refused(detail: impl Into<String>) -> GatewayError {
    GatewayError::JoinRefused {
        detail: detail.into(),
    }
}

impl KernelGateway {
    pub(crate) async fn join(
        &self,
        request: JoinKernelLinkRequest,
    ) -> Result<GatewayKernel, GatewayError> {
        let config = self.config.snapshot();
        if config.cloud_relay.is_some() {
            return Err(refused(
                "this kernel is enrolled in Chariox Cloud, which lists your kernels itself; pairing links are for self-hosted relays.",
            ));
        }
        let relay_url = config.relay_url.clone().ok_or_else(|| {
            refused("this kernel has no relay configured; connect it to the relay the link names first.")
        })?;
        let token = crate::runtime::invite_tokens::decode_pairing_invite_token(&request.link)
            .map_err(|error| {
                refused(format!(
                    "the link is not a terminal pairing link ({error})."
                ))
            })?;
        if token.expires_at_ms <= crate::session::unix_epoch_ms() {
            return Err(refused(
                "the link has expired; make a new one on that machine.",
            ));
        }
        if token.relay_url != relay_url {
            return Err(refused(format!(
                "the link is for the relay {}, but this kernel uses {relay_url}; connect this kernel to that relay first.",
                token.relay_url
            )));
        }
        let client_token =
            match (request.relay_token, token.relay_token.as_str()) {
                (Some(given), _) => Some(given),
                (None, CLIENT_TOKEN_REQUIRED) => return Err(refused(
                    "that relay needs its client transport token; give it with --relay-token-env.",
                )),
                (None, "") => None,
                (None, shared) => Some(shared.to_string()),
            };
        let identity = self.identity()?;
        let own_token = config.relay_token.clone();
        let link = Link::open(LinkParams {
            relay_url: relay_url.clone(),
            auth_token: client_token.clone().or(own_token).ok_or_else(|| {
                refused("this kernel has no relay token; connect it to the relay first.")
            })?,
            target_kernel_id: token.target_daemon_id.clone(),
            // The relay announces the key at first contact; it is pinned from here on.
            target_public_key: None,
            client_private_key: identity.private_key.clone(),
        })
        .await
        .map_err(|failure| map_open_failure(&token.target_daemon_id, failure))?;
        let join = LocalDaemonRequest::JoinTerminalPairingLink(JoinTerminalPairingLinkRequest {
            pairing_link: request.link.clone(),
            terminal_id: None,
            terminal_type: Some(TerminalType::Cli),
            alias: Some("agent-gateway".into()),
            public_key_thumbprint: Some(identity.thumbprint.clone()),
        });
        let plaintext =
            serde_json::to_vec(&serde_json::json!({"request": join})).expect("join serializes");
        let request_id = format!(
            "gw-join-{}-{}",
            crate::session::unix_epoch_ms(),
            REQUEST_COUNTER.fetch_add(1, Ordering::Relaxed)
        );
        let answer = link
            .request(request_id, &plaintext, JOIN_DEADLINE)
            .await
            .map_err(|failure| map_failure(&token.target_daemon_id, failure))?;
        let answer: Value = serde_json::from_slice(&answer)
            .map_err(|error| refused(format!("the kernel's answer was unreadable ({error}).")))?;
        let admitted = answer
            .pointer("/TerminalPairingLinkJoined/kernel_pairing")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        if !admitted {
            return Err(refused(
                "the kernel did not admit this kernel's key; it may run an older Chariox kernel. Update it and make a new link.",
            ));
        }
        let mut presence = RelayKernelPresence {
            kernel_id: token.target_daemon_id.clone(),
            machine_id: token.issuer_machine_id.clone(),
            machine_alias: None,
            relay_alias: None,
            kernel_alias: token.target_daemon_alias.clone(),
            available_providers: Vec::new(),
            provider_accounts: Vec::new(),
            capabilities: Vec::new(),
            accepting_remote_leases: false,
            leased_agent_count: 0,
            local_session_count: 0,
            public_key: link.target_public_key().to_string(),
        };
        if let Ok(status) = status_of(&link).await {
            status.apply_to(&mut presence);
        }
        let mut known: Vec<RelayKernelPresence> = self
            .paired
            .all()?
            .into_iter()
            .map(|target| target.presence)
            .filter(|other| other.kernel_id != presence.kernel_id)
            .collect();
        known.push(presence.clone());
        let known: Vec<&RelayKernelPresence> = known.iter().collect();
        let target = GatewayKernel {
            name: super::names::display_name(&presence, &known),
            label: super::names::label(&presence),
            kernel_id: presence.kernel_id.clone(),
            machine_id: presence.machine_id.clone(),
            machine_alias: presence.machine_alias.clone(),
            status: GatewayKernelStatus::Online,
            last_seen_ms: crate::session::unix_epoch_ms(),
            gateway_ready: super::directory::is_gateway_ready(&presence),
            via: GatewayKernelVia::SelfHosted,
        };
        self.paired.save(PairedKernel {
            presence,
            relay_token: client_token,
            paired_at_ms: crate::session::unix_epoch_ms(),
        })?;
        Ok(target)
    }

    /// Drops a paired kernel; the kernel keeps its grant until revoked there.
    pub(crate) fn forget(&self, name: &str) -> Result<String, GatewayError> {
        let paired = self.paired.all()?;
        let known: Vec<&RelayKernelPresence> =
            paired.iter().map(|target| &target.presence).collect();
        let id = super::names::find(&known, name)?.kernel_id.clone();
        self.paired.remove(&id)?;
        Ok(id)
    }
}
