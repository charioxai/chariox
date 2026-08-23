use std::collections::BTreeMap;
use std::process::{Child, Command, Stdio};
use std::sync::{Mutex, OnceLock};
use std::thread::sleep;
use std::time::{Duration, Instant};

use crate::error::DaemonError;

use super::super::codex_client::{codex_endpoint_is_healthy, CODEX_ENDPOINT_STARTUP_TIMEOUT};
use super::ports::resolve_codex_catalog_port;

struct ManagedAccountEndpoint {
    endpoint: String,
    child: Child,
}

static CODEX_ACCOUNT_ENDPOINTS: OnceLock<Mutex<BTreeMap<String, ManagedAccountEndpoint>>> =
    OnceLock::new();

pub fn codex_catalog_endpoint() -> Result<String, DaemonError> {
    let _guard = crate::env_lock::lock();
    codex_catalog_endpoint_unlocked()
}

fn codex_catalog_endpoint_unlocked() -> Result<String, DaemonError> {
    let port = resolve_codex_catalog_port()?;
    Ok(format!("ws://127.0.0.1:{port}"))
}

pub(crate) fn ensure_codex_account_endpoint(
    owner_user_id: &str,
    account_profile: &str,
    environment: BTreeMap<String, String>,
) -> Result<String, DaemonError> {
    let key = format!("{owner_user_id}\0{account_profile}");
    let endpoints = CODEX_ACCOUNT_ENDPOINTS.get_or_init(|| Mutex::new(BTreeMap::new()));
    let mut endpoints = endpoints
        .lock()
        .map_err(|error| DaemonError::LocalTransport {
            operation: "ensure_codex_account_endpoint",
            message: error.to_string(),
        })?;
    if let Some(existing) = endpoints.get_mut(&key) {
        if codex_endpoint_is_healthy(&existing.endpoint) {
            return Ok(existing.endpoint.clone());
        }
        let _ = crate::runtime::process_health::terminate_process_tree(existing.child.id());
        let _ = existing.child.wait();
        endpoints.remove(&key);
    }

    let request = crate::provider::LaunchProviderRequest::new(
        "provider-account",
        "codex",
        "codex",
        account_profile,
        "default",
    )
    .with_owner_user_id(owner_user_id)
    .with_provider_account_env(environment);
    let launch = super::plan_codex_launch(Some(&request))?;
    let endpoint = launch
        .structured_endpoint
        .ok_or_else(|| DaemonError::LocalTransport {
            operation: "ensure_codex_account_endpoint",
            message: "Codex account launch did not expose an endpoint".to_string(),
        })?;
    let program = launch
        .pty_program
        .ok_or_else(|| DaemonError::LocalTransport {
            operation: "ensure_codex_account_endpoint",
            message: "Codex account launch did not expose an executable".to_string(),
        })?;
    let mut command = Command::new(program);
    command
        .args(launch.pty_args)
        .envs(launch.pty_env)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    for name in launch.pty_env_remove {
        command.env_remove(name);
    }
    for name in crate::account_profile::provider_auth_env_vars("codex") {
        command.env_remove(name);
    }
    let mut child = command
        .spawn()
        .map_err(|error| DaemonError::LocalTransport {
            operation: "ensure_codex_account_endpoint",
            message: format!("failed to start profile-specific Codex app-server: {error}"),
        })?;
    let deadline = Instant::now() + CODEX_ENDPOINT_STARTUP_TIMEOUT;
    loop {
        if codex_endpoint_is_healthy(&endpoint) {
            endpoints.insert(
                key,
                ManagedAccountEndpoint {
                    endpoint: endpoint.clone(),
                    child,
                },
            );
            return Ok(endpoint);
        }
        if let Some(status) = child
            .try_wait()
            .map_err(|error| DaemonError::LocalTransport {
                operation: "ensure_codex_account_endpoint",
                message: format!("failed to poll Codex app-server startup: {error}"),
            })?
        {
            return Err(DaemonError::LocalTransport {
                operation: "ensure_codex_account_endpoint",
                message: format!("profile-specific Codex app-server exited early: {status}"),
            });
        }
        if Instant::now() >= deadline {
            let _ = crate::runtime::process_health::terminate_process_tree(child.id());
            let _ = child.wait();
            return Err(DaemonError::LocalTransport {
                operation: "ensure_codex_account_endpoint",
                message: "timed out waiting for profile-specific Codex app-server".to_string(),
            });
        }
        sleep(Duration::from_millis(100));
    }
}

pub(crate) fn invalidate_codex_account_endpoint(owner_user_id: &str, account_profile: &str) {
    let Some(endpoints) = CODEX_ACCOUNT_ENDPOINTS.get() else {
        return;
    };
    let key = format!("{owner_user_id}\0{account_profile}");
    let Ok(mut endpoints) = endpoints.lock() else {
        return;
    };
    if let Some(mut endpoint) = endpoints.remove(&key) {
        let _ = crate::runtime::process_health::terminate_process_tree(endpoint.child.id());
        let _ = endpoint.child.wait();
    }
}

pub(crate) fn shutdown_codex_account_endpoints() {
    let Some(endpoints) = CODEX_ACCOUNT_ENDPOINTS.get() else {
        return;
    };
    let Ok(mut endpoints) = endpoints.lock() else {
        return;
    };
    let drained = std::mem::take(&mut *endpoints);
    drop(endpoints);

    for (_, mut endpoint) in drained {
        let _ = crate::runtime::process_health::terminate_process_tree(endpoint.child.id());
        let _ = endpoint.child.wait();
    }
}
