use std::fs;
use std::io::Write;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use tokio::net::TcpListener;
use tokio::task::JoinHandle;

use crate::config::DaemonConfig;
use crate::local::LOCAL_DAEMON_PROTOCOL_VERSION;
use crate::runtime::router::CommandRouter;

const ACTIVE_KERNEL_LEASE_SCHEMA_VERSION: u32 = 1;
const ACTIVE_KERNEL_HEARTBEAT_INTERVAL: Duration = Duration::from_secs(10);
const STALE_KERNEL_LEASE_RETENTION: Duration = Duration::from_secs(24 * 60 * 60);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct LocalKernelPresenceRecord {
    pub schema_version: u32,
    pub kernel_id: String,
    #[serde(default)]
    pub kernel_alias: Option<String>,
    pub machine_id: String,
    #[serde(default)]
    pub machine_alias: Option<String>,
    pub host: String,
    pub port: u16,
    #[serde(default)]
    pub relay_url: Option<String>,
    pub relay_connected: bool,
    pub process_id: u32,
    pub started_at_ms: u64,
    pub heartbeat_at_ms: u64,
    pub local_daemon_protocol_version: u32,
}

pub(crate) struct LocalKernelPresenceLease {
    path: PathBuf,
    heartbeat_task: JoinHandle<()>,
}

impl LocalKernelPresenceLease {
    pub(crate) async fn start(router: &Arc<CommandRouter>, listener: &TcpListener) -> Option<Self> {
        let local_addr = listener.local_addr().ok()?;
        let relay_status = router.transport_relay_status_snapshot().await;
        let now_ms = now_unix_ms();
        let record = LocalKernelPresenceRecord {
            schema_version: ACTIVE_KERNEL_LEASE_SCHEMA_VERSION,
            kernel_id: relay_status.daemon_id,
            kernel_alias: relay_status.daemon_alias,
            machine_id: relay_status.machine_id,
            machine_alias: relay_status.machine_alias,
            host: local_connect_host(local_addr),
            port: local_addr.port(),
            relay_url: relay_status.relay_url,
            relay_connected: relay_status.connected,
            process_id: std::process::id(),
            started_at_ms: now_ms,
            heartbeat_at_ms: now_ms,
            local_daemon_protocol_version: LOCAL_DAEMON_PROTOCOL_VERSION,
        };
        let path = active_kernel_path(&record.kernel_id);
        if let Some(directory) = path.parent() {
            prune_stale_presence_records(directory, now_ms);
        }
        if let Err(error) = write_presence_record(&path, &record) {
            crate::logging::warn_with_fields(
                "daemon.local_presence",
                "failed to publish local kernel presence",
                serde_json::json!({
                    "error": error.to_string(),
                    "path": path,
                }),
            );
            return None;
        }

        let heartbeat_path = path.clone();
        let heartbeat_router = Arc::clone(router);
        let heartbeat_task = tokio::spawn(async move {
            let mut record = record;
            loop {
                tokio::time::sleep(ACTIVE_KERNEL_HEARTBEAT_INTERVAL).await;
                let relay_status = heartbeat_router.transport_relay_status_snapshot().await;
                record.heartbeat_at_ms = now_unix_ms();
                record.kernel_alias = relay_status.daemon_alias;
                record.machine_alias = relay_status.machine_alias;
                record.relay_url = relay_status.relay_url;
                record.relay_connected = relay_status.connected;
                if let Err(error) = write_presence_record(&heartbeat_path, &record) {
                    crate::logging::warn_with_fields(
                        "daemon.local_presence",
                        "failed to refresh local kernel presence",
                        serde_json::json!({
                            "error": error.to_string(),
                            "path": heartbeat_path,
                        }),
                    );
                    break;
                }
            }
        });
        Some(Self {
            path,
            heartbeat_task,
        })
    }
}

impl Drop for LocalKernelPresenceLease {
    fn drop(&mut self) {
        self.heartbeat_task.abort();
        if let Err(error) = fs::remove_file(&self.path) {
            if error.kind() != std::io::ErrorKind::NotFound {
                crate::logging::warn_with_fields(
                    "daemon.local_presence",
                    "failed to remove local kernel presence",
                    serde_json::json!({
                        "error": error.to_string(),
                        "path": self.path,
                    }),
                );
            }
        }
    }
}

