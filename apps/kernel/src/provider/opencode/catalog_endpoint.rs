use std::collections::BTreeMap;
use std::process::{Child, Stdio};
use std::sync::{Mutex, OnceLock};
use std::thread::sleep;
use std::time::{Duration, Instant};

use crate::error::DaemonError;
use crate::provider::OpenCodeClient;

use super::ports::resolve_opencode_catalog_port;

struct ManagedAccountEndpoint {
    endpoint: String,
    child: Child,
}

static OPENCODE_ACCOUNT_ENDPOINTS: OnceLock<Mutex<BTreeMap<String, ManagedAccountEndpoint>>> =
    OnceLock::new();

pub fn opencode_catalog_endpoint() -> Result<String, DaemonError> {
    let _guard = crate::env_lock::lock();
    opencode_catalog_endpoint_unlocked()
}

fn opencode_catalog_endpoint_unlocked() -> Result<String, DaemonError> {
    let port = resolve_opencode_catalog_port()?;
    Ok(format!("http://127.0.0.1:{port}"))
}

pub(crate) fn ensure_opencode_account_endpoint(
    owner_user_id: &str,
    account_profile: &str,
    environment: BTreeMap<String, String>,
) -> Result<String, DaemonError> {
    let key = format!("{owner_user_id}\0{account_profile}");
    let endpoints = OPENCODE_ACCOUNT_ENDPOINTS.get_or_init(|| Mutex::new(BTreeMap::new()));
    let mut endpoints = endpoints
        .lock()
        .map_err(|error| DaemonError::LocalTransport {
            operation: "ensure_opencode_account_endpoint",
            message: error.to_string(),
        })?;
    if let Some(existing) = endpoints.get_mut(&key) {
        if endpoint_is_healthy(&existing.endpoint) {
            return Ok(existing.endpoint.clone());
        }
        let _ = crate::runtime::process_health::terminate_process_tree(existing.child.id());
        let _ = existing.child.wait();
        endpoints.remove(&key);
    }

    let request = crate::provider::LaunchProviderRequest::new(
        "provider-account",
        "opencode",
        "opencode",
        account_profile,
        "default",
    )
    .with_owner_user_id(owner_user_id)
    .with_provider_account_env(environment);
    let launch = crate::provider::apply_managed_provider_isolation(
        super::plan_opencode_launch(Some(&request))?,
        &request,
    )?;
    let endpoint =
        launch
            .structured_endpoint
            .clone()
            .ok_or_else(|| DaemonError::LocalTransport {
                operation: "ensure_opencode_account_endpoint",
                message: "OpenCode account launch did not expose an endpoint".to_string(),
            })?;
    let mut command = crate::provider::command_from_provider_launch(launch)?;
    command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    for name in crate::account_profile::provider_auth_env_vars("opencode") {
        command.env_remove(name);
    }
    let mut child = command
        .spawn()
        .map_err(|error| DaemonError::LocalTransport {
            operation: "ensure_opencode_account_endpoint",
            message: format!("failed to start profile-specific OpenCode server: {error}"),
        })?;
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if endpoint_is_healthy(&endpoint) {
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
                operation: "ensure_opencode_account_endpoint",
                message: format!("failed to poll profile-specific OpenCode server: {error}"),
            })?
        {
            return Err(DaemonError::LocalTransport {
                operation: "ensure_opencode_account_endpoint",
                message: format!("profile-specific OpenCode server exited early: {status}"),
            });
        }
        if Instant::now() >= deadline {
            let _ = crate::runtime::process_health::terminate_process_tree(child.id());
            let _ = child.wait();
            return Err(DaemonError::LocalTransport {
                operation: "ensure_opencode_account_endpoint",
                message: "timed out waiting for profile-specific OpenCode server".to_string(),
            });
        }
        sleep(Duration::from_millis(100));
    }
}

pub(crate) fn invalidate_opencode_account_endpoint(owner_user_id: &str, account_profile: &str) {
    let Some(endpoints) = OPENCODE_ACCOUNT_ENDPOINTS.get() else {
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

pub(crate) fn shutdown_opencode_account_endpoints() {
    let Some(endpoints) = OPENCODE_ACCOUNT_ENDPOINTS.get() else {
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

fn endpoint_is_healthy(base_url: &str) -> bool {
    OpenCodeClient::new("catalog", base_url)
        .and_then(|client| client.check_health())
        .is_ok()
}
