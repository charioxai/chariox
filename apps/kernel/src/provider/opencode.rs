use std::collections::BTreeMap;
use std::env;
use std::path::{Path, PathBuf};

use crate::error::DaemonError;
use crate::provider::{AgentEndpointMode, LaunchProviderRequest, ProviderLaunchResult};

use self::mcp_config::runtime_mcp_env;
use self::ports::resolve_opencode_launch_port;

mod catalog_endpoint;
mod mcp_config;
mod ports;

pub use catalog_endpoint::{ensure_opencode_catalog_endpoint, opencode_catalog_endpoint};

const OPENCODE_ENV_OVERRIDE: &str = "ARROBA_OPENCODE_BIN";
const OPENCODE_BIND_HOST_OVERRIDE: &str = "ARROBA_OPENCODE_BIND_HOST";

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

pub fn plan_opencode_launch(
    request: Option<&LaunchProviderRequest>,
) -> Result<ProviderLaunchResult, DaemonError> {
    let _guard = crate::env_lock::lock();
    plan_opencode_launch_unlocked(request)
}

fn plan_opencode_launch_unlocked(
    request: Option<&LaunchProviderRequest>,
) -> Result<ProviderLaunchResult, DaemonError> {
    if let Some(endpoint) = request.and_then(|request| request.structured_endpoint.clone()) {
        let working_directory = request.and_then(|request| request.working_directory.clone());
        return Ok(ProviderLaunchResult {
            endpoint_mode: AgentEndpointMode::External,
            process_label: "opencode:native-server-proxy".to_string(),
            pty_target: None,
            pty_program: None,
            pty_args: Vec::new(),
            pty_env: BTreeMap::new(),
            pty_env_remove: request
                .map(|request| request.provider_env_remove.clone())
                .unwrap_or_default(),
            working_directory,
            structured_endpoint: Some(endpoint),
        });
    }

    if request.is_some() {
        let executable = resolve_opencode_executable_unlocked()?;
        let port = resolve_opencode_launch_port(true)?;
        let base_url = format!("http://127.0.0.1:{port}");
        return Ok(managed_launch(
            executable,
            port,
            resolve_opencode_bind_host(),
            base_url,
            runtime_mcp_env(request)?,
            request
                .map(|request| request.provider_env_remove.clone())
                .unwrap_or_default(),
        ));
    }

    let port = resolve_opencode_launch_port(false)?;
    let base_url = format!("http://127.0.0.1:{port}");
    let executable = resolve_opencode_executable_unlocked()?;

    Ok(managed_launch(
        executable,
        port,
        resolve_opencode_bind_host(),
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
    bind_host: String,
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
            bind_host,
            "--port".to_string(),
            port.to_string(),
        ],
        pty_env,
        pty_env_remove,
        working_directory: None,
        structured_endpoint: Some(base_url),
    }
}

fn resolve_opencode_bind_host() -> String {
    env::var(OPENCODE_BIND_HOST_OVERRIDE).unwrap_or_else(|_| "127.0.0.1".to_string())
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
    fn plans_external_launch_when_structured_endpoint_is_supplied() {
        let request =
            LaunchProviderRequest::new("session-1", "opencode", "opencode", "default", "default")
                .with_structured_endpoint("http://127.0.0.1:45678");

        let launch = plan_opencode_launch(Some(&request)).expect("launch plan should resolve");

        assert_eq!(launch.endpoint_mode, AgentEndpointMode::External);
        assert_eq!(launch.pty_program, None);
        assert!(launch.pty_args.is_empty());
        assert_eq!(
            launch.structured_endpoint.as_deref(),
            Some("http://127.0.0.1:45678"),
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
        assert!(config.contains("\"oauth\":false"));
        assert!(config.contains("\"timeout\":45000"));
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
        assert!(config.contains("\"oauth\":false"));
        assert!(config.contains("\"timeout\":45000"));
        assert!(!config.contains("@playwright/mcp@latest"));
    }

    #[test]
    fn plans_required_workspace_live_sync_launch() {
        let _guard = env_guard();
        let path = std::env::temp_dir().join(format!(
            "arroba-opencode-resolve-test-{}-workspace-live-sync",
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
        .with_workspace_live_sync_managed();

        let launch = plan_opencode_launch(Some(&request))
            .expect("workspace live sync launch should resolve");

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
