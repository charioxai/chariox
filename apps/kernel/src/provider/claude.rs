use std::collections::BTreeMap;
use std::env;
use std::path::{Path, PathBuf};

use crate::error::DaemonError;
use crate::provider::{AgentEndpointMode, LaunchProviderRequest, ProviderLaunchResult};

mod catalog;
mod launch_args;
mod native_tui;

pub use catalog::claude_provider_catalog;
use catalog::CLAUDE_HEADLESS_PROVIDER_ID;
use launch_args::claude_launch_args;
pub(crate) use launch_args::claude_launch_args_for_run;
use native_tui::{claude_native_tui_args, prepare_claude_native_tui_files};

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
    if !request.client_interface.is_arroba() {
        let native = prepare_claude_native_tui_files()?;
        let mut pty_env = BTreeMap::new();
        pty_env.insert(
            "ARROBA_CLAUDE_NATIVE_EVENTS".to_string(),
            native.events_file.display().to_string(),
        );
        pty_env.insert(
            "ARROBA_CLAUDE_NATIVE_CONTEXT".to_string(),
            native.context_file.display().to_string(),
        );
        pty_env.insert(
            "ARROBA_CLAUDE_NATIVE_CONTEXT_RESPONSES".to_string(),
            native.context_response_dir.display().to_string(),
        );
        pty_env.insert(
            "ARROBA_CLAUDE_NATIVE_PERMISSION_RESPONSES".to_string(),
            native.permission_response_dir.display().to_string(),
        );
        pty_env.insert("TERM".to_string(), "xterm-256color".to_string());
        pty_env.insert("COLORTERM".to_string(), "truecolor".to_string());
        return Ok(ProviderLaunchResult {
            endpoint_mode: AgentEndpointMode::Managed,
            process_label: "claude:native-tui".to_string(),
            pty_target: None,
            pty_program: Some(executable.display().to_string()),
            pty_args: claude_native_tui_args(request, &native.settings_file)?,
            pty_env,
            pty_env_remove: claude_provider_env_remove(Some(request)),
            working_directory: request.working_directory.clone(),
            structured_endpoint: None,
        });
    }
    if request.provider == CLAUDE_HEADLESS_PROVIDER_ID {
        let native = prepare_claude_native_tui_files()?;
        let mut pty_env = BTreeMap::new();
        pty_env.insert(
            "ARROBA_CLAUDE_NATIVE_EVENTS".to_string(),
            native.events_file.display().to_string(),
        );
        pty_env.insert(
            "ARROBA_CLAUDE_NATIVE_CONTEXT".to_string(),
            native.context_file.display().to_string(),
        );
        pty_env.insert(
            "ARROBA_CLAUDE_NATIVE_CONTEXT_RESPONSES".to_string(),
            native.context_response_dir.display().to_string(),
        );
        pty_env.insert(
            "ARROBA_CLAUDE_NATIVE_PERMISSION_RESPONSES".to_string(),
            native.permission_response_dir.display().to_string(),
        );
        pty_env.insert(
            "ARROBA_CLAUDE_SETTINGS_FILE".to_string(),
            native.settings_file.display().to_string(),
        );
        return Ok(ProviderLaunchResult {
            endpoint_mode: AgentEndpointMode::Managed,
            process_label: "claude:headless".to_string(),
            pty_target: None,
            pty_program: Some(executable.display().to_string()),
            pty_args: claude_native_tui_args(request, &native.settings_file)?,
            pty_env,
            pty_env_remove: claude_provider_env_remove(Some(request)),
            working_directory: request.working_directory.clone(),
            structured_endpoint: None,
        });
    }
    let native = prepare_claude_native_tui_files()?;
    let mut pty_env = BTreeMap::new();
    pty_env.insert(
        "ARROBA_CLAUDE_NATIVE_EVENTS".to_string(),
        native.events_file.display().to_string(),
    );
    pty_env.insert(
        "ARROBA_CLAUDE_NATIVE_CONTEXT".to_string(),
        native.context_file.display().to_string(),
    );
    pty_env.insert(
        "ARROBA_CLAUDE_NATIVE_CONTEXT_RESPONSES".to_string(),
        native.context_response_dir.display().to_string(),
    );
    pty_env.insert(
        "ARROBA_CLAUDE_NATIVE_PERMISSION_RESPONSES".to_string(),
        native.permission_response_dir.display().to_string(),
    );
    pty_env.insert(
        "ARROBA_CLAUDE_SETTINGS_FILE".to_string(),
        native.settings_file.display().to_string(),
    );
    let mut args = claude_launch_args(request)?;
    args.extend([
        "--settings".to_string(),
        native.settings_file.display().to_string(),
    ]);
    Ok(ProviderLaunchResult {
        endpoint_mode: AgentEndpointMode::External,
        process_label: "claude:stream-json".to_string(),
        pty_target: None,
        pty_program: Some(executable.display().to_string()),
        pty_args: args,
        pty_env,
        pty_env_remove: claude_provider_env_remove(Some(request)),
        working_directory: request.working_directory.clone(),
        structured_endpoint: Some(CLAUDE_STRUCTURED_ENDPOINT.to_string()),
    })
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

    use crate::mcp::ArrobaMcpServerConfig;
    use crate::provider::{
        AgentEndpointMode, AgentExecutionMode, AgentPermissionLevel, LaunchProviderRequest,
        ProviderClientInterface, RuntimeMcpBinding,
    };

    use super::{claude_provider_catalog, plan_claude_launch, resolve_claude_executable};

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
    fn catalog_reads_additional_claude_model_options_cache() {
        let _guard = env_guard();
        let path = std::env::temp_dir().join(format!(
            "arroba-claude-config-models-{}.json",
            std::process::id()
        ));
        fs::write(
            &path,
            serde_json::json!({
                "additionalModelOptionsCache": [
                    { "model": "claude-sonnet-4-8", "displayName": "Claude Sonnet 4.8" },
                    "claude-opus-4-8",
                    { "model": "not-a-claude-model", "displayName": "Ignored" }
                ],
                "additionalModelCostsCache": {
                    "claude-haiku-4-5": {}
                }
            })
            .to_string(),
        )
        .expect("fixture should exist");
        std::env::set_var("ARROBA_CLAUDE_CONFIG", &path);

        let catalog = claude_provider_catalog();

        std::env::remove_var("ARROBA_CLAUDE_CONFIG");
        let _ = fs::remove_file(&path);

        assert_eq!(
            catalog
                .all
                .iter()
                .map(|provider| provider.id.as_str())
                .collect::<Vec<_>>(),
            vec!["claude-headless", "claude-p"]
        );
        assert_eq!(
            catalog.default.get("claude-headless").map(String::as_str),
            Some("claude-sonnet-4-6")
        );
        assert_eq!(
            catalog.default.get("claude-p").map(String::as_str),
            Some("claude-sonnet-4-6")
        );
        let models = &catalog.all[0].models;
        assert!(!models.contains_key("sonnet"));
        assert!(models.contains_key("claude-sonnet-4-6"));
        assert_eq!(
            models
                .get("claude-sonnet-4-8")
                .map(|model| model.name.as_str()),
            Some("Claude Sonnet 4.8")
        );
        assert!(models.contains_key("claude-opus-4-8"));
        assert!(models.contains_key("claude-haiku-4-5"));
        assert!(!models.contains_key("not-a-claude-model"));
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

        let request = LaunchProviderRequest::new(
            "session-1",
            "claude",
            "claude",
            "default",
            "claude/claude-sonnet-4-6",
        )
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
            .any(|pair| pair == ["--model", "claude-sonnet-4-6"]));
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
    fn plans_claude_print_mode_with_structured_stdio() {
        let _guard = env_guard();
        let path = std::env::temp_dir().join(format!(
            "arroba-claude-resolve-test-{}-print-mode",
            std::process::id()
        ));
        fs::write(&path, "#!/bin/sh\nsleep 60\n").expect("fixture should exist");
        std::env::set_var("ARROBA_CLAUDE_BIN", &path);

        let request = LaunchProviderRequest::new(
            "session-1",
            "claude",
            "claude-p",
            "default",
            "claude-p/claude-sonnet-4-6",
        );
        let launch = plan_claude_launch(Some(&request)).expect("launch should resolve");

        std::env::remove_var("ARROBA_CLAUDE_BIN");
        let _ = fs::remove_file(&path);

        assert_eq!(launch.endpoint_mode, AgentEndpointMode::External);
        assert_eq!(
            launch.structured_endpoint.as_deref(),
            Some("stdio://claude")
        );
        assert_eq!(launch.pty_args.first().map(String::as_str), Some("-p"));
        assert!(launch
            .pty_args
            .windows(2)
            .any(|pair| pair == ["--model", "claude-sonnet-4-6"]));
    }

    #[test]
    fn plans_claude_headless_mode_without_print_stream_json() {
        let _guard = env_guard();
        let path = std::env::temp_dir().join(format!(
            "arroba-claude-resolve-test-{}-headless-mode",
            std::process::id()
        ));
        fs::write(&path, "#!/bin/sh\nsleep 60\n").expect("fixture should exist");
        std::env::set_var("ARROBA_CLAUDE_BIN", &path);

        let request = LaunchProviderRequest::new(
            "session-1",
            "claude",
            "claude-headless",
            "default",
            "claude-headless/claude-sonnet-4-6",
        );
        let launch = plan_claude_launch(Some(&request)).expect("launch should resolve");

        std::env::remove_var("ARROBA_CLAUDE_BIN");
        let _ = fs::remove_file(&path);

        assert_eq!(launch.endpoint_mode, AgentEndpointMode::Managed);
        assert_eq!(launch.process_label, "claude:headless");
        assert_eq!(launch.structured_endpoint, None);
        assert!(!launch.pty_args.iter().any(|arg| arg == "-p"));
        assert!(!launch.pty_args.iter().any(|arg| arg == "stream-json"));
        assert!(launch
            .pty_args
            .windows(2)
            .any(|pair| pair == ["--model", "claude-sonnet-4-6"]));
        assert!(launch.pty_env.contains_key("ARROBA_CLAUDE_SETTINGS_FILE"));
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

        let request = LaunchProviderRequest::new(
            "session-1",
            "claude",
            "claude",
            "default",
            "claude-sonnet-4-6",
        );
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

        let request = LaunchProviderRequest::new(
            "session-1",
            "claude",
            "claude",
            "default",
            "claude-sonnet-4-6",
        )
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
        assert!(launch
            .pty_args
            .windows(2)
            .any(|pair| pair == ["--disallowedTools", "ToolSearch"]));
    }

    #[test]
    fn injects_mcp_config_into_native_tui_launch_args() {
        let _guard = env_guard();
        let path = std::env::temp_dir().join(format!(
            "arroba-claude-resolve-test-{}-native-mcp",
            std::process::id()
        ));
        fs::write(&path, "#!/bin/sh\nsleep 60\n").expect("fixture should exist");
        std::env::set_var("ARROBA_CLAUDE_BIN", &path);

        let request = LaunchProviderRequest::new(
            "session-1",
            "claude",
            "claude",
            "default",
            "claude-sonnet-4-6",
        )
        .with_client_interface(ProviderClientInterface::NativeTui)
        .with_runtime_mcp_binding(RuntimeMcpBinding::new(
            "http://127.0.0.1:43120/mcp",
            "token-123",
        ))
        .with_mcp_servers(vec![ArrobaMcpServerConfig::stdio(
            "browser",
            "npx",
            vec!["@playwright/mcp@latest".to_string()],
        )]);
        let launch = plan_claude_launch(Some(&request)).expect("launch should resolve");

        std::env::remove_var("ARROBA_CLAUDE_BIN");
        let _ = fs::remove_file(&path);

        assert_eq!(launch.endpoint_mode, AgentEndpointMode::Managed);
        let config_arg = launch
            .pty_args
            .windows(2)
            .find_map(|pair| (pair[0] == "--mcp-config").then(|| pair[1].as_str()))
            .expect("mcp config should be passed");
        let config: serde_json::Value =
            serde_json::from_str(config_arg).expect("mcp config should be JSON");
        assert_eq!(
            config.pointer("/mcpServers/arroba/url"),
            Some(&serde_json::json!("http://127.0.0.1:43120/mcp"))
        );
        assert!(config.pointer("/mcpServers/browser").is_some());
        assert!(launch
            .pty_args
            .iter()
            .any(|arg| arg == "--strict-mcp-config"));
    }
}
