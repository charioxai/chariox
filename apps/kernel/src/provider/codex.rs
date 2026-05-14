use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Mutex, OnceLock};
use std::thread::sleep;
use std::time::{Duration, Instant};

use crate::error::DaemonError;
use crate::mcp::{ArrobaMcpServerConfig, ArrobaMcpTransportConfig};
use crate::provider::{AgentEndpointMode, LaunchProviderRequest, ProviderLaunchResult};

use super::codex_client::codex_endpoint_is_healthy;

const CODEX_ENV_OVERRIDE: &str = "ARROBA_CODEX_BIN";
const CODEX_PORT_OVERRIDE: &str = "ARROBA_CODEX_PORT";
const CODEX_PORT_RANGE_OVERRIDE: &str = "ARROBA_CODEX_PORT_RANGE";
const CODEX_BIND_HOST_OVERRIDE: &str = "ARROBA_CODEX_BIND_HOST";
pub(crate) const CODEX_MCP_TOKEN_ENV: &str = "ARROBA_MCP_TOKEN";
static CODEX_MANAGED_CATALOG_PORT: OnceLock<Mutex<Option<u16>>> = OnceLock::new();
const CODEX_AUTH_ENV_VARS: &[&str] = &[
    "OPENAI_API_KEY",
    "OPENAI_BASE_URL",
    "OPENAI_ORG_ID",
    "OPENAI_ORGANIZATION",
    "OPENAI_PROJECT",
];
const CODEX_SESSION_ENV_VARS: &[&str] = &[
    "CODEX_THREAD_ID",
    "CODEX_TURN_METADATA_HEADER",
    "CODEX_TURN_STATE_HEADER",
    "CODEX_STARTING_DIFF",
    "CODEX_ESCALATE_SOCKET",
];

pub fn resolve_codex_executable() -> Result<PathBuf, DaemonError> {
    let _guard = crate::env_lock::lock();
    resolve_codex_executable_unlocked()
}

fn resolve_codex_executable_unlocked() -> Result<PathBuf, DaemonError> {
    if let Some(path) = env::var_os(CODEX_ENV_OVERRIDE).map(PathBuf::from) {
        return resolve_candidate(path, true).ok_or_else(|| {
            DaemonError::ProviderExecutableNotFound {
                adapter_key: "codex".to_string(),
                executable: env::var(CODEX_ENV_OVERRIDE).unwrap_or_else(|_| "codex".to_string()),
            }
        });
    }

    resolve_candidate(PathBuf::from("codex"), false).ok_or_else(|| {
        DaemonError::ProviderExecutableNotFound {
            adapter_key: "codex".to_string(),
            executable: "codex".to_string(),
        }
    })
}

pub fn plan_codex_launch(
    request: Option<&LaunchProviderRequest>,
) -> Result<ProviderLaunchResult, DaemonError> {
    let _guard = crate::env_lock::lock();
    plan_codex_launch_unlocked(request)
}

fn plan_codex_launch_unlocked(
    request: Option<&LaunchProviderRequest>,
) -> Result<ProviderLaunchResult, DaemonError> {
    if let Some(endpoint) = request.and_then(|request| request.structured_endpoint.clone()) {
        let working_directory = request.and_then(|request| request.working_directory.clone());
        return Ok(ProviderLaunchResult {
            endpoint_mode: AgentEndpointMode::External,
            process_label: "codex:native-app-server".to_string(),
            pty_target: None,
            pty_program: None,
            pty_args: Vec::new(),
            pty_env: BTreeMap::new(),
            pty_env_remove: codex_provider_env_remove(request),
            working_directory,
            structured_endpoint: Some(endpoint),
        });
    }

    let port = if request.is_some() {
        reserve_unused_port_from_range(CODEX_PORT_RANGE_OVERRIDE, "codex_reserve_port_range")?
            .unwrap_or(reserve_unused_port()?)
    } else {
        resolve_codex_port()?
    };
    let endpoint = format!("ws://127.0.0.1:{port}");
    let listen_endpoint = format!("ws://{}:{port}", resolve_codex_bind_host());

    let executable = resolve_codex_executable_unlocked()?;
    let (config_args, env) = runtime_mcp_config(request)?;
    let working_directory = request.and_then(|request| request.working_directory.clone());
    Ok(ProviderLaunchResult {
        endpoint_mode: AgentEndpointMode::Managed,
        process_label: "codex:app-server".to_string(),
        pty_target: None,
        pty_program: Some(executable.display().to_string()),
        pty_args: {
            let mut args = vec!["app-server".to_string()];
            args.extend(config_args);
            args.extend(["--listen".to_string(), listen_endpoint]);
            args
        },
        pty_env: env,
        pty_env_remove: codex_provider_env_remove(request),
        working_directory,
        structured_endpoint: Some(endpoint),
    })
}

