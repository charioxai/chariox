use rand::distributions::{Alphanumeric, DistString};

use crate::agent::AgentInstance;
use crate::config::DaemonConfig;
use crate::error::DaemonError;
use crate::provider::{LaunchProviderRequest, ProviderResumeState, RuntimeProviderRun};

pub(super) fn default_provider_env_remove(config: &DaemonConfig) -> Vec<String> {
    crate::secret::RuntimeSecretService::credential_env_names_from(&config.user_config.credentials)
        .into_iter()
        .collect()
}

pub(crate) fn sanitize_resume_state_for_launch(
    request: &LaunchProviderRequest,
    agent: &AgentInstance,
) -> ProviderResumeState {
    let resume_state = agent.provider_resume_state().clone();
    let requested_variant = request
        .variant
        .as_deref()
        .filter(|value| !value.trim().is_empty());
    let agent_variant = agent.effort().filter(|value| !value.trim().is_empty());
    let requested_model =
        normalize_resume_model_for_adapter(&request.adapter_key, request.model.as_str());
    let agent_model = agent
        .model()
        .map(|model| normalize_resume_model_for_adapter(&request.adapter_key, model));
    let model_or_variant_changed = agent_model.as_deref() != Some(requested_model.as_str())
        || agent_variant != requested_variant;
    if !model_or_variant_changed {
        return resume_state;
    }

    match request.adapter_key.as_str() {
        "opencode" => resume_state.without_opencode_session_id(),
        "codex" => resume_state.without_codex_thread_id(),
        "claude" => resume_state.without_claude_session_id(),
        _ => resume_state,
    }
}

pub(crate) fn failed_codex_resume_state_replacement(
    run: &RuntimeProviderRun,
    error: &DaemonError,
) -> Option<ProviderResumeState> {
    if run.adapter_key() != "codex" || run.resume_state().codex_thread_id().is_none() {
        return None;
    }
    let DaemonError::ProviderProtocol { operation, .. } = error else {
        return None;
    };
    if *operation != "codex_thread_resume" {
        return None;
    }
    Some(run.resume_state().without_codex_thread_id())
}

pub(crate) fn generate_runtime_mcp_auth_token() -> String {
    Alphanumeric.sample_string(&mut rand::thread_rng(), 32)
}

fn normalize_resume_model_for_adapter(adapter_key: &str, model: &str) -> String {
    let trimmed = model.trim();
    if adapter_key == "codex" {
        trimmed
            .strip_prefix("codex/")
            .unwrap_or(trimmed)
            .to_string()
    } else {
        trimmed.to_string()
    }
}
