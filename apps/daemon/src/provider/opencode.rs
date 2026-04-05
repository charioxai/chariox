use std::collections::BTreeMap;
use std::env;
use std::path::{Path, PathBuf};

use crate::error::DaemonError;
use crate::provider::{
    AgentEndpointMode, LaunchProviderRequest, OpenCodeClient, ProviderLaunchResult,
};

const OPENCODE_ENV_OVERRIDE: &str = "ARROBA_OPENCODE_BIN";
const OPENCODE_PORT_OVERRIDE: &str = "ARROBA_OPENCODE_PORT";
const OPENCODE_ENDPOINT_OVERRIDE: &str = "ARROBA_OPENCODE_ENDPOINT";

pub fn resolve_opencode_executable() -> Result<PathBuf, DaemonError> {
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
    if let Some(endpoint) = env::var_os(OPENCODE_ENDPOINT_OVERRIDE) {
        let endpoint = endpoint.to_string_lossy().trim().to_string();
        if !endpoint.is_empty() {
            return Ok(external_launch(endpoint));
        }
    }

    let port = resolve_opencode_port()?;
    let base_url = format!("http://127.0.0.1:{port}");
    if endpoint_is_healthy(&base_url) {
        return Ok(external_launch(base_url));
    }

    let executable = resolve_opencode_executable()?;

    Ok(ProviderLaunchResult {
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
        pty_env: runtime_mcp_env(request),
        working_directory: None,
        structured_endpoint: Some(base_url),
    })
}

pub fn opencode_catalog_endpoint() -> Result<String, DaemonError> {
    if let Some(endpoint) = env::var_os(OPENCODE_ENDPOINT_OVERRIDE) {
        let endpoint = endpoint.to_string_lossy().trim().to_string();
        if !endpoint.is_empty() {
            return Ok(endpoint);
        }
    }

    let port = resolve_opencode_port()?;
    Ok(format!("http://127.0.0.1:{port}"))
}

fn external_launch(endpoint: String) -> ProviderLaunchResult {
    ProviderLaunchResult {
        endpoint_mode: AgentEndpointMode::External,
        process_label: "opencode:endpoint".to_string(),
        pty_target: None,
        pty_program: None,
        pty_args: Vec::new(),
        pty_env: BTreeMap::new(),
        working_directory: None,
        structured_endpoint: Some(endpoint),
    }
}

fn runtime_mcp_env(request: Option<&LaunchProviderRequest>) -> BTreeMap<String, String> {
    let mut env = BTreeMap::new();
    let Some(binding) = request.and_then(|request| request.runtime_mcp_binding.as_ref()) else {
        return env;
    };
    let config = serde_json::json!({
        "mcp": {
            "arroba": {
                "type": "remote",
                "url": binding.server_url,
                "enabled": true,
                "headers": {
                    "Authorization": format!("Bearer {}", binding.auth_token),
                }
            }
        }
    });
    env.insert(OPENCODE_CONFIG_CONTENT_ENV.to_string(), config.to_string());
    env
}

fn endpoint_is_healthy(base_url: &str) -> bool {
    OpenCodeClient::new("catalog", base_url)
        .and_then(|client| client.check_health())
        .is_ok()
}

fn resolve_opencode_port() -> Result<u16, DaemonError> {
    let Some(value) = env::var_os(OPENCODE_PORT_OVERRIDE) else {
        return Err(DaemonError::InvalidConfig {
            field: "ARROBA_OPENCODE_PORT",
            message: "must be set to an explicit OpenCode server TCP port",
        });
    };

    let value = value.to_string_lossy().into_owned();
    value
        .parse::<u16>()
        .map_err(|_| DaemonError::InvalidConfig {
            field: "ARROBA_OPENCODE_PORT",
            message: "must be a valid TCP port",
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
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::{Mutex, MutexGuard, OnceLock};
    use std::thread;

    use crate::DaemonError;

    use crate::provider::{AgentEndpointMode, LaunchProviderRequest, RuntimeMcpBinding};

    use super::{plan_opencode_launch, resolve_opencode_executable};

    fn env_lock() -> &'static Mutex<()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
    }

    fn env_guard() -> MutexGuard<'static, ()> {
        env_lock()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
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
        let port = reserve_unused_port();
        std::env::set_var("ARROBA_OPENCODE_PORT", port.to_string());

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
    fn requires_explicit_opencode_port_override() {
        let _guard = env_guard();
        let previous_bin = std::env::var_os("ARROBA_OPENCODE_BIN");
        let path = std::env::temp_dir().join(format!(
            "arroba-opencode-resolve-test-{}-missing-port",
            std::process::id()
        ));
        fs::write(&path, "#!/bin/sh\nsleep 60\n").expect("fixture should exist");
        std::env::set_var("ARROBA_OPENCODE_BIN", &path);
        let previous_port = std::env::var_os("ARROBA_OPENCODE_PORT");
        std::env::remove_var("ARROBA_OPENCODE_PORT");

        let error = plan_opencode_launch(None).expect_err("missing override should fail");

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

        match error {
            DaemonError::InvalidConfig { field, message } => {
                assert_eq!(field, "ARROBA_OPENCODE_PORT");
                assert_eq!(
                    message,
                    "must be set to an explicit OpenCode server TCP port"
                );
            }
            other => panic!("unexpected error: {other}"),
        }
    }

    #[test]
    fn resolves_external_opencode_endpoint_without_launching_process() {
        let _guard = env_guard();
        std::env::set_var("ARROBA_OPENCODE_ENDPOINT", "http://127.0.0.1:43119");
        std::env::remove_var("ARROBA_OPENCODE_BIN");
        std::env::remove_var("ARROBA_OPENCODE_PORT");

        let launch = plan_opencode_launch(None).expect("external endpoint should resolve");

        std::env::remove_var("ARROBA_OPENCODE_ENDPOINT");

        assert_eq!(launch.endpoint_mode, AgentEndpointMode::External);
        assert_eq!(launch.pty_program, None);
        assert_eq!(launch.pty_args, Vec::<String>::new());
        assert_eq!(
            launch.structured_endpoint.as_deref(),
            Some("http://127.0.0.1:43119")
        );
    }

    #[test]
    fn reuses_healthy_shared_opencode_endpoint_without_launching_process() {
        let _guard = env_guard();
        let (endpoint, server) = start_health_server();
        let port = endpoint
            .rsplit(':')
            .next()
            .expect("endpoint should include a port")
            .to_string();
        let path = std::env::temp_dir().join(format!(
            "arroba-opencode-resolve-test-{}-reused-endpoint",
            std::process::id()
        ));
        fs::write(&path, "#!/bin/sh\nsleep 60\n").expect("fixture should exist");
        std::env::set_var("ARROBA_OPENCODE_BIN", &path);
        std::env::set_var("ARROBA_OPENCODE_PORT", &port);
        std::env::remove_var("ARROBA_OPENCODE_ENDPOINT");

        let launch = plan_opencode_launch(None).expect("healthy endpoint should be reused");

        std::env::remove_var("ARROBA_OPENCODE_BIN");
        std::env::remove_var("ARROBA_OPENCODE_PORT");
        let _ = fs::remove_file(&path);
        let _ = server.join();

        assert_eq!(launch.endpoint_mode, AgentEndpointMode::External);
        assert_eq!(launch.pty_program, None);
        assert_eq!(launch.pty_args, Vec::<String>::new());
        assert_eq!(
            launch.structured_endpoint.as_deref(),
            Some(endpoint.as_str())
        );
    }
}