fn resolve_codex_bind_host() -> String {
    env::var(CODEX_BIND_HOST_OVERRIDE).unwrap_or_else(|_| "127.0.0.1".to_string())
}

fn codex_provider_env_remove(request: Option<&LaunchProviderRequest>) -> Vec<String> {
    let mut names = request
        .map(|request| request.provider_env_remove.clone())
        .unwrap_or_default();
    for name in CODEX_SESSION_ENV_VARS {
        if !names.iter().any(|existing| existing == name) {
            names.push((*name).to_string());
        }
    }
    names
}

pub fn codex_catalog_endpoint() -> Result<String, DaemonError> {
    let _guard = crate::env_lock::lock();
    codex_catalog_endpoint_unlocked()
}

fn codex_catalog_endpoint_unlocked() -> Result<String, DaemonError> {
    let port = resolve_codex_port()?;
    Ok(format!("ws://127.0.0.1:{port}"))
}

pub fn ensure_codex_catalog_endpoint() -> Result<String, DaemonError> {
    let launch = plan_codex_launch(None)?;
    let endpoint =
        launch
            .structured_endpoint
            .clone()
            .ok_or_else(|| DaemonError::LocalTransport {
                operation: "ensure_codex_catalog_endpoint",
                message: "codex launch did not expose a structured endpoint".to_string(),
            })?;
    if codex_endpoint_is_healthy(&endpoint) {
        return Ok(endpoint);
    }
    if launch.endpoint_mode == AgentEndpointMode::External {
        return Err(DaemonError::LocalTransport {
            operation: "ensure_codex_catalog_endpoint",
            message: format!("configured Codex endpoint `{endpoint}` is not reachable"),
        });
    }

    let program = launch
        .pty_program
        .clone()
        .ok_or_else(|| DaemonError::LocalTransport {
            operation: "ensure_codex_catalog_endpoint",
            message: "codex launch did not provide an executable".to_string(),
        })?;
    let mut command = Command::new(program);
    command
        .args(&launch.pty_args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    if let Some(working_directory) = launch.working_directory.as_ref() {
        command.current_dir(working_directory);
    }
    let mut child = command.spawn().map_err(|error| {
        clear_codex_managed_catalog_port_if_unset();
        DaemonError::LocalTransport {
            operation: "ensure_codex_catalog_endpoint",
            message: format!("failed to start Codex app-server: {error}"),
        }
    })?;

    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if codex_endpoint_is_healthy(&endpoint) {
            return Ok(endpoint);
        }
        if let Some(status) = child
            .try_wait()
            .map_err(|error| DaemonError::LocalTransport {
                operation: "ensure_codex_catalog_endpoint",
                message: format!("failed to poll Codex app-server startup: {error}"),
            })?
        {
            clear_codex_managed_catalog_port_if_unset();
            return Err(DaemonError::LocalTransport {
                operation: "ensure_codex_catalog_endpoint",
                message: format!("Codex app-server exited before becoming healthy: {status}"),
            });
        }
        if Instant::now() >= deadline {
            clear_codex_managed_catalog_port_if_unset();
            return Err(DaemonError::LocalTransport {
                operation: "ensure_codex_catalog_endpoint",
                message: format!(
                    "timed out waiting for Codex app-server to become healthy at `{endpoint}`"
                ),
            });
        }
        sleep(Duration::from_millis(100));
    }
}

