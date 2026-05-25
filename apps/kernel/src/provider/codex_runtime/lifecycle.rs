//! Codex runtime endpoint readiness and thread lifecycle binding.

use std::thread::sleep;
use std::time::{Duration, Instant};

use crate::error::DaemonError;
use crate::provider::{AgentEndpointMode, CodexRunSelection, RuntimeProviderRun};

use super::super::codex_client::{
    codex_endpoint_is_healthy, CodexClient, CodexSocket, CodexThreadStartResponse,
};
use super::run_config::{codex_client_for_run, normalize_codex_model};
use super::{CodexRuntimeBinding, CodexRuntimeState};

const CODEX_MCP_THREAD_INIT_RETRY_TIMEOUT: Duration = Duration::from_secs(150);
const CODEX_MCP_THREAD_INIT_RETRY_INTERVAL: Duration = Duration::from_millis(500);

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
    let deadline = Instant::now() + Duration::from_secs(10);
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
    let mut socket = client.connect_initialized()?;
    let mut next_request_id = 1;
    let cwd = run
        .working_directory()
        .map(|path| path.to_string_lossy().to_string());
    let model = normalize_codex_model(run.model());
    let resumable_thread_id = run.resume_state().codex_thread_id().map(str::to_string);
    let (thread_id, selection) = match resumable_thread_id {
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
                thread_id,
                CodexRunSelection {
                    model: Some(format!("codex/{}", run.model())),
                    variant: run.variant().map(str::to_string),
                },
            )
        }
        Some(thread_id) => {
            crate::logging::info_with_fields(
                "daemon.provider.codex",
                "reusing codex thread",
                serde_json::json!({
                    "provider_run_id": run.id(),
                    "thread_id": thread_id,
                }),
            );
            let resume = resume_codex_thread_with_mcp_retry(
                &client,
                socket,
                next_request_id,
                run,
                &thread_id,
                cwd.as_deref(),
                model.as_deref(),
            );
            match resume {
                Ok((thread, resumed_socket, resumed_next_request_id)) => {
                    socket = resumed_socket;
                    next_request_id = resumed_next_request_id;
                    (
                        thread.thread.id,
                        CodexRunSelection {
                            model: Some(format!("codex/{}", thread.model)),
                            variant: thread.reasoning_effort,
                        },
                    )
                }
                Err(error) => {
                    crate::logging::error_with_fields(
                        "daemon.provider.codex",
                        "codex thread resume failed",
                        serde_json::json!({
                            "provider_run_id": run.id(),
                            "thread_id": thread_id,
                            "error": error.to_string(),
                        }),
                    );
                    return Err(DaemonError::ProviderProtocol {
                        provider_run_id: run.id().to_string(),
                        operation: "codex_thread_resume",
                        message: format!(
                            "Codex could not resume thread `{thread_id}`: {error}. Refusing to start a blank replacement thread."
                        ),
                    });
                }
            }
        }
        None => {
            let (thread, started_socket, started_next_request_id) =
                start_codex_thread_with_mcp_retry(
                    &client,
                    socket,
                    next_request_id,
                    run,
                    cwd.as_deref(),
                    model.as_deref(),
                )?;
            socket = started_socket;
            next_request_id = started_next_request_id;
            (
                thread.thread.id,
                CodexRunSelection {
                    model: Some(format!("codex/{}", thread.model)),
                    variant: thread.reasoning_effort,
                },
            )
        }
    };
    Ok(CodexRuntimeBinding {
        state: CodexRuntimeState::new(endpoint, thread_id, socket, next_request_id),
        selection,
    })
}

fn start_codex_thread_with_mcp_retry(
    client: &CodexClient,
    mut socket: CodexSocket,
    mut next_request_id: u64,
    run: &RuntimeProviderRun,
    cwd: Option<&str>,
    model: Option<&str>,
) -> Result<(CodexThreadStartResponse, CodexSocket, u64), DaemonError> {
    let deadline = Instant::now() + CODEX_MCP_THREAD_INIT_RETRY_TIMEOUT;
    loop {
        match client.thread_start(
            &mut socket,
            &mut next_request_id,
            cwd,
            model,
            run.write_access_mode(),
            run.execution_mode(),
            run.permission_level(),
        ) {
            Ok(thread) => return Ok((thread, socket, next_request_id)),
            Err(error) if is_codex_mcp_handshake_timeout(&error) && Instant::now() < deadline => {
                crate::logging::warn_with_fields(
                    "daemon.provider.codex",
                    "retrying codex thread/start after MCP handshake timeout",
                    serde_json::json!({
                        "provider_run_id": run.id(),
                        "error": error.to_string(),
                    }),
                );
                sleep(CODEX_MCP_THREAD_INIT_RETRY_INTERVAL);
                socket = client.connect_initialized()?;
                next_request_id = 1;
            }
            Err(error) => return Err(error),
        }
    }
}

fn resume_codex_thread_with_mcp_retry(
    client: &CodexClient,
    mut socket: CodexSocket,
    mut next_request_id: u64,
    run: &RuntimeProviderRun,
    thread_id: &str,
    cwd: Option<&str>,
    model: Option<&str>,
) -> Result<(CodexThreadStartResponse, CodexSocket, u64), DaemonError> {
    let deadline = Instant::now() + CODEX_MCP_THREAD_INIT_RETRY_TIMEOUT;
    loop {
        match client.thread_resume(
            &mut socket,
            &mut next_request_id,
            thread_id,
            cwd,
            model,
            run.write_access_mode(),
            run.execution_mode(),
            run.permission_level(),
        ) {
            Ok(thread) => return Ok((thread, socket, next_request_id)),
            Err(error) if is_codex_mcp_handshake_timeout(&error) && Instant::now() < deadline => {
                crate::logging::warn_with_fields(
                    "daemon.provider.codex",
                    "retrying codex thread/resume after MCP handshake timeout",
                    serde_json::json!({
                        "provider_run_id": run.id(),
                        "thread_id": thread_id,
                        "error": error.to_string(),
                    }),
                );
                sleep(CODEX_MCP_THREAD_INIT_RETRY_INTERVAL);
                socket = client.connect_initialized()?;
                next_request_id = 1;
            }
            Err(error) => return Err(error),
        }
    }
}

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
            message: "error creating thread: Fatal error: Failed to initialize session: required MCP servers failed to initialize: arroba: timed out handshaking with MCP server after 30s"
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
