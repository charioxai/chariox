use std::collections::BTreeMap;
use std::env;
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Mutex, OnceLock};
use std::thread::sleep;
use std::time::{Duration, Instant};

use crate::error::DaemonError;
use crate::mcp::{ArrobaMcpServerConfig, ArrobaMcpTransportConfig};
use crate::provider::{
    AgentEndpointMode, LaunchProviderRequest, OpenCodeClient, ProviderLaunchResult,
};

const OPENCODE_ENV_OVERRIDE: &str = "ARROBA_OPENCODE_BIN";
const OPENCODE_PORT_OVERRIDE: &str = "ARROBA_OPENCODE_PORT";
static OPENCODE_MANAGED_CATALOG_PORT: OnceLock<Mutex<Option<u16>>> = OnceLock::new();

pub fn resolve_opencode_executable() -> Result<PathBuf, DaemonError> {
    let _guard = crate::env_lock::lock();
    resolve_opencode_executable_unlocked()
}

fn resolve_opencode_executable_unlocked() -> Result<PathBuf, DaemonError> {
    if let Some(path) = env::var_os(OPENCODE_ENV_OVERRIDE).map(PathBuf::from) {
        return resolve_candidate(path, true).ok_or_else(|| {
            DaemonError::ProviderExecutableNotFound {
                adapter_key: "opencode".to_string(),
                executable: env::var(OPENCODE_ENV_OVERRIDE)
                    .unwrap_or_else(|_| "opencode".to_string()),
            }
        });
    }

    resolve_candidate(PathBuf::from("opencode"), false).ok_or_else(|| {
        DaemonError::ProviderExecutableNotFound {
            adapter_key: "opencode".to_string(),
            executable: "opencode".to_string(),
        }
    })
}

const OPENCODE_CONFIG_CONTENT_ENV: &str = "OPENCODE_CONFIG_CONTENT";

pub fn plan_opencode_launch(
    request: Option<&LaunchProviderRequest>,
) -> Result<ProviderLaunchResult, DaemonError> {
    let _guard = crate::env_lock::lock();
    plan_opencode_launch_unlocked(request)
}

fn plan_opencode_launch_unlocked(
    request: Option<&LaunchProviderRequest>,
) -> Result<ProviderLaunchResult, DaemonError> {
    if request.is_some() {
        let executable = resolve_opencode_executable_unlocked()?;
        let port = reserve_unused_port()?;
        let base_url = format!("http://127.0.0.1:{port}");
        return Ok(managed_launch(
            executable,
            port,
            base_url,
            runtime_mcp_env(request)?,
            request
                .map(|request| request.provider_env_remove.clone())
                .unwrap_or_default(),
        ));
    }

    let port = resolve_opencode_port()?;
    let base_url = format!("http://127.0.0.1:{port}");
    let executable = resolve_opencode_executable_unlocked()?;

    Ok(managed_launch(
        executable,
        port,
        base_url,
        runtime_mcp_env(request)?,
        request
            .map(|request| request.provider_env_remove.clone())
            .unwrap_or_default(),
    ))
}

fn managed_launch(
    executable: PathBuf,
    port: u16,
    base_url: String,
    pty_env: BTreeMap<String, String>,
    pty_env_remove: Vec<String>,
) -> ProviderLaunchResult {
    ProviderLaunchResult {
        endpoint_mode: AgentEndpointMode::Managed,
        process_label: "opencode:serve".to_string(),
        pty_target: None,
        pty_program: Some(executable.display().to_string()),
        pty_args: vec![
            "serve".to_string(),
            "--hostname".to_string(),
            "127.0.0.1".to_string(),
            "--port".to_string(),
            port.to_string(),
        ],
        pty_env,
        pty_env_remove,
        working_directory: None,
        structured_endpoint: Some(base_url),
    }
}

pub fn opencode_catalog_endpoint() -> Result<String, DaemonError> {
    let _guard = crate::env_lock::lock();
    opencode_catalog_endpoint_unlocked()
}

fn opencode_catalog_endpoint_unlocked() -> Result<String, DaemonError> {
    let port = resolve_opencode_port()?;
    Ok(format!("http://127.0.0.1:{port}"))
}

