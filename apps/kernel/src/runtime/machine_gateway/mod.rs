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
pub(crate) mod ledger;
mod link;

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use serde_json::Value;
use tokio::sync::Mutex;

use crate::local::{GatewayMachine, LocalDaemonRequest, OnBehalfOf};
use crate::runtime::projection::{
    DaemonConfigProjectionStore, RemoteRelayInventoryProjectionStore,
};
use crate::runtime::state::KernelRuntimeState;

pub(crate) use directory::GATEWAY_CAPABILITY;
use directory::{is_gateway_ready, Directory};
pub(crate) use error::GatewayError;
use identity::GatewayIdentity;
pub(crate) use inventory::{InventorySource, RelayInventory};
use link::{Link, LinkFailure, LinkParams};

/// The default time a forwarded request may take before `machine_request_timeout`.
pub(crate) const REQUEST_DEADLINE: Duration = Duration::from_secs(30);
const GATEWAY_CLIENT_ID: &str = "agent-gateway";
static REQUEST_COUNTER: AtomicU64 = AtomicU64::new(1);

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
}

impl MachineGateway {
    pub(crate) fn new(
        runtime_state: KernelRuntimeState,
        config: DaemonConfigProjectionStore,
        projection: RemoteRelayInventoryProjectionStore,
    ) -> Self {
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
        }
    }

    pub(crate) async fn machines(&self) -> Result<Vec<GatewayMachine>, GatewayError> {
        self.directory.machines().await
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
        let kernel = self.directory.resolve(machine).await?;
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
            Err(failure) => Err(map_failure(machine, failure)),
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
        let identity = self.identity()?;
        let connection = crate::runtime::cloud_relay_connection_executor::kernel_client_connection(
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
        })?;
        let link = Link::open(LinkParams {
            relay_url: connection.relay_url,
            auth_token: connection.relay_token,
            target_kernel_id: kernel.kernel_id.clone(),
            target_public_key: kernel.public_key.clone(),
            client_private_key: identity.private_key,
        })
        .await
        .map_err(|failure| map_failure(machine, failure))?;
        let link = Arc::new(link);
        links.insert(kernel.kernel_id.clone(), Arc::clone(&link));
        Ok(link)
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
            "authorization_denied" if error.message.contains("unpaired or revoked") => {
                GatewayError::Unpaired { name }
            }
            "invalid_request" => GatewayError::Incompatible {
                name,
                detail: error.message,
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