pub fn logout_codex() -> Result<(), DaemonError> {
    let executable = resolve_codex_executable()?;
    let status = Command::new(executable)
        .arg("logout")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map_err(|error| DaemonError::LocalTransport {
            operation: "logout_codex",
            message: format!("failed to start `codex logout`: {error}"),
        })?;
    if !status.success() {
        return Err(DaemonError::LocalTransport {
            operation: "logout_codex",
            message: format!("`codex logout` exited unsuccessfully: {status}"),
        });
    }
    Ok(())
}

fn runtime_mcp_config(
    request: Option<&LaunchProviderRequest>,
) -> Result<(Vec<String>, BTreeMap<String, String>), DaemonError> {
    let Some(request) = request else {
        return Ok((Vec::new(), inherited_codex_auth_env()));
    };
    if request.runtime_mcp_binding.is_none() && request.mcp_servers.is_empty() {
        return Ok((Vec::new(), inherited_codex_auth_env()));
    }
    let mut args = vec!["-c".to_string(), "mcp_servers={}".to_string()];
    if request.requires_managed_io() {
        let model_catalog_path = write_managed_io_model_catalog(request.model.as_str())?;
        args.splice(
            0..0,
            [
                "-c".to_string(),
                format!("model_catalog_json={:?}", model_catalog_path),
                "-c".to_string(),
                "features.apply_patch_freeform=false".to_string(),
                "-c".to_string(),
                "include_apply_patch_tool=false".to_string(),
                "-c".to_string(),
                "approval_policy=\"never\"".to_string(),
            ],
        );
    }
    let mut env = inherited_codex_auth_env();
    let provider_mcp_servers = super::mcp_proxy::provider_facing_mcp_proxy_configs_with_bearer_env(
        &request.mcp_servers,
        request
            .runtime_mcp_binding
            .as_ref()
            .map(|binding| binding.server_url.as_str()),
        request
            .runtime_mcp_binding
            .as_ref()
            .map(|binding| binding.auth_token.as_str()),
        CODEX_MCP_TOKEN_ENV,
    )?;
    for server in &provider_mcp_servers {
        append_codex_mcp_config(&mut args, server);
    }
    if let Some(binding) = request.runtime_mcp_binding.as_ref() {
        args.extend([
            "-c".to_string(),
            format!("mcp_servers.arroba.url={:?}", binding.server_url),
            "-c".to_string(),
            format!(
                "mcp_servers.arroba.bearer_token_env_var={:?}",
                CODEX_MCP_TOKEN_ENV
            ),
            "-c".to_string(),
            "mcp_servers.arroba.required=true".to_string(),
            "-c".to_string(),
            "mcp_servers.arroba.tool_timeout_sec=15".to_string(),
        ]);
        env.insert(CODEX_MCP_TOKEN_ENV.to_string(), binding.auth_token.clone());
    }
    Ok((args, env))
}

fn inherited_codex_auth_env() -> BTreeMap<String, String> {
    CODEX_AUTH_ENV_VARS
        .iter()
        .filter_map(|name| {
            env::var(name)
                .ok()
                .map(|value| ((*name).to_string(), value))
        })
        .collect()
}

