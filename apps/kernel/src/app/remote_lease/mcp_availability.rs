use crate::error::DaemonError;
use crate::execution_lease::LeasedAgent;
use crate::provider::{ProviderRunState, RuntimeProviderRun};
use crate::transport::relay_peer::{
    RemoteMcpAvailability, RemoteMcpAvailabilityStatus, RequiredRemoteMcp,
};

pub(super) fn validate_worker_mcp_runtime(
    config: &crate::mcp::ArrobaMcpServerConfig,
) -> RemoteMcpAvailabilityStatus {
    match &config.transport {
        crate::mcp::ArrobaMcpTransportConfig::Stdio {
            command,
            env_vars,
            cwd,
            ..
        } => {
            let missing_env = env_vars
                .iter()
                .filter(|name| std::env::var_os(name.as_str()).is_none())
                .cloned()
                .collect::<Vec<_>>();
            if !missing_env.is_empty() {
                return RemoteMcpAvailabilityStatus::MissingEnv { names: missing_env };
            }
            if let Some(cwd) = cwd {
                if !cwd.exists() {
                    return RemoteMcpAvailabilityStatus::Invalid {
                        reason: format!("cwd `{}` does not exist on worker", cwd.display()),
                    };
                }
            }
            if !command_is_available(command, cwd.as_deref()) {
                return RemoteMcpAvailabilityStatus::MissingCommand {
                    command: command.clone(),
                };
            }
            RemoteMcpAvailabilityStatus::Available
        }
        crate::mcp::ArrobaMcpTransportConfig::StreamableHttp {
            bearer_token_env_var,
            env_http_headers,
            ..
        } => {
            let mut missing_env = Vec::new();
            if let Some(name) = bearer_token_env_var {
                if std::env::var_os(name).is_none() {
                    missing_env.push(name.clone());
                }
            }
            for name in env_http_headers.values() {
                if std::env::var_os(name).is_none() {
                    missing_env.push(name.clone());
                }
            }
            missing_env.sort();
            missing_env.dedup();
            if !missing_env.is_empty() {
                return RemoteMcpAvailabilityStatus::MissingEnv { names: missing_env };
            }
            RemoteMcpAvailabilityStatus::Available
        }
    }
}

pub(super) fn provider_run_mcp_set_matches(
    run: &RuntimeProviderRun,
    required_mcps: &[RequiredRemoteMcp],
) -> Result<bool, DaemonError> {
    if run.state() == ProviderRunState::Ended {
        return Ok(false);
    }
    let mut current = run
        .mcp_servers()
        .iter()
        .map(|config| Ok((config.name.clone(), config.definition_hash()?)))
        .collect::<Result<Vec<_>, DaemonError>>()?;
    let mut required = required_mcps
        .iter()
        .map(|required| {
            (
                required.config.name.clone(),
                required.definition_hash.clone(),
            )
        })
        .collect::<Vec<_>>();
    current.sort();
    required.sort();
    Ok(current == required)
}

fn command_is_available(command: &str, cwd: Option<&std::path::Path>) -> bool {
    let command_path = std::path::PathBuf::from(command);
    if command_path.is_absolute() || command_path.components().count() > 1 {
        let candidate = if command_path.is_absolute() {
            command_path
        } else if let Some(cwd) = cwd {
            cwd.join(command_path)
        } else {
            command_path
        };
        return candidate.is_file();
    }
    let Some(path_var) = std::env::var_os("PATH") else {
        return false;
    };
    std::env::split_paths(&path_var)
        .map(|directory| directory.join(&command_path))
        .any(|candidate| candidate.is_file())
}

pub(super) fn format_remote_mcp_unavailable_message(
    leased_agent: &LeasedAgent,
    unavailable: &[RemoteMcpAvailability],
) -> String {
    let details = unavailable
        .iter()
        .map(|entry| {
            let status = match &entry.status {
                RemoteMcpAvailabilityStatus::Available => "available".to_string(),
                RemoteMcpAvailabilityStatus::Missing => "missing on worker".to_string(),
                RemoteMcpAvailabilityStatus::DefinitionMismatch { worker_hash } => {
                    format!("definition mismatch; worker has {worker_hash}")
                }
                RemoteMcpAvailabilityStatus::MissingCommand { command } => {
                    format!("missing command `{command}` on worker")
                }
                RemoteMcpAvailabilityStatus::MissingEnv { names } => {
                    format!(
                        "missing environment variable(s) on worker: {}",
                        names.join(", ")
                    )
                }
                RemoteMcpAvailabilityStatus::Invalid { reason } => reason.clone(),
            };
            format!(
                "- {} expected hash {}: {}",
                entry.name, entry.expected_hash, status
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    format!(
        "remote agent `{}` requires MCPs that are not available in worker Arroba. Install the matching MCP definition in the worker project or user registry, then retry.\n{}",
        leased_agent.id, details
    )
}