pub fn ensure_opencode_catalog_endpoint() -> Result<String, DaemonError> {
    let launch = plan_opencode_launch(None)?;
    let endpoint =
        launch
            .structured_endpoint
            .clone()
            .ok_or_else(|| DaemonError::LocalTransport {
                operation: "ensure_opencode_catalog_endpoint",
                message: "opencode launch did not expose a structured endpoint".to_string(),
            })?;
    if endpoint_is_healthy(&endpoint) {
        return Ok(endpoint);
    }
    if launch.endpoint_mode == AgentEndpointMode::External {
        return Err(DaemonError::LocalTransport {
            operation: "ensure_opencode_catalog_endpoint",
            message: format!("configured OpenCode endpoint `{endpoint}` is not reachable"),
        });
    }

    let program = launch
        .pty_program
        .clone()
        .ok_or_else(|| DaemonError::LocalTransport {
            operation: "ensure_opencode_catalog_endpoint",
            message: "opencode launch did not provide an executable".to_string(),
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
        clear_opencode_managed_catalog_port_if_unset();
        DaemonError::LocalTransport {
            operation: "ensure_opencode_catalog_endpoint",
            message: format!("failed to start OpenCode server: {error}"),
        }
    })?;

    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if endpoint_is_healthy(&endpoint) {
            return Ok(endpoint);
        }
        if let Some(status) = child
            .try_wait()
            .map_err(|error| DaemonError::LocalTransport {
                operation: "ensure_opencode_catalog_endpoint",
                message: format!("failed to poll OpenCode server startup: {error}"),
            })?
        {
            clear_opencode_managed_catalog_port_if_unset();
            return Err(DaemonError::LocalTransport {
                operation: "ensure_opencode_catalog_endpoint",
                message: format!("OpenCode server exited before becoming healthy: {status}"),
            });
        }
        if Instant::now() >= deadline {
            clear_opencode_managed_catalog_port_if_unset();
            return Err(DaemonError::LocalTransport {
                operation: "ensure_opencode_catalog_endpoint",
                message: format!(
                    "timed out waiting for OpenCode server to become healthy at `{endpoint}`"
                ),
            });
        }
        sleep(Duration::from_millis(100));
    }
}

fn runtime_mcp_env(
    request: Option<&LaunchProviderRequest>,
) -> Result<BTreeMap<String, String>, DaemonError> {
    let mut env = BTreeMap::new();
    let Some(request) = request else {
        return Ok(env);
    };
    let mut config = serde_json::Map::new();
    let mut mcp = serde_json::Map::new();
    let provider_mcp_servers = super::mcp_proxy::provider_facing_mcp_proxy_configs(
        &request.mcp_servers,
        request
            .runtime_mcp_binding
            .as_ref()
            .map(|binding| binding.server_url.as_str()),
        request
            .runtime_mcp_binding
            .as_ref()
            .map(|binding| binding.auth_token.as_str()),
    )?;
    for server in &provider_mcp_servers {
        mcp.insert(server.name.clone(), opencode_mcp_config(server));
    }
    if let Some(binding) = request.runtime_mcp_binding.as_ref() {
        mcp.insert(
            "arroba".to_string(),
            serde_json::json!({
                "type": "remote",
                "url": binding.server_url,
                "enabled": true,
                "headers": {
                    "Authorization": format!("Bearer {}", binding.auth_token),
                }
            }),
        );
    }
    if !mcp.is_empty() {
        config.insert("mcp".to_string(), serde_json::Value::Object(mcp));
    }
    if !config.is_empty() {
        env.insert(
            OPENCODE_CONFIG_CONTENT_ENV.to_string(),
            serde_json::Value::Object(config).to_string(),
        );
    }
    Ok(env)
}

fn reserve_unused_port() -> Result<u16, DaemonError> {
    TcpListener::bind(("127.0.0.1", 0))
        .map_err(|error| DaemonError::LocalTransport {
            operation: "opencode_reserve_port",
            message: error.to_string(),
        })?
        .local_addr()
        .map(|address| address.port())
        .map_err(|error| DaemonError::LocalTransport {
            operation: "opencode_reserve_port",
            message: error.to_string(),
        })
}

