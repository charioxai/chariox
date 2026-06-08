use rand::distributions::{Alphanumeric, DistString};

use crate::agent::AgentInstance;
use crate::config::DaemonConfig;
use crate::error::DaemonError;
use crate::mcp::ArrobaMcpServerConfig;
use crate::provider::{LaunchProviderRequest, ProviderResumeState, RuntimeProviderRun};
use crate::session::RuntimeSession;

pub(super) fn default_provider_env_remove(config: &DaemonConfig) -> Vec<String> {
    let credentials = crate::credential::load_user_credentials().unwrap_or_default();
    let _ = config;
    crate::secret::RuntimeSecretService::credential_env_names_from(&credentials)
        .into_iter()
        .collect()
}

pub(crate) fn resolve_mcp_credentials_for_launch(
    config: &DaemonConfig,
    servers: Vec<ArrobaMcpServerConfig>,
) -> Result<Vec<ArrobaMcpServerConfig>, DaemonError> {
    if servers.is_empty() {
        return Ok(servers);
    }
    let credentials = crate::credential::load_user_credentials()?;
    let service = crate::secret::RuntimeSecretService::with_vault_config(
        credentials,
        &config.user_config.credential_vault,
    )?;
    servers
        .into_iter()
        .map(|server| server.resolve_credential_bindings(&service))
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

pub(crate) fn granted_mcp_servers_for_agent_launch(
    operation: &'static str,
    session: &RuntimeSession,
    agent: &AgentInstance,
) -> Result<Vec<ArrobaMcpServerConfig>, DaemonError> {
    let mcp_grants = agent.mcp_grants();
    if mcp_grants.is_empty() {
        return Ok(Vec::new());
    }
    let roots = crate::mcp::ArrobaMcpRegistry::user_root()
        .map(|root| vec![root])
        .unwrap_or_default();
    let registry = crate::mcp::ArrobaMcpRegistry::new(roots);
    let mut servers = Vec::new();
    for grant in mcp_grants {
        let Some(server) = registry.get(&grant)? else {
            crate::logging::warn_with_fields(
                "daemon.provider",
                "skipping missing MCP extension grant during provider launch",
                serde_json::json!({
                    "operation": operation,
                    "session_id": session.id(),
                    "agent_id": agent.id(),
                    "agent_ref": agent.agent_ref(),
                    "mcp": grant,
                }),
            );
            continue;
        };
        if server.enabled {
            servers.push(server);
        }
    }
    Ok(servers)
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