fn append_codex_mcp_config(args: &mut Vec<String>, server: &ArrobaMcpServerConfig) {
    let prefix = format!("mcp_servers.{}", server.name);
    match &server.transport {
        ArrobaMcpTransportConfig::Stdio {
            command,
            args: server_args,
            env,
            env_vars,
            cwd,
        } => {
            push_codex_config(args, format!("{prefix}.command={command:?}"));
            if !server_args.is_empty() {
                push_codex_config(args, format!("{prefix}.args={server_args:?}"));
            }
            for (key, value) in env {
                push_codex_config(args, format!("{prefix}.env.{key}={value:?}"));
            }
            if !env_vars.is_empty() {
                push_codex_config(args, format!("{prefix}.env_vars={env_vars:?}"));
            }
            if let Some(cwd) = cwd {
                push_codex_config(
                    args,
                    format!("{prefix}.cwd={:?}", cwd.display().to_string()),
                );
            }
        }
        ArrobaMcpTransportConfig::StreamableHttp {
            url,
            bearer_token_env_var,
            http_headers,
            env_http_headers,
        } => {
            let mut fields = vec![format!("url={url:?}")];
            if let Some(env_var) = bearer_token_env_var {
                fields.push(format!("bearer_token_env_var={env_var:?}"));
            }
            if !http_headers.is_empty() {
                fields.push(format!(
                    "http_headers={{{}}}",
                    http_headers
                        .iter()
                        .map(|(key, value)| format!("{key:?}={value:?}"))
                        .collect::<Vec<_>>()
                        .join(",")
                ));
            }
            if !env_http_headers.is_empty() {
                fields.push(format!(
                    "env_http_headers={{{}}}",
                    env_http_headers
                        .iter()
                        .map(|(key, value)| format!("{key:?}={value:?}"))
                        .collect::<Vec<_>>()
                        .join(",")
                ));
            }
            if server.required {
                fields.push("required=true".to_string());
            }
            if let Some(timeout) = server.startup_timeout_sec {
                fields.push(format!("startup_timeout_sec={timeout}"));
            }
            if let Some(timeout) = server.tool_timeout_sec {
                fields.push(format!("tool_timeout_sec={timeout}"));
            }
            if let Some(tools) = &server.enabled_tools {
                fields.push(format!("enabled_tools={tools:?}"));
            }
            if let Some(tools) = &server.disabled_tools {
                fields.push(format!("disabled_tools={tools:?}"));
            }
            push_codex_config(args, format!("{prefix}={{{}}}", fields.join(",")));
            return;
        }
    }
    if server.required {
        push_codex_config(args, format!("{prefix}.required=true"));
    }
    if let Some(timeout) = server.startup_timeout_sec {
        push_codex_config(args, format!("{prefix}.startup_timeout_sec={timeout}"));
    }
    if let Some(timeout) = server.tool_timeout_sec {
        push_codex_config(args, format!("{prefix}.tool_timeout_sec={timeout}"));
    }
}

fn push_codex_config(args: &mut Vec<String>, value: String) {
    args.push("-c".to_string());
    args.push(value);
}

fn write_managed_io_model_catalog(model: &str) -> Result<PathBuf, DaemonError> {
    let slug = model.rsplit('/').next().unwrap_or(model);
    let catalog = serde_json::json!({
        "models": [{
            "slug": slug,
            "display_name": slug,
            "description": "Arroba managed-I/O model metadata overlay",
            "default_reasoning_level": "medium",
            "supported_reasoning_levels": [
                { "effort": "low", "description": "Fast responses with lighter reasoning" },
                { "effort": "medium", "description": "Balanced reasoning" },
                { "effort": "high", "description": "Greater reasoning depth" }
            ],
            "shell_type": "shell_command",
            "visibility": "list",
            "supported_in_api": true,
            "priority": 0,
            "availability_nux": null,
            "upgrade": null,
            "base_instructions": "You are Codex, a coding agent. Follow the user instructions and use available tools exactly as requested.",
            "supports_reasoning_summaries": true,
            "default_reasoning_summary": "auto",
            "support_verbosity": true,
            "default_verbosity": "low",
            "apply_patch_tool_type": null,
            "web_search_tool_type": "text",
            "truncation_policy": { "mode": "tokens", "limit": 10000 },
            "supports_parallel_tool_calls": true,
            "supports_image_detail_original": true,
            "context_window": 272000,
            "effective_context_window_percent": 95,
            "experimental_supported_tools": [],
            "input_modalities": ["text", "image"]
        }]
    });
    let mut hasher = DefaultHasher::new();
    model.hash(&mut hasher);
    let path = env::temp_dir().join(format!(
        "arroba-codex-managed-io-models-{:x}.json",
        hasher.finish()
    ));
    let content = serde_json::to_string(&catalog).map_err(|error| DaemonError::LocalTransport {
        operation: "codex_managed_io_model_catalog",
        message: error.to_string(),
    })?;
    fs::write(&path, content).map_err(|error| DaemonError::LocalTransport {
        operation: "codex_managed_io_model_catalog",
        message: format!("failed to write managed-I/O Codex model catalog: {error}"),
    })?;
    Ok(path)
}

