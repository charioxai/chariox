//! OpenCode native permission request bridging into runtime interactions.

use std::sync::Arc;

use crate::error::DaemonError;
use crate::provider::opencode_client::OpenCodePermissionRequest;
use crate::provider::run_actor::{
    ProviderNativeInteractionBridge, ProviderNativeInteractionResolution,
};
use crate::provider::RuntimeProviderRun;

use super::{OpenCodeClient, OpenCodeRuntimeState};

pub(super) fn handle_permission_request(
    run: &RuntimeProviderRun,
    state: &OpenCodeRuntimeState,
    provider_run_id: &str,
    native_interaction_bridge: Option<Arc<dyn ProviderNativeInteractionBridge>>,
    request: &OpenCodePermissionRequest,
) -> Result<(), DaemonError> {
    crate::logging::debug_with_fields(
        "provider.opencode.permission",
        "received opencode permission request",
        serde_json::json!({
            "provider_run_id": provider_run_id,
            "session_id": state.session_id(),
            "request_id": request.id,
            "permission": request.permission,
            "tool": request.tool,
            "command": request.command,
            "cwd": request.cwd,
            "patterns": request.patterns,
        }),
    );
    let response = resolve_permission_interaction(run, native_interaction_bridge, request)?;
    crate::logging::debug_with_fields(
        "provider.opencode.permission",
        "replying to opencode permission request",
        serde_json::json!({
            "provider_run_id": provider_run_id,
            "session_id": state.session_id(),
            "request_id": request.id,
            "response": response,
        }),
    );
    let client = OpenCodeClient::new(provider_run_id, state.base_url())?;
    client.reply_permission(state.session_id(), &request.id, response)
}

fn resolve_permission_interaction(
    run: &RuntimeProviderRun,
    native_interaction_bridge: Option<Arc<dyn ProviderNativeInteractionBridge>>,
    request: &OpenCodePermissionRequest,
) -> Result<&'static str, DaemonError> {
    let Some(bridge) = native_interaction_bridge else {
        return Ok("reject");
    };
    let Some(agent_id) = run.agent_instance_id() else {
        return Ok("reject");
    };
    let level = match request.permission.as_str() {
        "edit" | "task" => crate::session::RuntimeInteractionLevel::Critical,
        _ => crate::session::RuntimeInteractionLevel::Warning,
    };
    let title = Some(format!(
        "OpenCode {} approval required",
        humanize_permission_name(&request.permission)
    ));
    let mut message = format!(
        "Approve OpenCode {} request?",
        humanize_permission_name(&request.permission).to_lowercase()
    );
    if let Some(tool) = request
        .tool
        .as_deref()
        .filter(|value| !value.trim().is_empty())
    {
        message.push_str(&format!("\n\ntool: {tool}"));
    }
    if let Some(command) = request
        .command
        .as_deref()
        .filter(|value| !value.trim().is_empty())
    {
        message.push_str(&format!("\n\ncommand: {command}"));
    }
    if let Some(cwd) = request
        .cwd
        .as_deref()
        .filter(|value| !value.trim().is_empty())
    {
        message.push_str(&format!("\n\ncwd: {cwd}"));
    }
    if let Some(reason) = request
        .reason
        .as_deref()
        .filter(|value| !value.trim().is_empty())
    {
        message.push_str(&format!("\n\nreason: {reason}"));
    }
    if !request.patterns.is_empty() {
        message.push_str(&format!("\n\npatterns: {}", request.patterns.join(", ")));
    }
    let interaction = crate::session::RuntimeInteraction::new(
        format!("opencode-permission-{}", request.id),
        agent_id,
        crate::session::RuntimeInteractionKind::Permission,
        level,
        title,
        message,
        vec![
            crate::session::RuntimeInteractionChoice::new(
                "allow_once",
                "Allow once",
                "allow_once",
                Some(crate::session::RuntimeInteractionChoiceStyle::Primary),
            ),
            crate::session::RuntimeInteractionChoice::new(
                "allow_session",
                "Allow for session",
                "allow_session",
                Some(crate::session::RuntimeInteractionChoiceStyle::Secondary),
            ),
            crate::session::RuntimeInteractionChoice::new(
                "deny",
                "Deny",
                "deny",
                Some(crate::session::RuntimeInteractionChoiceStyle::Danger),
            ),
        ],
        None,
        None,
        None,
    );
    crate::logging::debug_with_fields(
        "provider.opencode.permission",
        "bridging opencode permission request to runtime interaction",
        serde_json::json!({
            "provider_run_id": run.id(),
            "session_id": run.session_id(),
            "agent_id": agent_id,
            "request_id": request.id,
            "interaction_id": interaction.id(),
            "permission": request.permission,
        }),
    );
    let resolution = bridge.request_blocking(run.session_id(), interaction)?;
    crate::logging::debug_with_fields(
        "provider.opencode.permission",
        "runtime interaction resolved for opencode permission request",
        serde_json::json!({
            "provider_run_id": run.id(),
            "session_id": run.session_id(),
            "request_id": request.id,
            "status": resolution.status,
            "choice_id": resolution.choice_id,
            "reply": resolution.reply,
        }),
    );
    Ok(map_permission_resolution_to_opencode_response(&resolution))
}

fn map_permission_resolution_to_opencode_response(
    resolution: &ProviderNativeInteractionResolution,
) -> &'static str {
    match resolution.choice_id.as_deref() {
        Some("allow_once") => "once",
        Some("allow_session") => "always",
        Some("deny") | None => "reject",
        Some(_) => "reject",
    }
}

fn humanize_permission_name(permission: &str) -> String {
    match permission {
        "bash" => "Bash".to_string(),
        "edit" => "Edit".to_string(),
        "task" => "Task".to_string(),
        other => {
            let mut chars = other.chars();
            match chars.next() {
                Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
                None => "Permission".to_string(),
            }
        }
    }
}
