use std::collections::BTreeMap;
use std::env;
use std::path::{Path, PathBuf};

use crate::error::DaemonError;
use crate::mcp::{ArrobaMcpServerConfig, ArrobaMcpTransportConfig};
use crate::provider::{
    AgentEndpointMode, AgentExecutionMode, AgentPermissionLevel, LaunchProviderRequest,
    OpenCodeProviderCatalog, OpenCodeProviderInfo, OpenCodeProviderModel, ProviderLaunchResult,
    RuntimeProviderRun,
};

pub(crate) const CLAUDE_STRUCTURED_ENDPOINT: &str = "stdio://claude";

const CLAUDE_ENV_OVERRIDE: &str = "ARROBA_CLAUDE_BIN";
const CLAUDE_AUTH_ENV_VARS: &[&str] = &[
    "ANTHROPIC_API_KEY",
    "ANTHROPIC_AUTH_TOKEN",
    "ANTHROPIC_BASE_URL",
    "ANTHROPIC_CUSTOM_HEADERS",
];

pub fn resolve_claude_executable() -> Result<PathBuf, DaemonError> {
    let _guard = crate::env_lock::lock();
    resolve_claude_executable_unlocked()
}

fn resolve_claude_executable_unlocked() -> Result<PathBuf, DaemonError> {
    if let Some(path) = env::var_os(CLAUDE_ENV_OVERRIDE).map(PathBuf::from) {
        return resolve_candidate(path, true).ok_or_else(|| {
            DaemonError::ProviderExecutableNotFound {
                adapter_key: "claude".to_string(),
                executable: env::var(CLAUDE_ENV_OVERRIDE).unwrap_or_else(|_| "claude".to_string()),
            }
        });
    }

    resolve_candidate(PathBuf::from("claude"), false).ok_or_else(|| {
        DaemonError::ProviderExecutableNotFound {
            adapter_key: "claude".to_string(),
            executable: "claude".to_string(),
        }
    })
}

pub fn plan_claude_launch(
    request: Option<&LaunchProviderRequest>,
) -> Result<ProviderLaunchResult, DaemonError> {
    let _guard = crate::env_lock::lock();
    plan_claude_launch_unlocked(request)
}

fn plan_claude_launch_unlocked(
    request: Option<&LaunchProviderRequest>,
) -> Result<ProviderLaunchResult, DaemonError> {
    if let Some(endpoint) = request.and_then(|request| request.structured_endpoint.clone()) {
        let working_directory = request.and_then(|request| request.working_directory.clone());
        return Ok(ProviderLaunchResult {
            endpoint_mode: AgentEndpointMode::External,
            process_label: "claude:structured-stdio-proxy".to_string(),
            pty_target: None,
            pty_program: None,
            pty_args: Vec::new(),
            pty_env: BTreeMap::new(),
            pty_env_remove: claude_provider_env_remove(request),
            working_directory,
            structured_endpoint: Some(endpoint),
        });
    }

    let executable = resolve_claude_executable_unlocked()?;
    let request = request.ok_or_else(|| DaemonError::LocalTransport {
        operation: "plan_claude_launch",
        message: "Claude provider launch requires a provider run request".to_string(),
    })?;
    Ok(ProviderLaunchResult {
        endpoint_mode: AgentEndpointMode::External,
        process_label: "claude:stream-json".to_string(),
        pty_target: None,
        pty_program: Some(executable.display().to_string()),
        pty_args: claude_launch_args(request)?,
        pty_env: BTreeMap::new(),
        pty_env_remove: claude_provider_env_remove(Some(request)),
        working_directory: request.working_directory.clone(),
        structured_endpoint: Some(CLAUDE_STRUCTURED_ENDPOINT.to_string()),
    })
}

pub(crate) fn claude_launch_args_for_run(
    run: &RuntimeProviderRun,
    resume_session_id: Option<&str>,
) -> Result<Vec<String>, DaemonError> {
    claude_launch_args_from_parts(
        run.model(),
        run.variant(),
        run.execution_mode(),
        run.permission_level(),
        resume_session_id,
        run.runtime_mcp_server_url(),
        run.runtime_mcp_auth_token(),
        run.mcp_servers(),
    )
}

