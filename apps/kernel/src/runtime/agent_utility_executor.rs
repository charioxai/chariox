use std::time::{SystemTime, UNIX_EPOCH};

use rand::RngCore;
use tokio::time::{sleep, Duration, Instant as TokioInstant};

use crate::config::UserArchiveHistoryConfig;
use crate::error::DaemonError;
use crate::local::{
    AgentUtilityInput, AgentUtilityKind, AgentUtilityOutput, AgentUtilityResult,
    GenerateWorkspaceCommitMessageRequest, LocalDaemonRequest, LocalDaemonResponse,
    RunAgentUtilityRequest, SemanticHistorySearchUtilityInput, WorkspaceCommitMessageUtilityInput,
};
use crate::provider::{
    run_codex_utility_prompt, run_opencode_utility_prompt, ProviderRunState, RuntimeProviderRun,
};
use crate::runtime::history_requests::{
    knn_semantic_history_search, semantic_search_request_from_utility_input,
};
use crate::runtime::projection::DaemonConfigProjectionStore;
use crate::runtime::semantic_history_utility::{
    parse_semantic_history_search_utility_output, semantic_history_search_utility_prompt,
};
use crate::runtime::state::KernelRuntimeState;
use crate::runtime::workspace_commit_message_utility::workspace_commit_message_utility_prompt;

const AGENT_UTILITY_TIMEOUT: Duration = Duration::from_secs(120);
const AGENT_UTILITY_PROVIDER_READY_TIMEOUT: Duration = Duration::from_secs(60);

pub(crate) async fn execute_run_agent_utility_request(
    runtime_state: &KernelRuntimeState,
    config_projection: &DaemonConfigProjectionStore,
    request: RunAgentUtilityRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    let result =
        run_agent_utility(runtime_state, archive_config(config_projection), request).await?;
    Ok(LocalDaemonResponse::AgentUtilityCompleted { result })
}

pub(crate) async fn execute_generate_workspace_commit_message_request(
    runtime_state: &KernelRuntimeState,
    config_projection: &DaemonConfigProjectionStore,
    request: GenerateWorkspaceCommitMessageRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    let result = run_agent_utility(
        runtime_state,
        archive_config(config_projection),
        RunAgentUtilityRequest {
            session_id: request.session_id,
            agent_id: request.agent_id,
            kind: AgentUtilityKind::WorkspaceCommitMessage,
            input: AgentUtilityInput::WorkspaceCommitMessage(WorkspaceCommitMessageUtilityInput {
                workspace_id: request.workspace_id,
                worktree_id: request.worktree_id,
                compare_ref: request.compare_ref,
            }),
        },
    )
    .await?;
    let AgentUtilityOutput::WorkspaceCommitMessage { message } = result.output else {
        return Err(DaemonError::LocalTransport {
            operation: "generate workspace commit message",
            message: "workspace commit message utility returned unexpected output".to_string(),
        });
    };
    Ok(LocalDaemonResponse::WorkspaceCommitMessageGenerated { message })
}

pub(crate) async fn execute_agent_utility_request(
    runtime_state: &KernelRuntimeState,
    config_projection: &DaemonConfigProjectionStore,
    request: LocalDaemonRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    match request {
        LocalDaemonRequest::RunAgentUtility(request) => {
            execute_run_agent_utility_request(runtime_state, config_projection, request).await
        }
        LocalDaemonRequest::GenerateWorkspaceCommitMessage(request) => {
            execute_generate_workspace_commit_message_request(
                runtime_state,
                config_projection,
                request,
            )
            .await
        }
        _ => Err(DaemonError::LocalTransport {
            operation: "agent utility request",
            message: "unsupported agent utility request".to_string(),
        }),
    }
}

fn archive_config(config_projection: &DaemonConfigProjectionStore) -> UserArchiveHistoryConfig {
    config_projection.snapshot().user_config.history.archive
}

pub(crate) async fn run_agent_utility(
    runtime_state: &KernelRuntimeState,
    archive_config: UserArchiveHistoryConfig,
    request: RunAgentUtilityRequest,
) -> Result<AgentUtilityResult, DaemonError> {
    let (_agent, provider_run) = assert_agent_utility_can_run(
        runtime_state,
        &request.session_id,
        &request.agent_id,
        &request.kind,
    )
    .await?;
    let output = match (&request.kind, request.input) {
        (
            AgentUtilityKind::WorkspaceCommitMessage,
            AgentUtilityInput::WorkspaceCommitMessage(input),
        ) => AgentUtilityOutput::WorkspaceCommitMessage {
            message: run_workspace_commit_message_utility(provider_run, input).await?,
        },
        (
            AgentUtilityKind::SemanticHistorySearch,
            AgentUtilityInput::SemanticHistorySearch(input),
        ) => run_semantic_history_search_utility(archive_config, provider_run, input).await?,
        (kind, _) => {
            return Err(DaemonError::LocalTransport {
                operation: agent_utility_operation(kind),
                message: "agent utility input kind did not match requested utility".to_string(),
            })
        }
    };
    Ok(AgentUtilityResult {
        utility_run_id: format!("utility-{}", random_hex_id()),
        session_id: request.session_id,
        agent_id: request.agent_id,
        kind: request.kind,
        output,
        generated_at_ms: current_unix_ms(),
    })
}

