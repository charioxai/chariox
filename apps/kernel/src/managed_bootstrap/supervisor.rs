use std::process::{Child, Command, ExitStatus, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use chrono::Utc;

use crate::error::DaemonError;

use super::cloud::BootstrapCloudClient;
use super::release::VerifiedRelease;
use super::state::BootstrapConfig;
use super::{jittered, PendingConfirmation};

const MIN_RESTART_DELAY: Duration = Duration::from_secs(1);
const MAX_RESTART_DELAY: Duration = Duration::from_secs(30);
const MIN_CONFIRM_RETRY_DELAY: Duration = Duration::from_secs(1);
const MAX_CONFIRM_RETRY_DELAY: Duration = Duration::from_secs(30);
const MAX_CONFIRMATION_WAIT: Duration = Duration::from_secs(10 * 60);
const STABLE_RUNTIME: Duration = Duration::from_secs(30);

pub(super) struct KernelRun {
    status: ExitStatus,
    runtime: Duration,
}

pub(super) fn supervise_kernel(
    config: &BootstrapConfig,
    release: &VerifiedRelease,
    mut confirmation: Option<PendingConfirmation>,
    cloud: &impl BootstrapCloudClient,
) -> Result<(), DaemonError> {
    let mut restart_delay = MIN_RESTART_DELAY;
    loop {
        let started_at = Instant::now();
        match run_kernel_once(config, release, &mut confirmation, cloud) {
            Ok(run) => {
                if run.runtime >= STABLE_RUNTIME {
                    restart_delay = MIN_RESTART_DELAY;
                }
                crate::logging::warn_with_fields(
                    "managed_bootstrap.kernel_exit",
                    "managed kernel exited; supervisor will restart it",
                    serde_json::json!({
                        "status": run.status.code(),
                        "restart_delay_ms": restart_delay.as_millis(),
                    }),
                );
            }
            Err(error) => crate::logging::warn_with_fields(
                "managed_bootstrap.kernel_spawn_failed",
                "managed kernel run failed; supervisor will retry",
                serde_json::json!({
                    "error": error.to_string(),
                    "restart_delay_ms": restart_delay.as_millis(),
                }),
            ),
        }
        if started_at.elapsed() >= STABLE_RUNTIME {
            restart_delay = MIN_RESTART_DELAY;
        }
        thread::sleep(jittered(restart_delay));
        restart_delay = restart_delay.saturating_mul(2).min(MAX_RESTART_DELAY);
    }
}

pub(super) fn run_kernel_once(
    config: &BootstrapConfig,
    release: &VerifiedRelease,
    confirmation: &mut Option<PendingConfirmation>,
    cloud: &impl BootstrapCloudClient,
) -> Result<KernelRun, DaemonError> {
    let started_at = Instant::now();
    let mut child = spawn_kernel(config, release)?;
    if confirmation.is_some() {
        await_relay_ready_confirmation(config, &mut child, confirmation, cloud)?;
    }
    let status = child
        .wait()
        .map_err(|error| supervisor_error(&format!("wait for managed kernel: {error}")))?;
    Ok(KernelRun {
        status,
        runtime: started_at.elapsed(),
    })
}

fn spawn_kernel(config: &BootstrapConfig, release: &VerifiedRelease) -> Result<Child, DaemonError> {
    Command::new(&release.kernel_binary)
        .current_dir(&config.chariox_home)
        .env("CHARIOX_HOME", &config.chariox_home)
        .env(
            "CHARIOX_CAPABILITY_ISOLATION_ROOT",
            config.chariox_home.join("managed-context").join("kernel"),
        )
        .env(
            "CHARIOX_MANAGED_VAULT_PATH",
            config
                .chariox_home
                .join(".chariox")
                .join("vault")
                .join("vault.json"),
        )
        .env("CHARIOX_KERNEL_HOST", &config.kernel_host)
        .env("CHARIOX_KERNEL_PORT", config.kernel_port.to_string())
        .env_remove("CHARIOX_DAEMON_ID")
        .env_remove("CHARIOX_MACHINE_ID")
        .env_remove("CHARIOX_RELAY_TOKEN")
        .stdin(Stdio::null())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .spawn()
        .map_err(|error| supervisor_error(&format!("start managed kernel: {error}")))
}

fn await_relay_ready_confirmation(
    config: &BootstrapConfig,
    child: &mut Child,
    confirmation: &mut Option<PendingConfirmation>,
    cloud: &impl BootstrapCloudClient,
) -> Result<(), DaemonError> {
    let started_at = Instant::now();
    let mut retry_delay = MIN_CONFIRM_RETRY_DELAY;
    loop {
        if let Some(status) = child
            .try_wait()
            .map_err(|error| supervisor_error(&format!("inspect managed kernel: {error}")))?
        {
            return Err(supervisor_error(&format!(
                "managed kernel exited before relay-ready confirmation with status {status}"
            )));
        }
        let pending = confirmation
            .as_ref()
            .ok_or_else(|| supervisor_error("managed confirmation state disappeared"))?;
        match pending.confirm(config, cloud, Utc::now()) {
            Ok(()) => {
                confirmation.take();
                return Ok(());
            }
            Err(error) => {
                crate::logging::warn_with_fields(
                    "managed_bootstrap.confirm_pending",
                    "managed kernel is not relay-ready; confirmation will retry",
                    serde_json::json!({
                        "error": error.to_string(),
                        "retry_delay_ms": retry_delay.as_millis(),
                    }),
                );
            }
        }
        if started_at.elapsed() >= MAX_CONFIRMATION_WAIT {
            terminate_child(child);
            return Err(supervisor_error(
                "managed kernel did not establish relay presence before the confirmation deadline",
            ));
        }
        thread::sleep(jittered(retry_delay));
        retry_delay = retry_delay.saturating_mul(2).min(MAX_CONFIRM_RETRY_DELAY);
    }
}

fn terminate_child(child: &mut Child) {
    let _ = child.kill();
    let _ = child.wait();
}

fn supervisor_error(message: &str) -> DaemonError {
    DaemonError::LocalTransport {
        operation: "supervise managed kernel",
        message: message.to_string(),
    }
}
