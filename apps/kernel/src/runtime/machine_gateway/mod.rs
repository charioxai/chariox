//! The machine gateway of the LOCAL kernel (External Agents v2).
//!
//! An external agent holding a local grant names a machine on a request frame;
//! this kernel authorizes the request locally, then forwards it over the relay as
//! a terminal-class client of the target kernel, which stays the authority for
//! its own state. The agent never sees a relay token, key or Cloud credential.
mod directory;
mod error;
mod identity;
mod inventory;
mod join;
mod keeper;
pub(crate) mod ledger;
mod link;
pub(crate) mod origins;
mod paired;
mod self_hosted;
mod subscription;

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use serde_json::Value;
use tokio::sync::Mutex;

use crate::local::{LocalDaemonRequest, OnBehalfOf};
use crate::runtime::projection::{
    DaemonConfigProjectionStore, RemoteRelayInventoryProjectionStore,
};
use crate::runtime::state::KernelRuntimeState;

pub(crate) use directory::GATEWAY_CAPABILITY;
use directory::{is_gateway_ready, Directory};
pub(crate) use error::{is_public_code, GatewayError};
use identity::GatewayIdentity;
use inventory::RelayInventory;
use link::{Link, LinkFailure, LinkParams};
pub(crate) use subscription::{ForwardedSubscription, SubscribeRequest};

/// The default time a forwarded request may take before `machine_request_timeout`.
pub(crate) const REQUEST_DEADLINE: Duration = Duration::from_secs(30);
const GATEWAY_CLIENT_ID: &str = "agent-gateway";
pub(super) static REQUEST_COUNTER: AtomicU64 = AtomicU64::new(1);

/// The grant a forwarded request acts under, as the origin kernel knows it.
pub(crate) struct Origin {
    pub(crate) grant_id: String,
    pub(crate) harness: String,
}

pub(crate) struct MachineGateway {
    directory: Directory,
    config: DaemonConfigProjectionStore,
    runtime_state: KernelRuntimeState,
    identity: std::sync::Mutex<Option<GatewayIdentity>>,
    links: Mutex<HashMap<String, Arc<Link>>>,
    ledger: OnceLock<ledger::GatewayLedger>,
    keeper: keeper::Keeper,
    paired: Arc<paired::PairedMachines>,
    /// When each paired machine last answered, for the self-hosted list.
    last_answer: std::sync::Mutex<HashMap<String, u64>>,
}

impl MachineGateway {
    pub(crate) fn new(
        runtime_state: KernelRuntimeState,
        config: DaemonConfigProjectionStore,
        projection: RemoteRelayInventoryProjectionStore,
    ) -> Self {
        let paired = Arc::new(paired::PairedMachines::new(
            config
                .snapshot()
                .private_runtime_state_root()
                .join("gateway-machines.json"),
        ));
        let source = Arc::new(RelayInventory {
            config: config.clone(),
            projection,
        });
        Self {
            directory: Directory::new(source),
            config,
            runtime_state,
            identity: std::sync::Mutex::new(None),
            links: Mutex::new(HashMap::new()),
            ledger: OnceLock::new(),
            keeper: keeper::Keeper::default(),
            paired,
            last_answer: std::sync::Mutex::new(HashMap::new()),
        }
    }

    /// A relay request reached this kernel with `client_public_key`; false when that
    /// key is a gateway the owner revoked here.
    pub(crate) fn admit_origin(
        &self,
        client_public_key: &str,
        origin_kernel: Option<&str>,
    ) -> Result<bool, crate::error::DaemonError> {
        origins::admit(
            &origins::path_in(&self.config.snapshot().private_runtime_state_root()),
            client_public_key,
            origin_kernel,
            crate::session::unix_epoch_ms(),
        )
    }

    /// The durable ledger a target keeps for requests forwarded to it.
    pub(crate) fn ledger(&self) -> &ledger::GatewayLedger {
        self.ledger.get_or_init(|| {
            ledger::GatewayLedger::open(
                self.config
                    .snapshot()
                    .private_runtime_state_root()
                    .join("gateway-ledger.jsonl"),
            )
        })
    }