fn resolve_codex_port() -> Result<u16, DaemonError> {
    if let Some(value) = env::var_os(CODEX_PORT_OVERRIDE) {
        let value = value.to_string_lossy().into_owned();
        return value
            .parse::<u16>()
            .map_err(|_| DaemonError::InvalidConfig {
                field: "ARROBA_CODEX_PORT",
                message: "must be a valid TCP port",
            });
    }

    let port = CODEX_MANAGED_CATALOG_PORT.get_or_init(|| Mutex::new(None));
    let mut guard = port.lock().map_err(|error| DaemonError::LocalTransport {
        operation: "codex_managed_catalog_port",
        message: error.to_string(),
    })?;
    if let Some(port) = *guard {
        return Ok(port);
    }
    let reserved = reserve_unused_port()?;
    *guard = Some(reserved);
    Ok(reserved)
}

fn clear_codex_managed_catalog_port_if_unset() {
    if env::var_os(CODEX_PORT_OVERRIDE).is_some() {
        return;
    }
    if let Some(port) = CODEX_MANAGED_CATALOG_PORT.get() {
        if let Ok(mut guard) = port.lock() {
            *guard = None;
        }
    }
}

fn reserve_unused_port() -> Result<u16, DaemonError> {
    TcpListener::bind(("127.0.0.1", 0))
        .map_err(|error| DaemonError::LocalTransport {
            operation: "codex_reserve_port",
            message: error.to_string(),
        })?
        .local_addr()
        .map(|address| address.port())
        .map_err(|error| DaemonError::LocalTransport {
            operation: "codex_reserve_port",
            message: error.to_string(),
        })
}