fn claude_launch_args(request: &LaunchProviderRequest) -> Result<Vec<String>, DaemonError> {
    claude_launch_args_from_parts(
        request.model.as_str(),
        request.variant.as_deref(),
        request.execution_mode.unwrap_or_default(),
        request.permission_level.unwrap_or_default(),
        request
            .resume_state
            .as_ref()
            .and_then(|state| state.claude_session_id()),
        request
            .runtime_mcp_binding
            .as_ref()
            .map(|binding| binding.server_url.as_str()),
        request
            .runtime_mcp_binding
            .as_ref()
            .map(|binding| binding.auth_token.as_str()),
        &request.mcp_servers,
    )
}

fn claude_launch_args_from_parts(
    model: &str,
    variant: Option<&str>,
    execution_mode: AgentExecutionMode,
    permission_level: AgentPermissionLevel,
    resume_session_id: Option<&str>,
    runtime_mcp_server_url: Option<&str>,
    runtime_mcp_auth_token: Option<&str>,
    mcp_servers: &[ArrobaMcpServerConfig],
) -> Result<Vec<String>, DaemonError> {
    let mut args = vec![
        "-p".to_string(),
        "--input-format".to_string(),
        "stream-json".to_string(),
        "--output-format".to_string(),
        "stream-json".to_string(),
        "--verbose".to_string(),
        "--include-partial-messages".to_string(),
        "--replay-user-messages".to_string(),
    ];

    let model = normalized_claude_model(model);
    if !model.is_empty() && model != "default" {
        args.extend(["--model".to_string(), model]);
    }
    if let Some(variant) = variant.map(str::trim).filter(|value| !value.is_empty()) {
        args.extend(["--effort".to_string(), variant.to_string()]);
    }
    if let Some(session_id) = resume_session_id {
        args.extend(["--resume".to_string(), session_id.to_string()]);
    }
    if let Some(config) =
        claude_mcp_config(mcp_servers, runtime_mcp_server_url, runtime_mcp_auth_token)?
    {
        args.extend(["--mcp-config".to_string(), config]);
        args.push("--strict-mcp-config".to_string());
    }

    match (execution_mode, permission_level) {
        (AgentExecutionMode::Plan, _) => {
            args.extend(["--permission-mode".to_string(), "plan".to_string()]);
        }
        (AgentExecutionMode::Build, AgentPermissionLevel::Required) => {
            args.extend(["--permission-mode".to_string(), "default".to_string()]);
        }
        (AgentExecutionMode::Build, AgentPermissionLevel::Yolo) => {
            args.extend([
                "--permission-mode".to_string(),
                "bypassPermissions".to_string(),
                "--allow-dangerously-skip-permissions".to_string(),
            ]);
        }
    }

    Ok(args)
}

fn claude_mcp_config(
    backing_servers: &[ArrobaMcpServerConfig],
    runtime_mcp_url: Option<&str>,
    runtime_mcp_auth_token: Option<&str>,
) -> Result<Option<String>, DaemonError> {
    let mut mcp_servers = serde_json::Map::new();
    let provider_mcp_servers = super::mcp_proxy::provider_facing_mcp_proxy_configs(
        backing_servers,
        runtime_mcp_url,
        runtime_mcp_auth_token,
    )?;
    for server in &provider_mcp_servers {
        mcp_servers.insert(server.name.clone(), claude_mcp_server_config(server));
    }
    if let (Some(url), Some(token)) = (runtime_mcp_url, runtime_mcp_auth_token) {
        mcp_servers.insert(
            "arroba".to_string(),
            serde_json::json!({
                "type": "http",
                "url": url,
                "headers": {
                    "Authorization": format!("Bearer {token}"),
                },
            }),
        );
    }
    if mcp_servers.is_empty() {
        return Ok(None);
    }
    serde_json::to_string(&serde_json::json!({ "mcpServers": mcp_servers }))
        .map(Some)
        .map_err(|error| DaemonError::LocalTransport {
            operation: "claude_mcp_config",
            message: error.to_string(),
        })
}

fn claude_mcp_server_config(server: &ArrobaMcpServerConfig) -> serde_json::Value {
    match &server.transport {
        ArrobaMcpTransportConfig::Stdio {
            command,
            args,
            env,
            env_vars,
            cwd,
        } => {
            let mut resolved_env = env.clone();
            for name in env_vars {
                if let Ok(value) = std::env::var(name) {
                    resolved_env.insert(name.clone(), value);
                }
            }
            let mut config = serde_json::json!({
                "type": "stdio",
                "command": command,
                "args": args,
                "env": resolved_env,
            });
            if let Some(cwd) = cwd {
                config["cwd"] = serde_json::Value::String(cwd.display().to_string());
            }
            config
        }
        ArrobaMcpTransportConfig::StreamableHttp {
            url,
            bearer_token_env_var,
            http_headers,
            env_http_headers,
        } => {
            let mut headers = http_headers.clone();
            for (header, env_var) in env_http_headers {
                if let Ok(value) = std::env::var(env_var) {
                    headers.insert(header.clone(), value);
                }
            }
            if let Some(env_var) = bearer_token_env_var {
                if let Ok(value) = std::env::var(env_var) {
                    headers.insert("Authorization".to_string(), format!("Bearer {value}"));
                }
            }
            serde_json::json!({
                "type": "http",
                "url": url,
                "headers": headers,
            })
        }
    }
}