    /// Forwards `request` to `machine` and returns the target's response value.
    pub(crate) async fn forward(
        &self,
        machine: &str,
        origin: &Origin,
        command_id: &str,
        request: &LocalDaemonRequest,
        deadline: Duration,
    ) -> Result<Value, GatewayError> {
        let kernel = self.resolve(machine).await?;
        let gateway_ready = is_gateway_ready(&kernel);
        if !gateway_ready && matches!(request, LocalDaemonRequest::RespondToInteraction(_)) {
            return Err(GatewayError::Incompatible {
                name: machine.into(),
                detail: format!(
                    "it predates the {GATEWAY_CAPABILITY} contract that classifies approvals itself"
                ),
            });
        }
        let mut envelope = serde_json::json!({"command_id": command_id, "request": request});
        if gateway_ready {
            envelope["on_behalf_of"] = serde_json::to_value(OnBehalfOf {
                origin_kernel: self.config.snapshot().daemon_id,
                grant_id: origin.grant_id.clone(),
                harness: origin.harness.clone(),
            })
            .expect("on_behalf_of serializes");
        }
        let plaintext = serde_json::to_vec(&envelope).expect("envelope serializes");
        let link = self.link_for(machine, &kernel).await?;
        let request_id = format!(
            "gw-{}-{}",
            crate::session::unix_epoch_ms(),
            REQUEST_COUNTER.fetch_add(1, Ordering::Relaxed)
        );
        match link.request(request_id, &plaintext, deadline).await {
            Ok(bytes) => {
                serde_json::from_slice(&bytes).map_err(|error| GatewayError::Unreachable {
                    name: machine.into(),
                    detail: format!("unreadable answer: {error}"),
                })
            }
            Err(failure) => {
                if matches!(&failure, LinkFailure::Relay(error) if error.message.starts_with("invalid relay request payload"))
                {
                    self.links.lock().await.remove(&kernel.kernel_id);
                }
                Err(map_failure(machine, failure))
            }
        }
    }

    async fn link_for(
        &self,
        machine: &str,
        kernel: &chariox_relay::protocol::RelayKernelPresence,
    ) -> Result<Arc<Link>, GatewayError> {
        let mut links = self.links.lock().await;
        if let Some(link) = links.get(&kernel.kernel_id).filter(|link| link.usable()) {
            return Ok(Arc::clone(link));
        }
        let link = self.open_link(machine, kernel).await?;
        links.insert(kernel.kernel_id.clone(), Arc::clone(&link));
        Ok(link)
    }

    /// A fresh connection to `kernel`. A self-hosted machine is held to the key
    /// it had when it was paired, whatever the relay announces now.
    async fn open_link(
        &self,
        machine: &str,
        kernel: &chariox_relay::protocol::RelayKernelPresence,
    ) -> Result<Arc<Link>, GatewayError> {
        let identity = self.identity()?;
        let connection = self.connection(machine, kernel, &identity).await?;
        let pinned = match self.config.snapshot().cloud_relay {
            None => self
                .paired
                .find(&kernel.kernel_id)
                .map(|paired| paired.presence.public_key),
            Some(_) => None,
        };
        let link = Link::open(LinkParams {
            relay_url: connection.relay_url,
            auth_token: connection.relay_token,
            target_kernel_id: kernel.kernel_id.clone(),
            target_public_key: Some(pinned.unwrap_or_else(|| kernel.public_key.clone())),
            client_private_key: identity.private_key,
        })
        .await
        .map_err(|failure| map_open_failure(machine, failure))?;
        Ok(Arc::new(link))
    }

    async fn connection(
        &self,
        machine: &str,
        kernel: &chariox_relay::protocol::RelayKernelPresence,
        identity: &GatewayIdentity,
    ) -> Result<crate::local::KernelClientConnection, GatewayError> {
        // A scoped self-hosted relay admits this kernel as a client only with the
        // CLIENT token the person gave when pairing.
        let config = self.config.snapshot();
        if let (None, Some(relay_url), Some(token)) = (
            &config.cloud_relay,
            &config.relay_url,
            self.paired
                .find(&kernel.kernel_id)
                .and_then(|machine| machine.relay_token),
        ) {
            return Ok(crate::local::KernelClientConnection {
                relay_url: relay_url.clone(),
                relay_token: token,
                target_daemon_id: Some(kernel.kernel_id.clone()),
                target_daemon_alias: kernel.kernel_alias.clone(),
                token_expires_at: None,
                machine_id: Some(kernel.machine_id.clone()),
                kernel_id: Some(kernel.kernel_id.clone()),
            });
        }
        crate::runtime::cloud_relay_connection_executor::kernel_client_connection(
            &self.runtime_state,
            &self.config,
            kernel,
            Some(GATEWAY_CLIENT_ID.to_string()),
            None,
            Some(identity.thumbprint.clone()),
        )
        .await
        .map_err(|error| GatewayError::Unreachable {
            name: machine.into(),
            detail: error.to_string(),
        })
    }