pub(crate) fn opencode_mcp_config(server: &ArrobaMcpServerConfig) -> serde_json::Value {
    match &server.transport {
        ArrobaMcpTransportConfig::Stdio {
            command,
            args,
            env: static_env,
            env_vars,
            cwd,
        } => {
            let mut env = static_env.clone();
            for name in env_vars {
                if let Ok(value) = std::env::var(name) {
                    env.insert(name.clone(), value);
                }
            }
            let command_parts = std::iter::once(command.clone())
                .chain(args.iter().cloned())
                .collect::<Vec<_>>();
            let mut config = serde_json::json!({
                "type": "local",
                "command": command_parts,
                "enabled": server.enabled,
                "environment": env,
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
                "type": "remote",
                "url": url,
                "enabled": server.enabled,
                "headers": headers,
            })
        }
    }
}

fn endpoint_is_healthy(base_url: &str) -> bool {
    OpenCodeClient::new("catalog", base_url)
        .and_then(|client| client.check_health())
        .is_ok()
}

fn resolve_opencode_port() -> Result<u16, DaemonError> {
    if let Some(value) = env::var_os(OPENCODE_PORT_OVERRIDE) {
        let value = value.to_string_lossy().into_owned();
        return value
            .parse::<u16>()
            .map_err(|_| DaemonError::InvalidConfig {
                field: "ARROBA_OPENCODE_PORT",
                message: "must be a valid TCP port",
            });
    }

    let port = OPENCODE_MANAGED_CATALOG_PORT.get_or_init(|| Mutex::new(None));
    let mut guard = port.lock().map_err(|error| DaemonError::LocalTransport {
        operation: "opencode_managed_catalog_port",
        message: error.to_string(),
    })?;
    if let Some(port) = *guard {
        return Ok(port);
    }
    let reserved = reserve_unused_port()?;
    *guard = Some(reserved);
    Ok(reserved)
}