fn active_kernel_path(kernel_id: &str) -> PathBuf {
    let safe_kernel_id = kernel_id
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.') {
                character
            } else {
                '_'
            }
        })
        .collect::<String>();
    DaemonConfig::default_active_kernel_registry_dir().join(format!("{safe_kernel_id}.json"))
}

fn local_connect_host(local_addr: SocketAddr) -> String {
    if local_addr.ip().is_unspecified() {
        if local_addr.is_ipv6() {
            "::1".to_string()
        } else {
            "127.0.0.1".to_string()
        }
    } else {
        local_addr.ip().to_string()
    }
}

fn write_presence_record(path: &Path, record: &LocalKernelPresenceRecord) -> std::io::Result<()> {
    let Some(parent) = path.parent() else {
        return Err(std::io::Error::other("active kernel path has no parent"));
    };
    fs::create_dir_all(parent)?;
    set_owner_only_directory_permissions(parent)?;
    let temporary_path = path.with_extension(format!("{}.tmp", std::process::id()));
    let contents = serde_json::to_vec(record).map_err(std::io::Error::other)?;
    let mut file = fs::File::create(&temporary_path)?;
    set_owner_only_file_permissions(&temporary_path)?;
    file.write_all(&contents)?;
    fs::rename(temporary_path, path)
}

fn prune_stale_presence_records(directory: &Path, now_ms: u64) {
    let Ok(entries) = fs::read_dir(directory) else {
        return;
    };
    let retention_ms = STALE_KERNEL_LEASE_RETENTION.as_millis() as u64;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|value| value.to_str()) != Some("json") {
            continue;
        }
        let heartbeat_at_ms = fs::read(&path)
            .ok()
            .and_then(|contents| {
                serde_json::from_slice::<LocalKernelPresenceRecord>(&contents).ok()
            })
            .map(|record| record.heartbeat_at_ms);
        if heartbeat_at_ms
            .is_some_and(|heartbeat_at_ms| now_ms.saturating_sub(heartbeat_at_ms) > retention_ms)
        {
            let _ = fs::remove_file(path);
        }
    }
}

#[cfg(unix)]
fn set_owner_only_directory_permissions(path: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o700))
}

#[cfg(not(unix))]
fn set_owner_only_directory_permissions(_path: &Path) -> std::io::Result<()> {
    Ok(())
}

#[cfg(unix)]
fn set_owner_only_file_permissions(path: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o600))
}

#[cfg(not(unix))]
fn set_owner_only_file_permissions(_path: &Path) -> std::io::Result<()> {
    Ok(())
}

fn now_unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::{local_connect_host, LocalKernelPresenceRecord};
    use crate::local::LOCAL_DAEMON_PROTOCOL_VERSION;

    #[test]
    fn unspecified_bind_addresses_publish_loopback_connect_hosts() {
        assert_eq!(
            local_connect_host("0.0.0.0:43118".parse().unwrap()),
            "127.0.0.1"
        );
        assert_eq!(local_connect_host("[::]:43118".parse().unwrap()), "::1");
    }

    #[test]
    fn presence_record_serializes_the_discovery_contract() {
        let record = LocalKernelPresenceRecord {
            schema_version: 1,
            kernel_id: "kernel-1".to_string(),
            kernel_alias: Some("work".to_string()),
            machine_id: "machine-1".to_string(),
            machine_alias: Some("laptop".to_string()),
            host: "127.0.0.1".to_string(),
            port: 43118,
            relay_url: Some("ws://127.0.0.1:47000".to_string()),
            relay_connected: true,
            process_id: 42,
            started_at_ms: 100,
            heartbeat_at_ms: 110,
            local_daemon_protocol_version: LOCAL_DAEMON_PROTOCOL_VERSION,
        };
        let value = serde_json::to_value(record).unwrap();
        assert_eq!(value["kernel_id"], "kernel-1");
        assert_eq!(value["heartbeat_at_ms"], 110);
        assert_eq!(
            value["local_daemon_protocol_version"],
            LOCAL_DAEMON_PROTOCOL_VERSION
        );
    }
}
