//! OpenCode native permission request bridging into runtime interactions.

use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use crate::error::DaemonError;
use crate::provider::opencode_client::OpenCodePermissionRequest;
use crate::provider::run_actor::{
    ProviderNativeInteractionBridge, ProviderNativeInteractionResolution,
};
use crate::provider::{OpenCodeClient, RuntimeProviderRun};

use super::OpenCodeRuntimeState;

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
    if request_touches_unfenced_workspace_live_sync_root(run, request) {
        crate::logging::warn_with_fields(
            "provider.opencode.permission",
            "rejecting opencode native write request for protected workspace live sync root",
            serde_json::json!({
                "provider_run_id": run.id(),
                "session_id": run.session_id(),
                "request_id": request.id,
                "permission": request.permission,
                "patterns": request.patterns,
                "cwd": request.cwd,
            }),
        );
        return Ok("reject");
    }
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

fn request_touches_unfenced_workspace_live_sync_root(
    run: &RuntimeProviderRun,
    request: &OpenCodePermissionRequest,
) -> bool {
    if !run.requires_workspace_live_sync() || crate::provider::workspace_write_fence_active(run) {
        return false;
    }
    if !matches!(
        request.permission.as_str(),
        "bash" | "edit" | "write" | "multiedit" | "apply_patch" | "external_directory"
    ) {
        return false;
    }
    let cwd = request.cwd.as_deref().map(Path::new);
    run.workspace_live_sync_roots().iter().any(|root| {
        request
            .command
            .as_deref()
            .is_some_and(|command| text_mentions_root_path(command, root))
            || request
                .patterns
                .iter()
                .any(|pattern| path_text_touches_root(pattern, root, cwd))
            || (request.patterns.is_empty()
                && request.permission != "bash"
                && request
                    .cwd
                    .as_deref()
                    .is_some_and(|cwd| path_text_touches_root(cwd, root, None)))
            || (request.permission == "bash"
                && request
                    .command
                    .as_deref()
                    .is_some_and(|command| bash_relative_write_touches_root(command, cwd, root)))
    })
}

fn path_text_touches_root(text: &str, root: &Path, cwd: Option<&Path>) -> bool {
    if text_mentions_root_path(text, root) {
        return true;
    }
    let trimmed = text
        .trim()
        .trim_matches('"')
        .trim_matches('\'')
        .trim_end_matches('*')
        .trim_end_matches('/');
    let path = Path::new(trimmed);
    let resolved = if path.is_absolute() {
        Some(path.to_path_buf())
    } else {
        cwd.filter(|cwd| cwd.is_absolute())
            .map(|cwd| cwd.join(path))
    };
    resolved
        .as_deref()
        .is_some_and(|path| normalize_path(path).starts_with(normalize_path(root)))
}

fn text_mentions_root_path(text: &str, root: &Path) -> bool {
    let root_text = root.to_string_lossy();
    let mut remaining = text;
    while let Some(index) = remaining.find(root_text.as_ref()) {
        let after_index = index + root_text.len();
        let after = remaining[after_index..].chars().next();
        if match after {
            None => true,
            Some(ch) => {
                ch == '/'
                    || ch.is_whitespace()
                    || matches!(ch, '"' | '\'' | '`' | ',' | ';' | '&' | '|')
            }
        } {
            return true;
        }
        remaining = &remaining[after_index..];
    }
    false
}

fn bash_relative_write_touches_root(command: &str, cwd: Option<&Path>, root: &Path) -> bool {
    let Some(cwd) = cwd else {
        return false;
    };
    if !path_text_touches_root(&cwd.display().to_string(), root, None) {
        return false;
    }
    let tokens = shell_like_tokens(command);
    tokens
        .iter()
        .enumerate()
        .any(|(index, token)| match redirect_target_token(token) {
            Some(target) => path_text_touches_root(target, root, Some(cwd)),
            None if is_redirect_token(token) => tokens
                .get(index + 1)
                .is_some_and(|target| path_text_touches_root(target, root, Some(cwd))),
            None if is_write_command(token) => write_command_operands_touch_root(
                token,
                tokens.get(index + 1..).unwrap_or(&[]),
                cwd,
                root,
            ),
            None => false,
        })
}

fn shell_like_tokens(command: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut quote = None;
    for ch in command.chars() {
        match quote {
            Some(active) if ch == active => quote = None,
            Some(_) => current.push(ch),
            None if ch == '\'' || ch == '"' => quote = Some(ch),
            None if ch.is_whitespace() || matches!(ch, ';' | '&' | '|') => {
                if !current.is_empty() {
                    tokens.push(std::mem::take(&mut current));
                }
            }
            None => current.push(ch),
        }
    }
    if !current.is_empty() {
        tokens.push(current);
    }
    tokens
}

fn is_redirect_token(token: &str) -> bool {
    matches!(
        token,
        ">" | ">>" | "<>" | ">|" | "1>" | "1>>" | "2>" | "2>>" | "&>"
    )
}

fn redirect_target_token(token: &str) -> Option<&str> {
    [">>", ">", "1>>", "1>", "2>>", "2>", "&>", ">|", "<>"]
        .iter()
        .find_map(|prefix| {
            token
                .strip_prefix(prefix)
                .filter(|target| !target.is_empty())
        })
}

fn is_write_command(token: &str) -> bool {
    matches!(
        token,
        "touch" | "mkdir" | "rm" | "rmdir" | "mv" | "cp" | "install" | "tee" | "sed"
    )
}