fn clear_opencode_managed_catalog_port_if_unset() {
    if env::var_os(OPENCODE_PORT_OVERRIDE).is_some() {
        return;
    }
    if let Some(port) = OPENCODE_MANAGED_CATALOG_PORT.get() {
        if let Ok(mut guard) = port.lock() {
            *guard = None;
        }
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
    use crate::mcp::ArrobaMcpServerConfig;
    use crate::provider::{AgentEndpointMode, LaunchProviderRequest, RuntimeMcpBinding};
    use std::fs;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::thread;

    use super::{
        ensure_opencode_catalog_endpoint, plan_opencode_launch, resolve_opencode_executable,
    };

    fn env_guard() -> crate::env_lock::EnvGuard {
        crate::env_lock::lock()
    }

    fn reserve_unused_port() -> u16 {
        TcpListener::bind(("127.0.0.1", 0))
            .expect("ephemeral listener should bind")
            .local_addr()
            .expect("listener should expose a local address")
            .port()
    }

    fn start_health_server() -> (String, thread::JoinHandle<()>) {
        let listener = TcpListener::bind(("127.0.0.1", 0)).expect("test listener should bind");
        let address = listener
            .local_addr()
            .expect("listener should expose a local address");
        let handle = thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("client should connect");
            let mut request = [0_u8; 1024];
            let _ = stream.read(&mut request);
            let response = "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n{\"healthy\":true}";
            stream
                .write_all(response.as_bytes())
                .expect("server should write health response");
            stream.flush().expect("server should flush response");
        });
        (format!("http://{}", address), handle)
    }

    #[test]
    fn resolves_override_path_for_tests() {
        let _guard = env_guard();
        let path = std::env::temp_dir().join(format!(
            "arroba-opencode-resolve-test-{}",
            std::process::id()
        ));
        fs::write(&path, "#!/bin/sh\nexit 0\n").expect("fixture should exist");
        std::env::set_var("ARROBA_OPENCODE_BIN", &path);

        let resolved = resolve_opencode_executable().expect("override path should resolve");

        std::env::remove_var("ARROBA_OPENCODE_BIN");
        let _ = fs::remove_file(&path);
        assert_eq!(resolved, path);
    }

    #[test]
    fn plans_opencode_serve_launch() {
        let _guard = env_guard();
        let path = std::env::temp_dir().join(format!(
            "arroba-opencode-resolve-test-{}-serve",
            std::process::id()
        ));
        fs::write(&path, "#!/bin/sh\nsleep 60\n").expect("fixture should exist");
        std::env::set_var("ARROBA_OPENCODE_BIN", &path);
        let port = reserve_unused_port();
        std::env::set_var("ARROBA_OPENCODE_PORT", port.to_string());

        let launch = plan_opencode_launch(None).expect("launch plan should resolve");

        std::env::remove_var("ARROBA_OPENCODE_BIN");
        std::env::remove_var("ARROBA_OPENCODE_PORT");
        let _ = fs::remove_file(&path);

        assert_eq!(launch.endpoint_mode, AgentEndpointMode::Managed);
        assert_eq!(
            launch.pty_program.as_deref(),
            Some(path.to_string_lossy().as_ref())
        );
        assert_eq!(launch.pty_args[0], "serve");
        assert_eq!(launch.pty_args[1], "--hostname");
        assert_eq!(launch.pty_args[2], "127.0.0.1");
        assert_eq!(launch.pty_args[3], "--port");
        let planned_port = launch.pty_args[4]
            .parse::<u16>()
            .expect("port argument should be numeric");
        assert_eq!(planned_port, port);
        let endpoint = format!("http://127.0.0.1:{planned_port}");
        assert_eq!(
            launch.structured_endpoint.as_deref(),
            Some(endpoint.as_str())
        );
    }

    #[test]
    fn injects_runtime_mcp_config_into_managed_launch() {
        let _guard = env_guard();
        let path = std::env::temp_dir().join(format!(
            "arroba-opencode-resolve-test-{}-mcp",
            std::process::id()
        ));
        fs::write(&path, "#!/bin/sh\nsleep 60\n").expect("fixture should exist");
        std::env::set_var("ARROBA_OPENCODE_BIN", &path);
        std::env::set_var("ARROBA_OPENCODE_PORT", reserve_unused_port().to_string());

        let request = LaunchProviderRequest::new(
            "session-1",
            "opencode",
            "opencode",
            "default",
            "anthropic/claude-sonnet-4",
        )
        .with_runtime_mcp_binding(RuntimeMcpBinding::new(
            "http://127.0.0.1:43120/mcp",
            "token-123",
        ));
        let launch = plan_opencode_launch(Some(&request)).expect("launch plan should resolve");

        std::env::remove_var("ARROBA_OPENCODE_BIN");
        std::env::remove_var("ARROBA_OPENCODE_PORT");
        let _ = fs::remove_file(&path);

        let config = launch
            .pty_env
            .get("OPENCODE_CONFIG_CONTENT")
            .expect("opencode config env should be set");
        assert!(config.contains("\"mcp\""));
        assert!(config.contains("http://127.0.0.1:43120/mcp"));
        assert!(config.contains("Bearer token-123"));
    }

    #[test]
    fn runtime_mcp_launch_uses_isolated_opencode_server() {
        let _guard = env_guard();
        let path = std::env::temp_dir().join(format!(
            "arroba-opencode-resolve-test-{}-isolated-mcp",
            std::process::id()
        ));
        fs::write(&path, "#!/bin/sh\nsleep 60\n").expect("fixture should exist");
        std::env::set_var("ARROBA_OPENCODE_BIN", &path);
        std::env::remove_var("ARROBA_OPENCODE_ENDPOINT");
        std::env::remove_var("ARROBA_OPENCODE_PORT");

        let request = LaunchProviderRequest::new(
            "session-1",
            "opencode",
            "opencode",
            "default",
            "anthropic/claude-sonnet-4",
        )
        .with_runtime_mcp_binding(RuntimeMcpBinding::new(
            "http://127.0.0.1:43120/mcp",
            "token-123",
        ));
        let launch = plan_opencode_launch(Some(&request)).expect("launch plan should resolve");

        std::env::remove_var("ARROBA_OPENCODE_BIN");
        let _ = fs::remove_file(&path);

        assert_eq!(launch.endpoint_mode, AgentEndpointMode::Managed);
        assert_eq!(launch.pty_args[0], "serve");
        assert_eq!(launch.pty_args[1], "--hostname");
        assert_eq!(launch.pty_args[2], "127.0.0.1");
        assert_eq!(launch.pty_args[3], "--port");
        assert!(launch
            .structured_endpoint
            .as_deref()
            .is_some_and(|endpoint| { endpoint.starts_with("http://127.0.0.1:") }));
        assert!(launch.pty_env.contains_key("OPENCODE_CONFIG_CONTENT"));
    }

    #[test]
    fn provider_runs_ignore_external_opencode_endpoint_override() {
        let _guard = env_guard();
        let path = std::env::temp_dir().join(format!(
            "arroba-opencode-resolve-test-{}-ignore-endpoint",
            std::process::id()
        ));
        fs::write(&path, "#!/bin/sh\nsleep 60\n").expect("fixture should exist");
        std::env::set_var("ARROBA_OPENCODE_ENDPOINT", "http://127.0.0.1:43119");
        std::env::set_var("ARROBA_OPENCODE_BIN", &path);
        std::env::remove_var("ARROBA_OPENCODE_PORT");

        let request = LaunchProviderRequest::new(
            "session-1",
            "opencode",
            "opencode",
            "default",
            "anthropic/claude-sonnet-4",
        )
        .with_runtime_mcp_binding(RuntimeMcpBinding::new(
            "http://127.0.0.1:43120/mcp",
            "token-123",
        ));
        let launch = plan_opencode_launch(Some(&request)).expect("provider run should resolve");

        std::env::remove_var("ARROBA_OPENCODE_ENDPOINT");
        std::env::remove_var("ARROBA_OPENCODE_BIN");
        let _ = fs::remove_file(&path);

        assert_eq!(launch.endpoint_mode, AgentEndpointMode::Managed);
        assert_ne!(
            launch.structured_endpoint.as_deref(),
            Some("http://127.0.0.1:43119")
        );
    }

    #[test]
    fn injects_granted_mcp_config_into_managed_launch() {
        let _guard = env_guard();
        let path = std::env::temp_dir().join(format!(
            "arroba-opencode-resolve-test-{}-granted-mcp",
            std::process::id()
        ));
        fs::write(&path, "#!/bin/sh\nsleep 60\n").expect("fixture should exist");
        std::env::set_var("ARROBA_OPENCODE_BIN", &path);
        let port = reserve_unused_port();
        std::env::set_var("ARROBA_OPENCODE_PORT", port.to_string());

        let request = LaunchProviderRequest::new(
            "session-1",
            "opencode",
            "opencode",
            "default",
            "openai/gpt-5.3",
        )
        .with_mcp_servers(vec![ArrobaMcpServerConfig::stdio(
            "browser",
            "npx",
            vec!["@playwright/mcp@latest".to_string()],
        )]);
        let launch = plan_opencode_launch(Some(&request)).expect("launch plan should resolve");

        std::env::remove_var("ARROBA_OPENCODE_BIN");
        std::env::remove_var("ARROBA_OPENCODE_PORT");
        let _ = fs::remove_file(&path);

        let config = launch
            .pty_env
            .get("OPENCODE_CONFIG_CONTENT")
            .expect("opencode config env should be set");
        assert!(config.contains("\"browser\""));
        assert!(config.contains("\"type\":\"local\""));
        assert!(config.contains("@playwright/mcp@latest"));
        assert!(!config.contains("\"arroba\""));
    }

    #[test]
    fn renders_granted_mcp_as_provider_facing_proxy_when_runtime_mcp_is_bound() {
        let _guard = env_guard();
        let path = std::env::temp_dir().join(format!(
            "arroba-opencode-resolve-test-{}-proxied-mcp",
            std::process::id()
        ));
        fs::write(&path, "#!/bin/sh\nsleep 60\n").expect("fixture should exist");
        std::env::set_var("ARROBA_OPENCODE_BIN", &path);
        let port = reserve_unused_port();
        std::env::set_var("ARROBA_OPENCODE_PORT", port.to_string());

        let request = LaunchProviderRequest::new(
            "session-1",
            "opencode",
            "opencode",
            "default",
            "openai/gpt-5.3",
        )
        .with_runtime_mcp_binding(RuntimeMcpBinding::new(
            "http://127.0.0.1:43120/mcp",
            "token-123",
        ))
        .with_mcp_servers(vec![ArrobaMcpServerConfig::stdio(
            "browser",
            "npx",
            vec!["@playwright/mcp@latest".to_string()],
        )]);
        let launch = plan_opencode_launch(Some(&request)).expect("launch plan should resolve");

        std::env::remove_var("ARROBA_OPENCODE_BIN");
        std::env::remove_var("ARROBA_OPENCODE_PORT");
        let _ = fs::remove_file(&path);

        let config = launch
            .pty_env
            .get("OPENCODE_CONFIG_CONTENT")
            .expect("opencode config env should be set");
        assert!(config.contains("\"browser\""));
        assert!(config.contains("\"type\":\"remote\""));
        assert!(config.contains("http://127.0.0.1:43120/mcp/proxy/browser"));
        assert!(config.contains("Bearer token-123"));
        assert!(!config.contains("@playwright/mcp@latest"));
    }

    #[test]
    fn plans_required_managed_io_launch() {
        let _guard = env_guard();
        let path = std::env::temp_dir().join(format!(
            "arroba-opencode-resolve-test-{}-managed-io",
            std::process::id()
        ));
        fs::write(&path, "#!/bin/sh\nsleep 60\n").expect("fixture should exist");
        std::env::set_var("ARROBA_OPENCODE_BIN", &path);
        let port = reserve_unused_port();
        std::env::set_var("ARROBA_OPENCODE_PORT", port.to_string());
        let request = LaunchProviderRequest::new(
            "session-1",
            "opencode",
            "opencode",
            "default",
            "anthropic/claude-sonnet-4",
        )
        .with_managed_io_required();

        let launch =
            plan_opencode_launch(Some(&request)).expect("managed I/O launch should resolve");

        std::env::remove_var("ARROBA_OPENCODE_BIN");
        std::env::remove_var("ARROBA_OPENCODE_PORT");
        let _ = fs::remove_file(&path);

        assert_eq!(launch.endpoint_mode, AgentEndpointMode::Managed);
        let planned_port = launch.pty_args[4]
            .parse::<u16>()
            .expect("port argument should be numeric");
        assert_eq!(
            launch.structured_endpoint.as_deref(),
            Some(format!("http://127.0.0.1:{planned_port}").as_str())
        );
    }

    #[test]
    fn plans_catalog_launch_without_explicit_opencode_port_override() {
        let _guard = env_guard();
        let previous_bin = std::env::var_os("ARROBA_OPENCODE_BIN");
        let path = std::env::temp_dir().join(format!(
            "arroba-opencode-resolve-test-{}-managed-catalog-port",
            std::process::id()
        ));
        fs::write(&path, "#!/bin/sh\nsleep 60\n").expect("fixture should exist");
        std::env::set_var("ARROBA_OPENCODE_BIN", &path);
        let previous_port = std::env::var_os("ARROBA_OPENCODE_PORT");
        std::env::remove_var("ARROBA_OPENCODE_PORT");

        let launch = plan_opencode_launch(None).expect("managed catalog port should resolve");

        if let Some(previous_bin) = previous_bin {
            std::env::set_var("ARROBA_OPENCODE_BIN", previous_bin);
        } else {
            std::env::remove_var("ARROBA_OPENCODE_BIN");
        }
        let _ = fs::remove_file(&path);
        if let Some(previous_port) = previous_port {
            std::env::set_var("ARROBA_OPENCODE_PORT", previous_port);
        } else {
            std::env::remove_var("ARROBA_OPENCODE_PORT");
        }

        assert_eq!(launch.endpoint_mode, AgentEndpointMode::Managed);
        assert!(launch
            .structured_endpoint
            .as_deref()
            .is_some_and(|endpoint| endpoint.starts_with("http://127.0.0.1:")));
    }

    #[test]
    fn ensures_healthy_catalog_opencode_endpoint_without_spawning_duplicate_process() {
        let _guard = env_guard();
        let (endpoint, server) = start_health_server();
        let port = endpoint
            .rsplit(':')
            .next()
            .expect("endpoint should include a port")
            .to_string();
        let path = std::env::temp_dir().join(format!(
            "arroba-opencode-resolve-test-{}-healthy-catalog",
            std::process::id()
        ));
        fs::write(&path, "#!/bin/sh\nsleep 60\n").expect("fixture should exist");
        std::env::set_var("ARROBA_OPENCODE_BIN", &path);
        std::env::set_var("ARROBA_OPENCODE_PORT", &port);

        let resolved =
            ensure_opencode_catalog_endpoint().expect("healthy catalog endpoint should resolve");

        std::env::remove_var("ARROBA_OPENCODE_BIN");
        std::env::remove_var("ARROBA_OPENCODE_PORT");
        let _ = fs::remove_file(&path);
        let _ = server.join();

        assert_eq!(resolved, endpoint);
    }
}