fn normalized_claude_model(model: &str) -> String {
    model
        .trim()
        .strip_prefix("claude/")
        .unwrap_or_else(|| model.trim())
        .to_string()
}

fn claude_provider_env_remove(request: Option<&LaunchProviderRequest>) -> Vec<String> {
    let mut names = request
        .map(|request| request.provider_env_remove.clone())
        .unwrap_or_default();
    for name in CLAUDE_AUTH_ENV_VARS {
        if !names.iter().any(|existing| existing == name) {
            names.push((*name).to_string());
        }
    }
    names
}

pub fn claude_provider_catalog() -> OpenCodeProviderCatalog {
    let mut models = BTreeMap::new();
    for (id, name) in [
        ("sonnet", "Claude Sonnet"),
        ("opus", "Claude Opus"),
        ("haiku", "Claude Haiku"),
        ("claude-sonnet-4-6", "Claude Sonnet 4.6"),
        ("claude-opus-4-7", "Claude Opus 4.7"),
    ] {
        models.insert(id.to_string(), claude_model(id, name));
    }
    OpenCodeProviderCatalog {
        all: vec![OpenCodeProviderInfo {
            id: "claude".to_string(),
            name: "Claude Code".to_string(),
            remote_machine_aliases: Vec::new(),
            models,
        }],
        default: BTreeMap::from([("claude".to_string(), "sonnet".to_string())]),
        connected: if resolve_claude_executable().is_ok() {
            vec!["claude".to_string()]
        } else {
            Vec::new()
        },
    }
}

fn claude_model(id: &str, name: &str) -> OpenCodeProviderModel {
    OpenCodeProviderModel {
        id: id.to_string(),
        name: name.to_string(),
        status: "available".to_string(),
        limit: None,
        variants: BTreeMap::from([
            ("low".to_string(), serde_json::json!({ "name": "Low" })),
            (
                "medium".to_string(),
                serde_json::json!({ "name": "Medium" }),
            ),
            ("high".to_string(), serde_json::json!({ "name": "High" })),
            (
                "xhigh".to_string(),
                serde_json::json!({ "name": "Extra High" }),
            ),
            ("max".to_string(), serde_json::json!({ "name": "Max" })),
        ]),
    }
}

fn resolve_candidate(candidate: PathBuf, treat_as_literal_path: bool) -> Option<PathBuf> {
    if treat_as_literal_path || candidate.components().count() > 1 {
        return candidate.exists().then_some(candidate);
    }

    if candidate.is_absolute() && candidate.exists() {
        return Some(candidate);
    }

    let path_var = env::var_os("PATH")?;
    env::split_paths(&path_var)
        .map(|directory| directory.join(&candidate))
        .find(|path| is_executable_file(path))
}

fn is_executable_file(path: &Path) -> bool {
    path.is_file()
}

#[cfg(test)]
mod tests {
    use std::fs;

    use crate::provider::{
        AgentEndpointMode, AgentExecutionMode, AgentPermissionLevel, LaunchProviderRequest,
        RuntimeMcpBinding,
    };

    use super::{plan_claude_launch, resolve_claude_executable};

    fn env_guard() -> crate::env_lock::EnvGuard {
        crate::env_lock::lock()
    }

    #[test]
    fn resolves_override_path_for_tests() {
        let _guard = env_guard();
        let path =
            std::env::temp_dir().join(format!("arroba-claude-resolve-test-{}", std::process::id()));
        fs::write(&path, "#!/bin/sh\nexit 0\n").expect("fixture should exist");
        std::env::set_var("ARROBA_CLAUDE_BIN", &path);

        let resolved = resolve_claude_executable().expect("override path should resolve");

        std::env::remove_var("ARROBA_CLAUDE_BIN");
        let _ = fs::remove_file(&path);
        assert_eq!(resolved, path);
    }