async fn assert_agent_utility_can_run(
    runtime_state: &KernelRuntimeState,
    session_id: &str,
    agent_id: &str,
    kind: &AgentUtilityKind,
) -> Result<(crate::agent::AgentInstance, RuntimeProviderRun), DaemonError> {
    let started_at = TokioInstant::now();
    loop {
        let (agent, provider_run) = runtime_state
            .agent_utility_provider_run(session_id, agent_id, agent_utility_operation(kind))
            .await?;
        if provider_run.state() == ProviderRunState::Running {
            return Ok((agent, provider_run));
        }
        let state = provider_run.state();
        if state != ProviderRunState::Starting
            || started_at.elapsed() >= AGENT_UTILITY_PROVIDER_READY_TIMEOUT
        {
            return Err(DaemonError::LocalTransport {
                operation: agent_utility_operation(kind),
                message: format!("agent `{agent_id}` provider runtime is not running ({state:?})",),
            });
        }
        sleep(Duration::from_millis(250)).await;
    }
}

async fn run_workspace_commit_message_utility(
    provider_run: RuntimeProviderRun,
    input: WorkspaceCommitMessageUtilityInput,
) -> Result<String, DaemonError> {
    let prompt = workspace_commit_message_utility_prompt(&input)?;
    run_provider_utility_prompt(provider_run, prompt, "run workspace commit message utility").await
}

async fn run_semantic_history_search_utility(
    archive_config: UserArchiveHistoryConfig,
    provider_run: RuntimeProviderRun,
    input: SemanticHistorySearchUtilityInput,
) -> Result<AgentUtilityOutput, DaemonError> {
    let requested_limit = input.limit.unwrap_or(20).clamp(1, 50);
    let search_request = semantic_search_request_from_utility_input(&input);
    let (candidates, _next_cursor, unavailable_reason) =
        knn_semantic_history_search(archive_config, search_request, requested_limit).await?;
    if let Some(reason) = unavailable_reason {
        return Err(DaemonError::LocalTransport {
            operation: "run semantic history search utility",
            message: reason,
        });
    }
    let prompt = semantic_history_search_utility_prompt(&input, &candidates)?;
    let output =
        run_provider_utility_prompt(provider_run, prompt, "run semantic history search utility")
            .await?;
    let parsed = parse_semantic_history_search_utility_output(&output, &candidates)?;
    Ok(AgentUtilityOutput::SemanticHistorySearch {
        answer: parsed.answer,
        matches: parsed.matches,
    })
}

fn agent_utility_operation(kind: &AgentUtilityKind) -> &'static str {
    match kind {
        AgentUtilityKind::WorkspaceCommitMessage => "run workspace commit message utility",
        AgentUtilityKind::SemanticHistorySearch => "run semantic history search utility",
    }
}

async fn run_provider_utility_prompt(
    provider_run: RuntimeProviderRun,
    prompt: String,
    operation: &'static str,
) -> Result<String, DaemonError> {
    tokio::task::spawn_blocking(move || match provider_run.adapter_key() {
        "codex" => run_codex_utility_prompt(&provider_run, &prompt, AGENT_UTILITY_TIMEOUT),
        "opencode" => run_opencode_utility_prompt(&provider_run, &prompt, AGENT_UTILITY_TIMEOUT),
        adapter_key => Err(DaemonError::LocalTransport {
            operation,
            message: format!(
                "agent utility prompts are not supported for provider adapter `{adapter_key}`"
            ),
        }),
    })
    .await
    .map_err(|error| DaemonError::LocalTransport {
        operation,
        message: format!("agent utility prompt task failed: {error}"),
    })?
}

fn random_hex_id() -> String {
    let mut bytes = [0u8; 16];
    rand::thread_rng().fill_bytes(&mut bytes);
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn current_unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utility_operations_name_supported_kinds() {
        assert_eq!(
            agent_utility_operation(&AgentUtilityKind::WorkspaceCommitMessage),
            "run workspace commit message utility"
        );
        assert_eq!(
            agent_utility_operation(&AgentUtilityKind::SemanticHistorySearch),
            "run semantic history search utility"
        );
    }
}
