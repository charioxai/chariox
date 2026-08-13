//! Codex runtime endpoint readiness and thread lifecycle binding.

use std::thread::sleep;
use std::time::{Duration, Instant};

use crate::error::DaemonError;
use crate::provider::{AgentEndpointMode, CodexRunSelection, RuntimeProviderRun};

use super::super::codex_client::{codex_endpoint_is_healthy, CODEX_ENDPOINT_STARTUP_TIMEOUT};
use super::run_config::{codex_client_for_run, normalize_codex_model};
use super::{CodexRuntimeBinding, CodexRuntimeState};

pub fn initialize_codex_runtime(
    run: &RuntimeProviderRun,
) -> Result<CodexRuntimeBinding, DaemonError> {
    let endpoint = run
        .structured_endpoint()
        .ok_or_else(|| DaemonError::ProviderProtocol {
            provider_run_id: run.id().to_string(),
            operation: "codex_endpoint_missing",
            message: "codex run did not expose a structured endpoint".to_string(),
        })?
        .to_string();
    let deadline = Instant::now() + CODEX_ENDPOINT_STARTUP_TIMEOUT;
    while !codex_endpoint_is_healthy(&endpoint) {
        if Instant::now() >= deadline {
            return Err(DaemonError::ProviderProtocol {
                provider_run_id: run.id().to_string(),
                operation: "codex_endpoint_unhealthy",
                message: format!(
                    "timed out waiting for Codex app-server to become healthy at `{endpoint}`"
                ),
            });
        }
        sleep(Duration::from_millis(100));
    }
    let client = codex_client_for_run(run, &endpoint, None)?;
    let socket = client.connect_initialized()?;
    let next_request_id = 1;
    let resumable_thread_id = run.resume_state().codex_thread_id().map(str::to_string);
    let (state, selection) = match resumable_thread_id {
        Some(thread_id) if run.endpoint_mode() == AgentEndpointMode::External => {
            crate::logging::info_with_fields(
                "daemon.provider.codex",
                "binding native codex thread without resume",
                serde_json::json!({
                    "provider_run_id": run.id(),
                    "thread_id": thread_id,
                }),
            );
            (
                CodexRuntimeState::new(endpoint, thread_id, socket, next_request_id),
                CodexRunSelection {
                    model: normalize_codex_model(run.model()),
                    variant: run.variant().map(str::to_string),
                },
            )
        }
        pending_thread_id => {
            if let Some(thread_id) = pending_thread_id.as_deref() {
                crate::logging::info_with_fields(
                    "daemon.provider.codex",
                    "deferring codex thread resume until first prompt",
                    serde_json::json!({
                        "provider_run_id": run.id(),
                        "thread_id": thread_id,
                    }),
                );
            } else {
                crate::logging::info_with_fields(
                    "daemon.provider.codex",
                    "deferring codex thread start until first prompt",
                    serde_json::json!({
                        "provider_run_id": run.id(),
                    }),
                );
            }
            (
                CodexRuntimeState::pending(endpoint, pending_thread_id, socket, next_request_id),
                CodexRunSelection {
                    model: normalize_codex_model(run.model()),
                    variant: run.variant().map(str::to_string),
                },
            )
        }
    };
    Ok(CodexRuntimeBinding { state, selection })
}

#[cfg(test)]
fn is_codex_mcp_handshake_timeout(error: &DaemonError) -> bool {
    let DaemonError::ProviderProtocol {
        operation, message, ..
    } = error
    else {
        return false;
    };

    matches!(*operation, "thread/start" | "thread/resume")
        && message.contains("required MCP servers failed to initialize")
        && message.contains("timed out handshaking with MCP server")
}

#[cfg(test)]
mod tests {
    use crate::error::DaemonError;

    use super::is_codex_mcp_handshake_timeout;

    #[test]
    fn classifies_codex_mcp_handshake_timeout_as_retryable() {
        let error = DaemonError::ProviderProtocol {
            provider_run_id: "provider-run-1".to_string(),
            operation: "thread/start",
            message: "error creating thread: Fatal error: Failed to initialize session: required MCP servers failed to initialize: chariox: timed out handshaking with MCP server after 30s"
                .to_string(),
        };

        assert!(is_codex_mcp_handshake_timeout(&error));
    }

    #[test]
    fn does_not_retry_stale_codex_thread_errors() {
        let error = DaemonError::ProviderProtocol {
            provider_run_id: "provider-run-1".to_string(),
            operation: "thread/resume",
            message: "no rollout found for thread".to_string(),
        };

        assert!(!is_codex_mcp_handshake_timeout(&error));
    }
}
