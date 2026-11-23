use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use chariox_relay::protocol::ClientTarget;
use sha2::{Digest, Sha256};
use tokio::sync::{watch, RwLock};
use tokio::time::{sleep_until, timeout, Instant};

use crate::config::DaemonConfig;
use crate::managed_bootstrap::ConfirmedManagedKernelRegistration;
use crate::runtime::cloud_api_client::{cloud_error_is_retryable, post_cloud_json};
use crate::runtime::managed_kernel_quiescence::hmac_signature;
use crate::transport::relay_client::{
    send_peer_request_to_known_kernel_via_relay_with_timeout, RelayClientState,
};
use crate::transport::relay_peer::{RelayPeerRequest, RelayPeerResponse};

const SOAK_TIME: Duration = Duration::from_secs(5 * 60);
const ATTACH_TIMEOUT: Duration = Duration::from_secs(2 * 60);
const FILE_TIMEOUT: Duration = Duration::from_secs(10);
const COMPILE_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum StartupProbeFailure {
    RelayUnavailable,
    RelayInterrupted,
    FileCheckFailed,
    CompileFailed,
}

#[derive(serde::Deserialize)]
struct HealthReceipt {
    accepted: bool,
}

pub(crate) async fn verify_and_report(
    config: DaemonConfig,
    registration: ConfirmedManagedKernelRegistration,
    relay: Arc<RwLock<RelayClientState>>,
    mut shutdown: watch::Receiver<bool>,
) {
    let Some(profile) = config.cloud_relay.as_ref() else {
        return;
    };
    let Some(credential) = profile.machine_credential.as_deref() else {
        return;
    };
    let result = tokio::select! {
        result = verify_startup(&config, &relay) => result,
        _ = shutdown.changed() => return,
    };
    let outcome = match result {
        Ok(()) => serde_json::json!("passed"),
        Err(failure) => serde_json::to_value(failure).expect("startup check outcome serializes"),
    };
    let mut fields = BTreeMap::from([
        ("protocolVersion", serde_json::json!(1)),
        ("action", serde_json::json!("startup_health")),
        ("accountId", serde_json::json!(profile.account_id)),
        (
            "environmentId",
            serde_json::json!(registration.environment_id),
        ),
        ("machineId", serde_json::json!(registration.machine_id)),
        ("kernelId", serde_json::json!(registration.kernel_id)),
        ("outcome", outcome),
    ]);
    let signature =
        hmac_signature(credential, &fields).expect("startup check fields can be signed");
    fields.insert("signature", serde_json::json!(signature));
    fields.insert("machineCredential", serde_json::json!(credential));
    let body = serde_json::to_value(fields).expect("startup check request serializes");
    loop {
        let report = tokio::select! {
            report = post_cloud_json::<HealthReceipt>(
                profile.api_url.clone(), "/v1/kernels/startup-health", body.clone(),
            ) => report,
            _ = shutdown.changed() => return,
        };
        match report {
            Ok(receipt) if receipt.accepted => return,
            Err(error) if cloud_error_is_retryable(&error) => {}
            _ => {
                crate::logging::warn_with_fields(
                    "managed_startup.probe_report_failed",
                    "The service did not accept the managed startup check result",
                    serde_json::json!({"environment_id": registration.environment_id}),
                );
                return;
            }
        }
        tokio::select! {
            () = tokio::time::sleep(Duration::from_secs(5)) => {}
            _ = shutdown.changed() => return,
        }
    }
}

async fn verify_startup(
    config: &DaemonConfig,
    relay: &Arc<RwLock<RelayClientState>>,
) -> Result<(), StartupProbeFailure> {
    wait_for_stable_relay(relay).await?;
    let directory = Scratch::new().await?;
    let probe_file = directory.0.join("probe.py");
    let returned_file = directory.0.join("returned.py");
    let contents = format!(
        "# {:016x}\nresult = sum(range(128))\n",
        rand::random::<u64>()
    );
    write(&probe_file, contents.as_bytes()).await?;

    // Files normally avoids the relay for local copies. Use the encrypted peer
    // request here so the probe also traverses the relay's reply queue.
    let expected = format!("{:x}", Sha256::digest(contents.as_bytes()));
    let returned = read_verified_file(config, relay, &probe_file, &expected).await?;
    write(&returned_file, &returned).await?;
    read_verified_file(config, relay, &returned_file, &expected).await?;
    compile(&returned_file).await?;
    verify_same_connection(relay).await
}

async fn wait_for_stable_relay(
    relay: &Arc<RwLock<RelayClientState>>,
) -> Result<(), StartupProbeFailure> {
    let mut changes = relay.read().await.connection_changes();
    let attach_deadline = Instant::now() + ATTACH_TIMEOUT;
    loop {
        changes.borrow_and_update();
        let connection = relay.read().await.startup_probe_connection();
        let deadline = match connection {
            (0, None) => attach_deadline,
            (1, Some(connected_since)) => connected_since + SOAK_TIME,
            _ => return Err(StartupProbeFailure::RelayInterrupted),
        };
        if Instant::now() >= deadline {
            return if connection.0 == 0 {
                Err(StartupProbeFailure::RelayUnavailable)
            } else {
                Ok(())
            };
        }
        tokio::select! {
            changed = changes.changed() => {
                if changed.is_err() {
                    return Err(StartupProbeFailure::RelayUnavailable);
                }
            }
            () = sleep_until(deadline) => {}
        }
    }
}