fn write_command_operands_touch_root(
    command: &str,
    operands: &[String],
    cwd: &Path,
    root: &Path,
) -> bool {
    let mut sed_in_place = command != "sed";
    for operand in operands {
        if operand == "--" {
            continue;
        }
        if command == "sed" && (operand == "-i" || operand.starts_with("-i")) {
            sed_in_place = true;
            continue;
        }
        if operand.starts_with('-') {
            continue;
        }
        if sed_in_place && path_text_touches_root(operand, root, Some(cwd)) {
            return true;
        }
    }
    false
}

fn normalize_path(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                normalized.pop();
            }
            _ => normalized.push(component.as_os_str()),
        }
    }
    normalized
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

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::path::PathBuf;

    use super::request_touches_unfenced_workspace_live_sync_root;
    use crate::provider::opencode_client::OpenCodePermissionRequest;
    use crate::provider::{
        AgentEndpointMode, LaunchProviderRequest, ProviderLaunchResult, RuntimeProviderRun,
    };

    fn request(
        permission: &str,
        command: Option<&str>,
        cwd: Option<&str>,
        patterns: &[&str],
    ) -> OpenCodePermissionRequest {
        OpenCodePermissionRequest {
            id: "permission-1".to_string(),
            session_id: "opencode-session-1".to_string(),
            permission: permission.to_string(),
            tool: Some(permission.to_string()),
            command: command.map(str::to_string),
            cwd: cwd.map(str::to_string),
            reason: None,
            patterns: patterns.iter().map(|value| value.to_string()).collect(),
        }
    }

    fn run(protected_root: &str, fenced: bool) -> RuntimeProviderRun {
        let mut pty_env = BTreeMap::new();
        if fenced {
            pty_env.insert(
                "ARROBA_WORKSPACE_WRITE_FENCE".to_string(),
                "macos-seatbelt".to_string(),
            );
        }
        let request =
            LaunchProviderRequest::new("session-1", "agent-1", "opencode", "opencode", "gpt-5.2")
                .with_workspace_live_sync_managed()
                .with_workspace_live_sync_roots(vec![PathBuf::from(protected_root)]);
        RuntimeProviderRun::new(
            "provider-run-1",
            &request,
            ProviderLaunchResult {
                endpoint_mode: AgentEndpointMode::Managed,
                process_label: "opencode:test".to_string(),
                pty_target: None,
                pty_program: None,
                pty_args: Vec::new(),
                pty_env,
                pty_env_remove: Vec::new(),
                working_directory: None,
                structured_endpoint: Some("http://127.0.0.1:1".to_string()),
            },
        )
    }

    #[test]
    fn unfenced_workspace_live_sync_rejects_native_bash_into_protected_root() {
        let run = run("/tmp/arroba-workspace", false);
        let request = request(
            "bash",
            Some("printf x > /tmp/arroba-workspace/src/main.rs"),
            None,
            &[],
        );

        assert!(request_touches_unfenced_workspace_live_sync_root(
            &run, &request
        ));
    }

    #[test]
    fn unfenced_workspace_live_sync_allows_native_bash_outside_protected_root() {
        let run = run("/tmp/arroba-workspace", false);
        let request = request(
            "bash",
            Some("printf x > /tmp/outside-repo/file.txt"),
            Some("/tmp/outside-repo"),
            &["/tmp/outside-repo/*"],
        );

        assert!(!request_touches_unfenced_workspace_live_sync_root(
            &run, &request
        ));
    }

    #[test]
    fn unfenced_workspace_live_sync_allows_explicit_outside_bash_from_protected_cwd() {
        let run = run("/tmp/arroba-workspace", false);
        let request = request(
            "bash",
            Some("printf x > /tmp/outside-repo/file.txt"),
            Some("/tmp/arroba-workspace"),
            &[],
        );

        assert!(!request_touches_unfenced_workspace_live_sync_root(
            &run, &request
        ));
    }

    #[test]
    fn unfenced_workspace_live_sync_rejects_relative_bash_write_from_protected_cwd() {
        let run = run("/tmp/arroba-workspace", false);
        let request = request(
            "bash",
            Some("printf x > src/main.rs"),
            Some("/tmp/arroba-workspace"),
            &[],
        );

        assert!(request_touches_unfenced_workspace_live_sync_root(
            &run, &request
        ));
    }

    #[test]
    fn unfenced_workspace_live_sync_rejects_relative_edit_pattern_from_protected_cwd() {
        let run = run("/tmp/arroba-workspace", false);
        let request = request(
            "edit",
            None,
            Some("/tmp/arroba-workspace"),
            &["src/main.rs"],
        );

        assert!(request_touches_unfenced_workspace_live_sync_root(
            &run, &request
        ));
    }

    #[test]
    fn unfenced_workspace_live_sync_does_not_match_sibling_prefix() {
        let run = run("/tmp/arroba-workspace", false);
        let request = request(
            "bash",
            Some("printf x > /tmp/arroba-workspace-copy/file.txt"),
            None,
            &["/tmp/arroba-workspace-copy/*"],
        );

        assert!(!request_touches_unfenced_workspace_live_sync_root(
            &run, &request
        ));
    }

    #[test]
    fn fenced_workspace_live_sync_defers_native_bash_policy_to_platform_fence() {
        let run = run("/tmp/arroba-workspace", true);
        let request = request(
            "bash",
            Some("printf x > /tmp/arroba-workspace/src/main.rs"),
            None,
            &[],
        );

        assert!(!request_touches_unfenced_workspace_live_sync_root(
            &run, &request
        ));
    }
}
