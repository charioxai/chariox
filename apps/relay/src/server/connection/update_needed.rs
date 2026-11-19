//! A kernel older than this relay cannot register: its registration no longer reads. The relay
//! still carries none of its traffic, but records who it refused and the protocol it reported,
//! so the kernel's own machine entry says Update needed instead of the kernel vanishing.
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Instant;

use serde::Deserialize;
use serde_json::json;
use tokio::sync::RwLock;

use super::support::{relay_log, validate_registration_identity, verify_relay_token};
use crate::auth::{RelayAction, RelayAuthVerifier};
use crate::registry::{
    capabilities_name_kernel_target, DaemonKey, RelayRegistry, UpdateNeededKernel,
};

/// Only what names an old kernel; the rest of its registration may have any older shape.
#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum OldRegister {
    DaemonRegister { registration: OldRegistration },
}

#[derive(Deserialize)]
struct OldRegistration {
    auth_token: String,
    daemon_id: String,
    machine_id: String,
    #[serde(default)]
    machine_alias: Option<String>,
    public_key: String,
    #[serde(default)]
    capabilities: Vec<String>,
    #[serde(default)]
    local_protocol: Option<u32>,
}

/// The close reason for a first frame that does not read. `Ok(None)`: it is no kernel's
/// registration, so the frame is simply invalid. A refused kernel is recorded only once its
/// token names it, its machine and its key.
pub(super) async fn refuse_old_registration(
    text: &str,
    auth_verifier: &RelayAuthVerifier,
    registry: &Arc<RwLock<RelayRegistry>>,
    peer_addr: SocketAddr,
) -> Result<Option<String>, std::io::Error> {
    let Ok(OldRegister::DaemonRegister { registration }) = serde_json::from_str(text) else {
        return Ok(None);
    };
    let identity = verify_relay_token(
        auth_verifier,
        &registration.auth_token,
        RelayAction::DaemonRegister,
        None,
    )?;
    if let Err(reason) = validate_registration_identity(
        &identity,
        &registration.daemon_id,
        &registration.machine_id,
        &registration.public_key,
    ) {
        return Ok(Some(reason.to_string()));
    }
    relay_log(
        "warn",
        "relay_registration_update_needed",
        json!({
            "peer_addr": peer_addr.to_string(),
            "daemon_id": registration.daemon_id,
            "local_protocol": registration.local_protocol,
        }),
    );
    if capabilities_name_kernel_target(&registration.capabilities) {
        registry.write().await.record_update_needed_kernel(
            DaemonKey::new(identity.realm_id, registration.daemon_id),
            UpdateNeededKernel {
                machine_id: registration.machine_id,
                machine_alias: registration.machine_alias,
                local_protocol: registration.local_protocol,
                refused_at: Instant::now(),
            },
        );
    }
    Ok(Some(update_needed_reason(registration.local_protocol)))
}

fn update_needed_reason(local_protocol: Option<u32>) -> String {
    let protocol = local_protocol.map_or_else(
        || "reports no protocol".to_string(),
        |protocol| format!("runs protocol {protocol}"),
    );
    format!(
        "update needed: this kernel {protocol} and is older than the relay; update Chariox on this machine and restart the kernel"
    )
}