async fn verify_same_connection(
    relay: &Arc<RwLock<RelayClientState>>,
) -> Result<(), StartupProbeFailure> {
    match relay.read().await.startup_probe_connection() {
        (1, Some(_)) => Ok(()),
        _ => Err(StartupProbeFailure::RelayInterrupted),
    }
}

async fn read_verified_file(
    config: &DaemonConfig,
    relay: &Arc<RwLock<RelayClientState>>,
    path: &Path,
    expected: &str,
) -> Result<Vec<u8>, StartupProbeFailure> {
    let reply = send_peer_request_to_known_kernel_via_relay_with_timeout(
        config,
        relay,
        ClientTarget {
            daemon_id: Some(config.daemon_id.clone()),
            daemon_alias: None,
        },
        &config.relay_public_key,
        RelayPeerRequest::ReadMachineFile {
            path: path
                .to_str()
                .ok_or(StartupProbeFailure::FileCheckFailed)?
                .to_owned(),
            offset: 0,
        },
        FILE_TIMEOUT,
    )
    .await;
    verify_same_connection(relay).await?;
    let Ok(RelayPeerResponse::FileSliceRead {
        size_bytes,
        sha256,
        content,
        ..
    }) = reply
    else {
        return Err(StartupProbeFailure::FileCheckFailed);
    };
    let bytes = content
        .decode()
        .ok_or(StartupProbeFailure::FileCheckFailed)?;
    if size_bytes != bytes.len() as u64
        || sha256.as_deref() != Some(expected)
        || format!("{:x}", Sha256::digest(&*bytes)) != expected
    {
        return Err(StartupProbeFailure::FileCheckFailed);
    }
    Ok(bytes.to_vec())
}

async fn compile(path: &Path) -> Result<(), StartupProbeFailure> {
    let status = timeout(
        COMPILE_TIMEOUT,
        tokio::process::Command::new("/usr/bin/python3")
            .args(["-I", "-m", "py_compile"])
            .arg(path)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .status(),
    )
    .await;
    match status {
        Ok(Ok(status)) if status.success() => Ok(()),
        _ => Err(StartupProbeFailure::CompileFailed),
    }
}

async fn write(path: &Path, bytes: &[u8]) -> Result<(), StartupProbeFailure> {
    let (path, bytes) = (path.to_owned(), bytes.to_owned());
    crate::shutdown::spawn_blocking("write managed startup probe", move || {
        std::fs::write(path, bytes)
    })
    .await
    .map_err(|_| StartupProbeFailure::FileCheckFailed)?
    .map_err(|_| StartupProbeFailure::FileCheckFailed)
}

struct Scratch(PathBuf);

impl Scratch {
    async fn new() -> Result<Self, StartupProbeFailure> {
        let root = super::empty::managed_user_workspace_root()
            .map_err(|_| StartupProbeFailure::FileCheckFailed)?;
        let path = root.join(format!(
            ".chariox-startup-probe-{:016x}",
            rand::random::<u64>()
        ));
        crate::shutdown::spawn_blocking("prepare managed startup probe", move || {
            let mut directory = std::fs::DirBuilder::new();
            #[cfg(unix)]
            {
                use std::os::unix::fs::DirBuilderExt;
                directory.mode(0o700);
            }
            directory.create(&path).map(|()| Self(path))
        })
        .await
        .map_err(|_| StartupProbeFailure::FileCheckFailed)?
        .map_err(|_| StartupProbeFailure::FileCheckFailed)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let directory = self.0.clone();
        drop(crate::shutdown::spawn_blocking(
            "remove managed startup probe",
            move || {
                if let Err(error) = std::fs::remove_dir_all(directory) {
                    crate::logging::warn_with_fields(
                        "managed_startup.probe_cleanup",
                        "could not remove the managed startup check files",
                        serde_json::json!({"error": error.to_string()}),
                    );
                }
            },
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transport::relay_client::RelayOutgoingSender;

    #[tokio::test(start_paused = true)]
    async fn readiness_waits_five_minutes_on_the_acknowledged_connection() {
        let relay = Arc::new(RwLock::new(RelayClientState::default()));
        let (sender, _priority, _events) = RelayOutgoingSender::channel(1);
        relay
            .write()
            .await
            .test_set_connected_sender(sender, "wss://relay.example.test");
        let stable = tokio::spawn({
            let relay = Arc::clone(&relay);
            async move { wait_for_stable_relay(&relay).await }
        });
        tokio::time::advance(SOAK_TIME - Duration::from_secs(1)).await;
        assert!(!stable.is_finished());
        tokio::time::advance(Duration::from_secs(1)).await;
        assert_eq!(stable.await.unwrap(), Ok(()));
    }

    #[tokio::test(start_paused = true)]
    async fn readiness_fails_when_the_relay_never_acknowledges() {
        let relay = Arc::new(RwLock::new(RelayClientState::default()));
        assert_eq!(
            wait_for_stable_relay(&relay).await,
            Err(StartupProbeFailure::RelayUnavailable)
        );
    }

    #[tokio::test(start_paused = true)]
    async fn reconnecting_during_startup_fails_the_check() {
        let relay = Arc::new(RwLock::new(RelayClientState::default()));
        let (sender, _priority, _events) = RelayOutgoingSender::channel(1);
        {
            let mut state = relay.write().await;
            state.test_set_connected_sender(sender.clone(), "wss://relay.example.test");
            state.test_mark_disconnected();
            state.test_set_connected_sender(sender, "wss://relay.example.test");
        }
        assert_eq!(
            wait_for_stable_relay(&relay).await,
            Err(StartupProbeFailure::RelayInterrupted)
        );
    }
}