    fn identity(&self) -> Result<GatewayIdentity, GatewayError> {
        let mut identity = self.identity.lock().expect("identity poisoned");
        if let Some(identity) = identity.as_ref() {
            return Ok(identity.clone());
        }
        let created = GatewayIdentity::load_or_create(&self.config.snapshot())?;
        *identity = Some(created.clone());
        Ok(created)
    }
}

/// Opening the link sends nothing to the machine, so a connect that does not
/// finish is "unreachable", never a request timeout that invites a retry.
fn map_open_failure(machine: &str, failure: LinkFailure) -> GatewayError {
    match failure {
        LinkFailure::TimedOut => GatewayError::Unreachable {
            name: machine.into(),
            detail: "the relay did not complete the connection in time".into(),
        },
        failure => map_failure(machine, failure),
    }
}

fn map_failure(machine: &str, failure: LinkFailure) -> GatewayError {
    let name = machine.to_string();
    match failure {
        LinkFailure::Closed => GatewayError::ConnectionLost { name },
        LinkFailure::TimedOut => GatewayError::RequestTimeout { name },
        LinkFailure::IdentityMismatch => GatewayError::Unreachable {
            name,
            detail: "the machine answered with a different key than the one it registered".into(),
        },
        LinkFailure::Relay(error) => match error.code.as_str() {
            "target_not_connected" => GatewayError::Offline { name },
            // The relay drops a request the target never answered when the target disconnects.
            "target_disconnected" => GatewayError::ConnectionLost { name },
            "authorization_denied" if error.message.contains("unpaired or revoked") => {
                GatewayError::Unpaired { name }
            }
            // The machine could not decrypt the request: it restarted with another key since
            // this connection pinned the old one. A new connection checks the key again.
            "invalid_request" if error.message.starts_with("invalid relay request payload") => {
                GatewayError::Unreachable {
                    name,
                    detail: "the machine could not read the request; it may have restarted with a new key. Retry once".into(),
                }
            }
            "invalid_request" => GatewayError::Incompatible {
                name,
                // An older target lists every variant it knows; the first part says what is wrong.
                detail: error.message.chars().take(240).collect(),
            },
            "connect_failed" | "connect_refused" | "empty_response" | "decrypt_failed"
            | "encrypt_failed" => GatewayError::Unreachable {
                name,
                detail: error.message,
            },
            _ => GatewayError::Remote {
                name,
                code: error.code,
                message: error.message,
                retryable: error.retryable,
            },
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // EA2-09(c): with the relay stalled, a machine without an open link is
    // unreachable within the connect bound; an open link that stalls times out.
    #[test]
    fn a_stalled_connect_is_unreachable_and_a_stalled_request_times_out() {
        assert_eq!(
            map_open_failure("b3", LinkFailure::TimedOut).code(),
            "machine_unreachable"
        );
        assert_eq!(
            map_failure("b3", LinkFailure::TimedOut).code(),
            "machine_request_timeout"
        );
        assert_eq!(
            map_open_failure("b3", LinkFailure::Closed).code(),
            "machine_connection_lost"
        );
        // A target that restarted with a new key cannot read an old pinned request: retry, not "update".
        let unreadable = chariox_relay::protocol::RelayError {
            code: "invalid_request".into(),
            message: "invalid relay request payload: aead::Error".into(),
            retryable: false,
        };
        assert_eq!(
            map_failure("b3", LinkFailure::Relay(unreadable)).code(),
            "machine_unreachable"
        );
        // EA2-09(a): the target killed with a request in flight is a lost connection, not a raw relay code.
        let disconnected = chariox_relay::protocol::RelayError {
            code: "target_disconnected".into(),
            message: "the target disconnected".into(),
            retryable: true,
        };
        assert_eq!(
            map_failure("b3", LinkFailure::Relay(disconnected)).code(),
            "machine_connection_lost"
        );
    }
}
