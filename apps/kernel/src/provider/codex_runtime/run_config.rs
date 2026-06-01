//! Codex client construction and run option normalization.

use std::sync::Arc;

use crate::error::DaemonError;
use crate::provider::{CodexClient, ProviderNativeInteractionBridge, RuntimeProviderRun};

pub(super) fn codex_client_for_run(
    run: &RuntimeProviderRun,
    endpoint: &str,
    native_interaction_bridge: Option<Arc<dyn ProviderNativeInteractionBridge>>,
) -> Result<CodexClient, DaemonError> {
    Ok(CodexClient::new(run.id(), endpoint)?
        .with_runtime_context(Some(run.session_id()), run.agent_instance_id())
        .with_runtime_mcp_binding(run.runtime_mcp_server_url(), run.runtime_mcp_auth_token())
        .with_native_interaction_bridge(native_interaction_bridge)
        .with_mcp_servers(run.mcp_servers())
        .with_write_access_mode(run.write_access_mode())
        .with_workspace_live_sync_roots(run.workspace_live_sync_roots()))
}

pub(super) fn normalize_codex_model(model: &str) -> Option<String> {
    let model = model.trim();
    if model.is_empty() || model == "default" {
        return None;
    }
    Some(model.strip_prefix("codex/").unwrap_or(model).to_string())
}

pub(super) fn normalize_variant(variant: Option<&str>) -> Option<String> {
    variant
        .map(str::trim)
        .filter(|value| !value.is_empty() && *value != "default")
        .map(str::to_string)
}
