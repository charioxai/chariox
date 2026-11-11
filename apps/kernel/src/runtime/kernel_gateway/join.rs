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

/// What the pairing link says about the kernel. The link's name is the computer's name at
/// pairing time, so it is kept as the machine name that the kernel's status refreshes, never
/// as a kernel name that would outlive a rename.
fn paired_presence(
    token: &crate::runtime::invite_tokens::PairingInviteToken,
    public_key: &str,
) -> RelayKernelPresence {
    RelayKernelPresence {
        kernel_id: token.target_daemon_id.clone(),
        machine_id: token.issuer_machine_id.clone(),
        machine_alias: token.target_daemon_alias.clone(),
        relay_alias: None,
        kernel_alias: None,
        available_providers: Vec::new(),
        provider_accounts: Vec::new(),
        capabilities: Vec::new(),
        accepting_remote_leases: false,
        leased_agent_count: 0,
        local_session_count: 0,
        public_key: public_key.to_string(),
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
        let mut presence = paired_presence(&token, link.target_public_key());
        if let Ok(status) = status_of(&link).await {
            status.apply_to(&mut presence);
        }
        let target = GatewayKernel {
            name: super::names::display_name(&presence),
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

#[cfg(test)]
mod tests {
    use super::super::self_hosted::TargetStatus;
    use super::*;
    use crate::runtime::invite_tokens::PairingInviteToken;

    fn link(name: &str) -> PairingInviteToken {
        PairingInviteToken {
            version: 1,
            intent: crate::local::PairingInviteIntent::Client,
            invite_id: "invite".into(),
            relay_url: "wss://relay.example/ws".into(),
            relay_token: String::new(),
            target_daemon_id: "k-1".into(),
            target_daemon_alias: Some(name.into()),
            issuer_machine_id: "machine-1".into(),
            issued_at_ms: 1,
            expires_at_ms: 2,
            terminal_type: None,
            pairing_code: None,
            terminal_id: None,
        }
    }

    #[test]
    fn pairing_keeps_the_computer_name_only_until_the_kernel_says_its_new_one() {
        let mut presence = paired_presence(&link("Old Name"), "key");
        assert_eq!(presence.machine_alias.as_deref(), Some("Old Name"));
        assert_eq!(presence.kernel_alias, None);
        TargetStatus {
            capabilities: vec!["kernel_gateway_v1".into()],
            machine_alias: Some("Studio Box".into()),
        }
        .apply_to(&mut presence);
        assert_eq!(super::super::names::display_name(&presence), "Studio Box");
    }
}