fn reserve_unused_port_from_range(
    env_name: &'static str,
    operation: &'static str,
) -> Result<Option<u16>, DaemonError> {
    let Some(value) = env::var_os(env_name) else {
        return Ok(None);
    };
    let value = value.to_string_lossy();
    let Some((start, end)) = value.split_once('-') else {
        return Err(DaemonError::InvalidConfig {
            field: env_name,
            message: "must use START-END TCP port range syntax",
        });
    };
    let start = start
        .parse::<u16>()
        .map_err(|_| DaemonError::InvalidConfig {
            field: env_name,
            message: "range start must be a valid TCP port",
        })?;
    let end = end.parse::<u16>().map_err(|_| DaemonError::InvalidConfig {
        field: env_name,
        message: "range end must be a valid TCP port",
    })?;
    if start > end {
        return Err(DaemonError::InvalidConfig {
            field: env_name,
            message: "range start must be less than or equal to range end",
        });
    }
    for port in start..=end {
        if TcpListener::bind(("127.0.0.1", port)).is_ok() {
            return Ok(Some(port));
        }
    }
    Err(DaemonError::LocalTransport {
        operation,
        message: format!("no available port in {env_name}={value}"),
    })
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
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;

    use crate::mcp::ArrobaMcpServerConfig;
    use crate::provider::{AgentEndpointMode, LaunchProviderRequest, RuntimeMcpBinding};

    use super::{logout_codex, plan_codex_launch, resolve_codex_executable};

    fn env_guard() -> crate::env_lock::EnvGuard {
        crate::env_lock::lock()
    }

    #[test]
    fn resolves_override_path_for_tests() {
        let _guard = env_guard();
        let path =
            std::env::temp_dir().join(format!("arroba-codex-resolve-test-{}", std::process::id()));
        fs::write(&path, "#!/bin/sh\nexit 0\n").expect("fixture should exist");
        std::env::set_var("ARROBA_CODEX_BIN", &path);

        let resolved = resolve_codex_executable().expect("override path should resolve");

        std::env::remove_var("ARROBA_CODEX_BIN");
        let _ = fs::remove_file(&path);
        assert_eq!(resolved, path);
    }

    #[test]
    fn plans_codex_app_server_launch() {
        let _guard = env_guard();
        let path = std::env::temp_dir().join(format!(
            "arroba-codex-resolve-test-{}-serve",
            std::process::id()
        ));
        fs::write(&path, "#!/bin/sh\nsleep 60\n").expect("fixture should exist");
        std::env::set_var("ARROBA_CODEX_BIN", &path);
        std::env::set_var("ARROBA_CODEX_PORT", "43142");

        let launch = plan_codex_launch(None).expect("launch plan should resolve");

        std::env::remove_var("ARROBA_CODEX_BIN");
        std::env::remove_var("ARROBA_CODEX_PORT");
        let _ = fs::remove_file(&path);

        assert_eq!(launch.endpoint_mode, AgentEndpointMode::Managed);
        assert_eq!(
            launch.pty_program.as_deref(),
            Some(path.to_string_lossy().as_ref())
        );
        assert_eq!(
            launch.pty_args,
            vec![
                "app-server".to_string(),
                "--listen".to_string(),
                "ws://127.0.0.1:43142".to_string(),
            ]
        );
        assert_eq!(
            launch.structured_endpoint.as_deref(),
            Some("ws://127.0.0.1:43142")
        );
    }

    #[test]
    fn plans_codex_launch_scrubs_inherited_session_env() {
        let _guard = env_guard();
        let path = std::env::temp_dir().join(format!(
            "arroba-codex-resolve-test-{}-env-remove",
            std::process::id()
        ));
        fs::write(&path, "#!/bin/sh\nsleep 60\n").expect("fixture should exist");
        std::env::set_var("ARROBA_CODEX_BIN", &path);
        std::env::set_var("ARROBA_CODEX_PORT", "43142");
        let request =
            LaunchProviderRequest::new("session-1", "codex", "codex", "default", "codex-mini")
                .with_provider_env_remove(vec!["OPENAI_API_KEY".to_string()]);

        let launch = plan_codex_launch(Some(&request)).expect("launch plan should resolve");

        std::env::remove_var("ARROBA_CODEX_BIN");
        std::env::remove_var("ARROBA_CODEX_PORT");
        let _ = fs::remove_file(&path);

        assert!(launch
            .pty_env_remove
            .iter()
            .any(|name| name == "OPENAI_API_KEY"));
        assert!(launch
            .pty_env_remove
            .iter()
            .any(|name| name == "CODEX_THREAD_ID"));
        assert!(launch
            .pty_env_remove
            .iter()
            .any(|name| name == "CODEX_TURN_METADATA_HEADER"));
        assert!(launch
            .pty_env_remove
            .iter()
            .any(|name| name == "CODEX_TURN_STATE_HEADER"));
    }

    #[test]
    fn plans_codex_catalog_launch_without_explicit_port_override() {
        let _guard = env_guard();
        let path = std::env::temp_dir().join(format!(
            "arroba-codex-resolve-test-{}-managed-catalog-port",
            std::process::id()
        ));
        fs::write(&path, "#!/bin/sh\nsleep 60\n").expect("fixture should exist");
        std::env::set_var("ARROBA_CODEX_BIN", &path);
        std::env::remove_var("ARROBA_CODEX_PORT");

        let launch = plan_codex_launch(None).expect("managed catalog port should resolve");

        std::env::remove_var("ARROBA_CODEX_BIN");
        std::env::remove_var("ARROBA_CODEX_PORT");
        let _ = fs::remove_file(&path);

        assert_eq!(launch.endpoint_mode, AgentEndpointMode::Managed);
        assert!(launch
            .structured_endpoint
            .as_deref()
            .is_some_and(|endpoint| endpoint.starts_with("ws://127.0.0.1:")));
    }

    #[test]
    fn injects_runtime_mcp_config_into_managed_launch() {
        let _guard = env_guard();
        let path = std::env::temp_dir().join(format!(
            "arroba-codex-resolve-test-{}-mcp",
            std::process::id()
        ));
        fs::write(&path, "#!/bin/sh\nsleep 60\n").expect("fixture should exist");
        std::env::set_var("ARROBA_CODEX_BIN", &path);
        std::env::set_var("ARROBA_CODEX_PORT", "43143");

        let request =
            LaunchProviderRequest::new("session-1", "codex", "codex", "default", "codex-mini")
                .with_managed_io_required()
                .with_runtime_mcp_binding(RuntimeMcpBinding::new(
                    "http://127.0.0.1:43120/mcp",
                    "token-123",
                ));
        let launch = plan_codex_launch(Some(&request)).expect("launch plan should resolve");

        std::env::remove_var("ARROBA_CODEX_BIN");
        std::env::remove_var("ARROBA_CODEX_PORT");
        let _ = fs::remove_file(&path);

        assert_eq!(
            launch.pty_env.get("ARROBA_MCP_TOKEN").map(String::as_str),
            Some("token-123")
        );
        assert!(launch
            .pty_args
            .iter()
            .any(|arg| arg.contains("mcp_servers.arroba.url")));
        assert!(launch
            .pty_args
            .iter()
            .any(|arg| arg.contains("mcp_servers.arroba.bearer_token_env_var")));
        assert!(launch
            .pty_args
            .iter()
            .any(|arg| arg == "mcp_servers.arroba.required=true"));
        assert!(launch
            .pty_args
            .iter()
            .any(|arg| arg == "mcp_servers.arroba.tool_timeout_sec=15"));
        assert!(launch
            .pty_args
            .iter()
            .any(|arg| arg.contains("model_catalog_json")));
        assert!(launch
            .pty_args
            .iter()
            .any(|arg| arg == "features.apply_patch_freeform=false"));
        assert!(launch
            .pty_args
            .iter()
            .any(|arg| arg == "include_apply_patch_tool=false"));
        assert!(launch
            .pty_args
            .iter()
            .any(|arg| arg == "approval_policy=\"never\""));
    }

    #[test]
    fn runtime_mcp_config_does_not_force_managed_io_overrides_for_unrestricted_launch() {
        let _guard = env_guard();
        let path = std::env::temp_dir().join(format!(
            "arroba-codex-resolve-test-{}-runtime-mcp-unrestricted",
            std::process::id()
        ));
        fs::write(&path, "#!/bin/sh\nsleep 60\n").expect("fixture should exist");
        std::env::set_var("ARROBA_CODEX_BIN", &path);
        std::env::set_var("ARROBA_CODEX_PORT", "43143");

        let request =
            LaunchProviderRequest::new("session-1", "codex", "codex", "default", "codex-mini")
                .with_runtime_mcp_binding(RuntimeMcpBinding::new(
                    "http://127.0.0.1:43120/mcp",
                    "token-123",
                ));
        let launch = plan_codex_launch(Some(&request)).expect("launch plan should resolve");

        std::env::remove_var("ARROBA_CODEX_BIN");
        std::env::remove_var("ARROBA_CODEX_PORT");
        let _ = fs::remove_file(&path);

        assert!(launch
            .pty_args
            .iter()
            .any(|arg| arg.contains("mcp_servers.arroba.url")));
        assert!(!launch
            .pty_args
            .iter()
            .any(|arg| arg.contains("model_catalog_json")));
        assert!(!launch
            .pty_args
            .iter()
            .any(|arg| arg == "features.apply_patch_freeform=false"));
        assert!(!launch
            .pty_args
            .iter()
            .any(|arg| arg == "include_apply_patch_tool=false"));
        assert!(!launch
            .pty_args
            .iter()
            .any(|arg| arg == "approval_policy=\"never\""));
    }

    #[test]
    fn injects_granted_mcp_config_into_managed_launch() {
        let _guard = env_guard();
        let path = std::env::temp_dir().join(format!(
            "arroba-codex-resolve-test-{}-granted-mcp",
            std::process::id()
        ));
        fs::write(&path, "#!/bin/sh\nsleep 60\n").expect("fixture should exist");
        std::env::set_var("ARROBA_CODEX_BIN", &path);
        std::env::set_var("ARROBA_CODEX_PORT", "43144");

        let request =
            LaunchProviderRequest::new("session-1", "codex", "codex", "default", "codex-mini")
                .with_mcp_servers(vec![ArrobaMcpServerConfig::stdio(
                    "browser",
                    "npx",
                    vec!["@playwright/mcp@latest".to_string()],
                )]);
        let launch = plan_codex_launch(Some(&request)).expect("launch plan should resolve");

        std::env::remove_var("ARROBA_CODEX_BIN");
        std::env::remove_var("ARROBA_CODEX_PORT");
        let _ = fs::remove_file(&path);

        assert!(launch
            .pty_args
            .iter()
            .any(|arg| arg == "mcp_servers.browser.command=\"npx\""));
        assert!(launch
            .pty_args
            .iter()
            .any(|arg| arg.contains("mcp_servers.browser.args")));
        assert!(!launch
            .pty_args
            .iter()
            .any(|arg| arg.contains("mcp_servers.arroba.url")));
    }

    #[test]
    fn renders_granted_mcp_as_provider_facing_proxy_when_runtime_mcp_is_bound() {
        let _guard = env_guard();
        let path = std::env::temp_dir().join(format!(
            "arroba-codex-resolve-test-{}-proxied-mcp",
            std::process::id()
        ));
        fs::write(&path, "#!/bin/sh\nsleep 60\n").expect("fixture should exist");
        std::env::set_var("ARROBA_CODEX_BIN", &path);
        std::env::set_var("ARROBA_CODEX_PORT", "43144");

        let request =
            LaunchProviderRequest::new("session-1", "codex", "codex", "default", "codex-mini")
                .with_runtime_mcp_binding(RuntimeMcpBinding::new(
                    "http://127.0.0.1:43120/mcp",
                    "token-123",
                ))
                .with_mcp_servers(vec![ArrobaMcpServerConfig::stdio(
                    "browser",
                    "npx",
                    vec!["@playwright/mcp@latest".to_string()],
                )]);
        let launch = plan_codex_launch(Some(&request)).expect("launch plan should resolve");

        std::env::remove_var("ARROBA_CODEX_BIN");
        std::env::remove_var("ARROBA_CODEX_PORT");
        let _ = fs::remove_file(&path);

        let browser_config = launch
            .pty_args
            .iter()
            .find(|arg| arg.starts_with("mcp_servers.browser={"))
            .expect("browser MCP should be rendered as one streamable HTTP table");
        assert!(browser_config.contains("url=\"http://127.0.0.1:43120/mcp/proxy/browser\""));
        assert!(browser_config.contains("bearer_token_env_var=\"ARROBA_MCP_TOKEN\""));
        assert!(!browser_config.contains("http_headers"));
        assert!(!launch
            .pty_args
            .iter()
            .any(|arg| arg == "mcp_servers.browser.command=\"npx\""));
    }

    #[test]
    fn plans_required_managed_io_launch() {
        let _guard = env_guard();
        let path = std::env::temp_dir().join(format!(
            "arroba-codex-resolve-test-{}-managed-io",
            std::process::id()
        ));
        fs::write(&path, "#!/bin/sh\nsleep 60\n").expect("fixture should exist");
        std::env::set_var("ARROBA_CODEX_BIN", &path);
        std::env::set_var("ARROBA_CODEX_PORT", "43144");
        let request =
            LaunchProviderRequest::new("session-1", "codex", "codex", "default", "codex-mini")
                .with_managed_io_required();

        let launch = plan_codex_launch(Some(&request)).expect("managed I/O launch should resolve");

        std::env::remove_var("ARROBA_CODEX_BIN");
        std::env::remove_var("ARROBA_CODEX_PORT");
        let _ = fs::remove_file(&path);

        assert_eq!(launch.endpoint_mode, AgentEndpointMode::Managed);
        assert!(launch
            .structured_endpoint
            .as_deref()
            .is_some_and(|endpoint| endpoint.starts_with("ws://127.0.0.1:")));
    }

    #[test]
    fn logout_codex_invokes_the_configured_executable() {
        let _guard = env_guard();
        let path =
            std::env::temp_dir().join(format!("arroba-codex-logout-test-{}", std::process::id()));
        let marker =
            std::env::temp_dir().join(format!("arroba-codex-logout-marker-{}", std::process::id()));
        fs::write(
            &path,
            format!(
                "#!/bin/sh\nprintf '%s' \"$1\" > \"{}\"\nexit 0\n",
                marker.display()
            ),
        )
        .expect("fixture should exist");
        #[cfg(unix)]
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755))
            .expect("fixture should be executable");
        std::env::set_var("ARROBA_CODEX_BIN", &path);

        logout_codex().expect("logout should succeed");

        std::env::remove_var("ARROBA_CODEX_BIN");
        let logged = fs::read_to_string(&marker).expect("marker should be written");
        let _ = fs::remove_file(&path);
        let _ = fs::remove_file(&marker);

        assert_eq!(logged, "logout");
    }
}
