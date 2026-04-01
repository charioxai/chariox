use std::env;
use std::path::{Path, PathBuf};

use crate::error::DaemonError;
use crate::provider::{AgentEndpointMode, ProviderLaunchResult};

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

pub fn plan_opencode_launch() -> Result<ProviderLaunchResult, DaemonError> {
    if let Some(endpoint) = env::var_os(OPENCODE_ENDPOINT_OVERRIDE) {
        let endpoint = endpoint.to_string_lossy().trim().to_string();
        if !endpoint.is_empty() {
            return Ok(ProviderLaunchResult {
                endpoint_mode: AgentEndpointMode::External,
                process_label: "opencode:endpoint".to_string(),
                pty_target: None,
                pty_program: None,
                pty_args: Vec::new(),
                working_directory: None,
                structured_endpoint: Some(endpoint),
            });
        }
    }

    let executable = resolve_opencode_executable()?;
    let port = resolve_opencode_port()?;
    let base_url = format!("http://127.0.0.1:{port}");

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
    use std::sync::{Mutex, OnceLock};

    use crate::DaemonError;

    use crate::provider::AgentEndpointMode;

    use super::{plan_opencode_launch, resolve_opencode_executable};

    fn env_lock() -> &'static Mutex<()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
    }

    #[test]
    fn resolves_override_path_for_tests() {
        let _guard = env_lock().lock().expect("env lock should not be poisoned");
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
        let _guard = env_lock().lock().expect("env lock should not be poisoned");
        let path = std::env::temp_dir().join(format!(
            "arroba-opencode-resolve-test-{}-serve",
            std::process::id()
        ));
        fs::write(&path, "#!/bin/sh\nsleep 60\n").expect("fixture should exist");
        std::env::set_var("ARROBA_OPENCODE_BIN", &path);
        std::env::set_var("ARROBA_OPENCODE_PORT", "43111");

        let launch = plan_opencode_launch().expect("launch plan should resolve");

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
        let port = launch.pty_args[4]
            .parse::<u16>()
            .expect("port argument should be numeric");
        assert!(port >= 43111);
        let endpoint = format!("http://127.0.0.1:{port}");
        assert_eq!(
            launch.structured_endpoint.as_deref(),
            Some(endpoint.as_str())
        );
    }

    #[test]
    fn requires_explicit_opencode_port_override() {
        let _guard = env_lock().lock().expect("env lock should not be poisoned");
        let previous_bin = std::env::var_os("ARROBA_OPENCODE_BIN");
        let path = std::env::temp_dir().join(format!(
            "arroba-opencode-resolve-test-{}-missing-port",
            std::process::id()
        ));
        fs::write(&path, "#!/bin/sh\nsleep 60\n").expect("fixture should exist");
        std::env::set_var("ARROBA_OPENCODE_BIN", &path);
        let previous_port = std::env::var_os("ARROBA_OPENCODE_PORT");
        std::env::remove_var("ARROBA_OPENCODE_PORT");

        let error = plan_opencode_launch().expect_err("missing override should fail");

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
        let _guard = env_lock().lock().expect("env lock should not be poisoned");
        std::env::set_var("ARROBA_OPENCODE_ENDPOINT", "http://127.0.0.1:43119");
        std::env::remove_var("ARROBA_OPENCODE_BIN");
        std::env::remove_var("ARROBA_OPENCODE_PORT");

        let launch = plan_opencode_launch().expect("external endpoint should resolve");

        std::env::remove_var("ARROBA_OPENCODE_ENDPOINT");

        assert_eq!(launch.endpoint_mode, AgentEndpointMode::External);
        assert_eq!(launch.pty_program, None);
        assert_eq!(launch.pty_args, Vec::<String>::new());
        assert_eq!(
            launch.structured_endpoint.as_deref(),
            Some("http://127.0.0.1:43119")
        );
    }
}
