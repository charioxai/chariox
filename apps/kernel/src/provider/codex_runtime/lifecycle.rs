//! Codex runtime endpoint readiness and thread lifecycle binding.

use std::thread::sleep;
use std::time::{Duration, Instant};

use crate::error::DaemonError;
use crate::provider::{
    AgentEndpointMode, CodexRunSelection, ProviderResumeState, RuntimeProviderRun,
};

use super::super::codex_client::codex_endpoint_is_healthy;
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
            let resume = client.thread_resume(
                &mut socket,
                &mut next_request_id,
                &thread_id,
                cwd.as_deref(),
                model.as_deref(),
                run.write_access_mode(),
                run.execution_mode(),
                run.permission_level(),
            );
            match resume {
                Ok(thread) => (
                    thread.thread.id,
                    CodexRunSelection {
                        model: Some(format!("codex/{}", thread.model)),
                        variant: thread.reasoning_effort,
                    },
                ),
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
            let thread = client.thread_start(
                &mut socket,
                &mut next_request_id,
                cwd.as_deref(),
                model.as_deref(),
                run.write_access_mode(),
                run.execution_mode(),
                run.permission_level(),
            )?;
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
        state: CodexRuntimeState::new(endpoint, thread_id.clone(), socket, next_request_id),
        selection,
        resume_state: ProviderResumeState::from_codex_thread_id(thread_id),
    })
}
