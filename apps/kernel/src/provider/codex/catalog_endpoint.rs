use std::process::{Command, Stdio};
use std::thread::sleep;
use std::time::{Duration, Instant};

use crate::error::DaemonError;
use crate::provider::{AgentEndpointMode, ProviderCatalogEndpoint};

use super::super::codex_client::{codex_endpoint_is_healthy, CODEX_ENDPOINT_STARTUP_TIMEOUT};
use super::ports::{clear_codex_catalog_port_if_unset, resolve_codex_catalog_port};

pub fn codex_catalog_endpoint() -> Result<String, DaemonError> {
    let _guard = crate::env_lock::lock();
    codex_catalog_endpoint_unlocked()
}

fn codex_catalog_endpoint_unlocked() -> Result<String, DaemonError> {
    let port = resolve_codex_catalog_port()?;
    Ok(format!("ws://127.0.0.1:{port}"))
}

pub fn ensure_codex_catalog_endpoint() -> Result<String, DaemonError> {
    lease_codex_catalog_endpoint().map(ProviderCatalogEndpoint::into_persistent_endpoint)
}

pub(crate) fn lease_codex_catalog_endpoint() -> Result<ProviderCatalogEndpoint, DaemonError> {
    let launch = super::plan_codex_launch(None)?;
    let endpoint =
        launch
            .structured_endpoint
            .clone()
            .ok_or_else(|| DaemonError::LocalTransport {
                operation: "ensure_codex_catalog_endpoint",
                message: "codex launch did not expose a structured endpoint".to_string(),
            })?;
    if codex_endpoint_is_healthy(&endpoint) {
        return Ok(ProviderCatalogEndpoint::existing(endpoint));
    }
    if launch.endpoint_mode == AgentEndpointMode::External {
        return Err(DaemonError::LocalTransport {
            operation: "ensure_codex_catalog_endpoint",
            message: format!("configured Codex endpoint `{endpoint}` is not reachable"),
        });
    }

    let program = launch
        .pty_program
        .clone()
        .ok_or_else(|| DaemonError::LocalTransport {
            operation: "ensure_codex_catalog_endpoint",
            message: "codex launch did not provide an executable".to_string(),
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
        clear_codex_catalog_port_if_unset();
        DaemonError::LocalTransport {
            operation: "ensure_codex_catalog_endpoint",
            message: format!("failed to start Codex app-server: {error}"),
        }
    })?;

    let deadline = Instant::now() + CODEX_ENDPOINT_STARTUP_TIMEOUT;
    loop {
        if codex_endpoint_is_healthy(&endpoint) {
            return Ok(ProviderCatalogEndpoint::managed(endpoint, child));
        }
        if let Some(status) = child
            .try_wait()
            .map_err(|error| DaemonError::LocalTransport {
                operation: "ensure_codex_catalog_endpoint",
                message: format!("failed to poll Codex app-server startup: {error}"),
            })?
        {
            clear_codex_catalog_port_if_unset();
            return Err(DaemonError::LocalTransport {
                operation: "ensure_codex_catalog_endpoint",
                message: format!("Codex app-server exited before becoming healthy: {status}"),
            });
        }
        if Instant::now() >= deadline {
            clear_codex_catalog_port_if_unset();
            return Err(DaemonError::LocalTransport {
                operation: "ensure_codex_catalog_endpoint",
                message: format!(
                    "timed out waiting for Codex app-server to become healthy at `{endpoint}`"
                ),
            });
        }
        sleep(Duration::from_millis(100));
    }
}
