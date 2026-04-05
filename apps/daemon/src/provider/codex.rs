use std::collections::BTreeMap;
use std::env;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread::sleep;
use std::time::{Duration, Instant};

use crate::error::DaemonError;
use crate::provider::{AgentEndpointMode, LaunchProviderRequest, ProviderLaunchResult};

use super::codex_client::codex_endpoint_is_healthy;

const CODEX_ENV_OVERRIDE: &str = "ARROBA_CODEX_BIN";
const CODEX_PORT_OVERRIDE: &str = "ARROBA_CODEX_PORT";
const CODEX_ENDPOINT_OVERRIDE: &str = "ARROBA_CODEX_ENDPOINT";
const CODEX_MCP_TOKEN_ENV: &str = "ARROBA_MCP_TOKEN";

pub fn resolve_codex_executable() -> Result<PathBuf, DaemonError> {
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
    if let Some(endpoint) = env::var_os(CODEX_ENDPOINT_OVERRIDE) {
        let endpoint = endpoint.to_string_lossy().trim().to_string();
        if !endpoint.is_empty() {
            return Ok(external_launch(endpoint));
        }
    }

    let port = resolve_codex_port()?;
    let endpoint = format!("ws://127.0.0.1:{port}");
    if codex_endpoint_is_healthy(&endpoint) {
        return Ok(external_launch(endpoint));
    }

    let executable = resolve_codex_executable()?;
    let (config_args, env) = runtime_mcp_config(request);
    Ok(ProviderLaunchResult {
        endpoint_mode: AgentEndpointMode::Managed,
        process_label: "codex:app-server".to_string(),
        pty_target: None,
        pty_program: Some(executable.display().to_string()),
        pty_args: {
            let mut args = vec!["app-server".to_string()];
            args.extend(config_args);
            args.extend(["--listen".to_string(), endpoint.clone()]);
            args
        },
        pty_env: env,
        working_directory: None,
        structured_endpoint: Some(endpoint),
    })
}

pub fn codex_catalog_endpoint() -> Result<String, DaemonError> {
    if let Some(endpoint) = env::var_os(CODEX_ENDPOINT_OVERRIDE) {
        let endpoint = endpoint.to_string_lossy().trim().to_string();
        if !endpoint.is_empty() {
            return Ok(endpoint);
        }
    }

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
    let mut child = command
        .spawn()
        .map_err(|error| DaemonError::LocalTransport {
            operation: "ensure_codex_catalog_endpoint",
            message: format!("failed to start Codex app-server: {error}"),
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
            return Err(DaemonError::LocalTransport {
                operation: "ensure_codex_catalog_endpoint",
                message: format!("Codex app-server exited before becoming healthy: {status}"),
            });
        }
        if Instant::now() >= deadline {
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

fn external_launch(endpoint: String) -> ProviderLaunchResult {
    ProviderLaunchResult {
        endpoint_mode: AgentEndpointMode::External,
        process_label: "codex:endpoint".to_string(),
        pty_target: None,
        pty_program: None,
        pty_args: Vec::new(),
        pty_env: BTreeMap::new(),
        working_directory: None,
        structured_endpoint: Some(endpoint),
    }
}

fn runtime_mcp_config(
    request: Option<&LaunchProviderRequest>,
) -> (Vec<String>, BTreeMap<String, String>) {
    let Some(binding) = request.and_then(|request| request.runtime_mcp_binding.as_ref()) else {
        return (Vec::new(), BTreeMap::new());
    };
    let args = vec![
        "-c".to_string(),
        format!("mcp_servers.arroba.url={:?}", binding.server_url),
        "-c".to_string(),
        format!(
            "mcp_servers.arroba.bearer_token_env_var={:?}",
            CODEX_MCP_TOKEN_ENV
        ),
    ];
    let mut env = BTreeMap::new();
    env.insert(CODEX_MCP_TOKEN_ENV.to_string(), binding.auth_token.clone());
    (args, env)
}

fn resolve_codex_port() -> Result<u16, DaemonError> {
    let Some(value) = env::var_os(CODEX_PORT_OVERRIDE) else {
        return Err(DaemonError::InvalidConfig {
            field: "ARROBA_CODEX_PORT",
            message: "must be set to an explicit Codex app-server TCP port",
        });
    };

    let value = value.to_string_lossy().into_owned();
    value
        .parse::<u16>()
        .map_err(|_| DaemonError::InvalidConfig {
            field: "ARROBA_CODEX_PORT",
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
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;
    use std::sync::{Mutex, MutexGuard, OnceLock};

    use crate::provider::{AgentEndpointMode, LaunchProviderRequest, RuntimeMcpBinding};

    use super::{logout_codex, plan_codex_launch, resolve_codex_executable};

    fn env_lock() -> &'static Mutex<()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
    }

    fn env_guard() -> MutexGuard<'static, ()> {
        env_lock()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
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
