use std::process::{Command, Stdio};
use std::thread::sleep;
use std::time::{Duration, Instant};

use crate::error::DaemonError;
use crate::provider::{AgentEndpointMode, OpenCodeClient, ProviderCatalogEndpoint};

use super::ports::{clear_opencode_catalog_port_if_unset, resolve_opencode_catalog_port};

pub fn opencode_catalog_endpoint() -> Result<String, DaemonError> {
    let _guard = crate::env_lock::lock();
    opencode_catalog_endpoint_unlocked()
}

fn opencode_catalog_endpoint_unlocked() -> Result<String, DaemonError> {
    let port = resolve_opencode_catalog_port()?;
    Ok(format!("http://127.0.0.1:{port}"))
}

pub fn ensure_opencode_catalog_endpoint() -> Result<String, DaemonError> {
    lease_opencode_catalog_endpoint().map(ProviderCatalogEndpoint::into_persistent_endpoint)
}

pub(crate) fn lease_opencode_catalog_endpoint() -> Result<ProviderCatalogEndpoint, DaemonError> {
    let launch = super::plan_opencode_launch(None)?;
    let endpoint =
        launch
            .structured_endpoint
            .clone()
            .ok_or_else(|| DaemonError::LocalTransport {
                operation: "ensure_opencode_catalog_endpoint",
                message: "opencode launch did not expose a structured endpoint".to_string(),
            })?;
    if endpoint_is_healthy(&endpoint) {
        return Ok(ProviderCatalogEndpoint::existing(endpoint));
    }
    if launch.endpoint_mode == AgentEndpointMode::External {
        return Err(DaemonError::LocalTransport {
            operation: "ensure_opencode_catalog_endpoint",
            message: format!("configured OpenCode endpoint `{endpoint}` is not reachable"),
        });
    }

    let program = launch
        .pty_program
        .clone()
        .ok_or_else(|| DaemonError::LocalTransport {
            operation: "ensure_opencode_catalog_endpoint",
            message: "opencode launch did not provide an executable".to_string(),
        })?;
    let mut command = Command::new(program);
    command
        .args(&launch.pty_args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    if let Some(working_directory) = launch.working_directory.as_ref() {
        command.current_dir(working_directory);
    }
    let mut child = command.spawn().map_err(|error| {
        clear_opencode_catalog_port_if_unset();
        DaemonError::LocalTransport {
            operation: "ensure_opencode_catalog_endpoint",
            message: format!("failed to start OpenCode server: {error}"),
        }
    })?;

    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if endpoint_is_healthy(&endpoint) {
            return Ok(ProviderCatalogEndpoint::managed(endpoint, child));
        }
        if let Some(status) = child
            .try_wait()
            .map_err(|error| DaemonError::LocalTransport {
                operation: "ensure_opencode_catalog_endpoint",
                message: format!("failed to poll OpenCode server startup: {error}"),
            })?
        {
            clear_opencode_catalog_port_if_unset();
            return Err(DaemonError::LocalTransport {
                operation: "ensure_opencode_catalog_endpoint",
                message: format!("OpenCode server exited before becoming healthy: {status}"),
            });
        }
        if Instant::now() >= deadline {
            clear_opencode_catalog_port_if_unset();
            return Err(DaemonError::LocalTransport {
                operation: "ensure_opencode_catalog_endpoint",
                message: format!(
                    "timed out waiting for OpenCode server to become healthy at `{endpoint}`"
                ),
            });
        }
        sleep(Duration::from_millis(100));
    }
}

fn endpoint_is_healthy(base_url: &str) -> bool {
    OpenCodeClient::new("catalog", base_url)
        .and_then(|client| client.check_health())
        .is_ok()
}
