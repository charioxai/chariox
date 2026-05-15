//! Runtime machine/kernel identity and registry persistence.

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use rand::RngCore;
use serde::{Deserialize, Serialize};

use crate::transport::relay_crypto;

use super::{paths::default_config_dir, DaemonConfig};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct RuntimeIdentity {
    pub(super) daemon_id: String,
    pub(super) machine_id: String,
    #[serde(default)]
    pub(super) machine_alias: Option<String>,
    #[serde(default)]
    pub(super) daemon_alias: Option<String>,
    pub(super) relay_public_key: String,
    pub(super) relay_private_key: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct MachineIdentity {
    machine_id: String,
    #[serde(default)]
    machine_alias: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
struct KernelRegistry {
    #[serde(default = "kernel_registry_version")]
    version: u32,
    #[serde(default)]
    machine_id: String,
    #[serde(default)]
    kernels: BTreeMap<String, KernelIdentityRecord>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
struct KernelIdentityRecord {
    kernel_id: String,
    #[serde(default)]
    kernel_alias: Option<String>,
    #[serde(default)]
    host: String,
    #[serde(default)]
    port: u16,
    #[serde(default)]
    relay_public_key: String,
    #[serde(default)]
    relay_private_key: String,
    #[serde(default)]
    created_at_ms: u64,
    #[serde(default)]
    last_seen_at_ms: u64,
}

fn kernel_registry_version() -> u32 {
    1
}

pub(super) fn load_or_create_runtime_identity(host: &str, port: u16) -> RuntimeIdentity {
    let machine_identity = load_or_create_machine_identity();
    let endpoint_key = kernel_identity_key(host, port);
    let registry_path = DaemonConfig::default_kernel_registry_path();
    let mut registry = fs::read_to_string(&registry_path)
        .ok()
        .and_then(|contents| serde_json::from_str::<KernelRegistry>(&contents).ok())
        .unwrap_or_else(|| KernelRegistry {
            version: kernel_registry_version(),
            machine_id: machine_identity.machine_id.clone(),
            kernels: BTreeMap::new(),
        });
    if registry.version == 0 {
        registry.version = kernel_registry_version();
    }
    if registry.machine_id.trim().is_empty() {
        registry.machine_id = machine_identity.machine_id.clone();
    }

    let now_ms = now_unix_ms();
    let record = registry.kernels.entry(endpoint_key).or_insert_with(|| {
        let legacy_identity = load_legacy_runtime_identity().filter(|identity| {
            is_default_kernel_endpoint(host, port)
                && identity.machine_id == machine_identity.machine_id
                && !identity_is_invalid(identity)
        });
        if let Some(identity) = legacy_identity {
            return KernelIdentityRecord {
                kernel_id: identity.daemon_id,
                kernel_alias: identity.daemon_alias,
                host: host.trim().to_string(),
                port,
                relay_public_key: identity.relay_public_key,
                relay_private_key: identity.relay_private_key,
                created_at_ms: now_ms,
                last_seen_at_ms: now_ms,
            };
        }

        let relay_private_key = relay_crypto::generate_private_key_base64();
        let relay_public_key = relay_crypto::public_key_from_private_key_base64(&relay_private_key)
            .unwrap_or_default();
        KernelIdentityRecord {
            kernel_id: format!("kernel-{}", generate_identity_suffix()),
            kernel_alias: None,
            host: host.trim().to_string(),
            port,
            relay_public_key,
            relay_private_key,
            created_at_ms: now_ms,
            last_seen_at_ms: now_ms,
        }
    });
    if record.host.trim().is_empty() {
        record.host = host.trim().to_string();
    }
    if record.port == 0 {
        record.port = port;
    }
    record.last_seen_at_ms = now_ms;
    let record_snapshot = record.clone();
    let identity = RuntimeIdentity {
        daemon_id: record_snapshot.kernel_id.clone(),
        machine_id: machine_identity.machine_id.clone(),
        machine_alias: machine_identity.machine_alias,
        daemon_alias: record_snapshot.kernel_alias.clone(),
        relay_public_key: record_snapshot.relay_public_key.clone(),
        relay_private_key: record_snapshot.relay_private_key.clone(),
    };

    persist_kernel_registry(&registry_path, &registry);
    persist_kernel_identity(&identity, &record_snapshot);
    identity
}

fn load_or_create_machine_identity() -> MachineIdentity {
    let path = DaemonConfig::default_machine_identity_path();
    if let Ok(contents) = fs::read_to_string(&path) {
        if let Ok(identity) = serde_json::from_str::<MachineIdentity>(&contents) {
            if !identity.machine_id.trim().is_empty() {
                return identity;
            }
        }
    }

    let identity = load_legacy_runtime_identity()
        .filter(|identity| !identity.machine_id.trim().is_empty())
        .map(|identity| MachineIdentity {
            machine_id: identity.machine_id,
            machine_alias: identity.machine_alias,
        })
        .unwrap_or_else(|| MachineIdentity {
            machine_id: format!("machine-{}", generate_identity_suffix()),
            machine_alias: None,
        });
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    if let Ok(contents) = serde_json::to_string_pretty(&identity) {
        let _ = fs::write(&path, contents);
    }
    identity
}

fn load_legacy_runtime_identity() -> Option<RuntimeIdentity> {
    let path = DaemonConfig::default_runtime_identity_path();
    let contents = fs::read_to_string(path).ok()?;
    let identity = serde_json::from_str::<RuntimeIdentity>(&contents).ok()?;
    if identity_is_invalid(&identity) {
        return None;
    }
    Some(identity)
}

fn identity_is_invalid(identity: &RuntimeIdentity) -> bool {
    identity.daemon_id.trim().is_empty()
        || identity.machine_id.trim().is_empty()
        || identity.relay_public_key.trim().is_empty()
        || identity.relay_private_key.trim().is_empty()
}

fn persist_kernel_registry(path: &PathBuf, registry: &KernelRegistry) {
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    if let Ok(contents) = serde_json::to_string_pretty(registry) {
        let _ = fs::write(path, contents);
    }
}

fn persist_kernel_identity(identity: &RuntimeIdentity, record: &KernelIdentityRecord) {
    let path = default_config_dir()
        .join("kernels")
        .join(&identity.daemon_id)
        .join("identity.json");
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    if let Ok(contents) = serde_json::to_string_pretty(record) {
        let _ = fs::write(path, contents);
    }
}

fn kernel_identity_key(host: &str, port: u16) -> String {
    format!("{}:{port}", host.trim().to_ascii_lowercase())
}

fn is_default_kernel_endpoint(host: &str, port: u16) -> bool {
    port == 43118 && matches!(host.trim(), "127.0.0.1" | "localhost")
}

fn now_unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

pub(super) fn generate_identity_suffix() -> String {
    let mut bytes = [0u8; 8];
    rand::thread_rng().fill_bytes(&mut bytes);
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