    #[test]
    fn plans_structured_stdio_launch_with_permission_mapping() {
        let _guard = env_guard();
        let path = std::env::temp_dir().join(format!(
            "arroba-claude-resolve-test-{}-launch",
            std::process::id()
        ));
        fs::write(&path, "#!/bin/sh\nsleep 60\n").expect("fixture should exist");
        std::env::set_var("ARROBA_CLAUDE_BIN", &path);
        std::env::set_var("ANTHROPIC_API_KEY", "not-used-by-arroba");

        let request =
            LaunchProviderRequest::new("session-1", "claude", "claude", "default", "claude/sonnet")
                .with_variant(Some("high".to_string()))
                .with_execution_mode(AgentExecutionMode::Plan)
                .with_permission_level(AgentPermissionLevel::Yolo);
        let launch = plan_claude_launch(Some(&request)).expect("launch should resolve");

        std::env::remove_var("ARROBA_CLAUDE_BIN");
        std::env::remove_var("ANTHROPIC_API_KEY");
        let _ = fs::remove_file(&path);

        assert_eq!(launch.endpoint_mode, AgentEndpointMode::External);
        assert_eq!(
            launch.structured_endpoint.as_deref(),
            Some("stdio://claude")
        );
        assert!(launch
            .pty_args
            .windows(2)
            .any(|pair| pair == ["--model", "sonnet"]));
        assert!(launch
            .pty_args
            .windows(2)
            .any(|pair| pair == ["--effort", "high"]));
        assert!(launch
            .pty_args
            .windows(2)
            .any(|pair| pair == ["--permission-mode", "plan"]));
        assert!(launch
            .pty_env_remove
            .iter()
            .any(|name| name == "ANTHROPIC_API_KEY"));
    }

    #[test]
    fn maps_yolo_build_to_bypass_permissions() {
        let _guard = env_guard();
        let path = std::env::temp_dir().join(format!(
            "arroba-claude-resolve-test-{}-yolo",
            std::process::id()
        ));
        fs::write(&path, "#!/bin/sh\nsleep 60\n").expect("fixture should exist");
        std::env::set_var("ARROBA_CLAUDE_BIN", &path);

        let request =
            LaunchProviderRequest::new("session-1", "claude", "claude", "default", "sonnet");
        let launch = plan_claude_launch(Some(&request)).expect("launch should resolve");

        std::env::remove_var("ARROBA_CLAUDE_BIN");
        let _ = fs::remove_file(&path);

        assert!(launch
            .pty_args
            .windows(2)
            .any(|pair| pair == ["--permission-mode", "bypassPermissions"]));
        assert!(launch
            .pty_args
            .iter()
            .any(|arg| arg == "--allow-dangerously-skip-permissions"));
    }

    #[test]
    fn injects_runtime_mcp_config_into_launch_args() {
        let _guard = env_guard();
        let path = std::env::temp_dir().join(format!(
            "arroba-claude-resolve-test-{}-mcp",
            std::process::id()
        ));
        fs::write(&path, "#!/bin/sh\nsleep 60\n").expect("fixture should exist");
        std::env::set_var("ARROBA_CLAUDE_BIN", &path);

        let request =
            LaunchProviderRequest::new("session-1", "claude", "claude", "default", "sonnet")
                .with_runtime_mcp_binding(RuntimeMcpBinding::new(
                    "http://127.0.0.1:43120/mcp",
                    "token-123",
                ));
        let launch = plan_claude_launch(Some(&request)).expect("launch should resolve");

        std::env::remove_var("ARROBA_CLAUDE_BIN");
        let _ = fs::remove_file(&path);

        let config_arg = launch
            .pty_args
            .windows(2)
            .find_map(|pair| (pair[0] == "--mcp-config").then(|| pair[1].as_str()))
            .expect("mcp config should be passed");
        let config: serde_json::Value =
            serde_json::from_str(config_arg).expect("mcp config should be JSON");
        assert_eq!(
            config.pointer("/mcpServers/arroba/type"),
            Some(&serde_json::json!("http"))
        );
        assert_eq!(
            config.pointer("/mcpServers/arroba/url"),
            Some(&serde_json::json!("http://127.0.0.1:43120/mcp"))
        );
        assert_eq!(
            config.pointer("/mcpServers/arroba/headers/Authorization"),
            Some(&serde_json::json!("Bearer token-123"))
        );
        assert!(launch
            .pty_args
            .iter()
            .any(|arg| arg == "--strict-mcp-config"));
    }
}
