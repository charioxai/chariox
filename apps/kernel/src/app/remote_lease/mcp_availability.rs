use crate::error::DaemonError;
use crate::execution_lease::LeasedAgent;
use crate::provider::{ProviderRunState, RuntimeProviderRun};
use crate::transport::relay_peer::{
    RemoteMcpAvailability, RemoteMcpAvailabilityStatus, RemoteMcpCheckContext, RequiredRemoteMcp,
};

use super::RemoteLeaseRuntime;

fn validate_worker_mcp_runtime(
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

fn format_remote_mcp_unavailable_message(
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
        "remote agent `{}` requires MCPs that are not available in worker Arroba. Install the matching MCP definition in the worker project or user registry, or revoke the worker-local MCP grant and expose the home MCP as a home-proxy extension, then retry.\n{}",
        leased_agent.id, details
    )
}

#[cfg(test)]
mod tests {
    use super::format_remote_mcp_unavailable_message;
    use crate::execution_lease::LeasedAgent;
    use crate::transport::relay_peer::{RemoteMcpAvailability, RemoteMcpAvailabilityStatus};

    #[test]
    fn remote_lease_mcp_unavailable_message_names_worker_local_and_home_proxy_recovery() {
        let leased_agent = LeasedAgent::new(
            "leased-agent-1".to_string(),
            "lease-1".to_string(),
            "home-agent-1".to_string(),
            "codex".to_string(),
            None,
            None,
            None,
            None,
            "worker-session-1".to_string(),
            "worker-agent-1".to_string(),
            "attachment-1".to_string(),
        );
        let unavailable = vec![RemoteMcpAvailability {
            name: "filesystem".to_string(),
            expected_hash: "hash-1".to_string(),
            status: RemoteMcpAvailabilityStatus::MissingEnv {
                names: vec!["FS_ROOT".to_string()],
            },
        }];
        let message = format_remote_mcp_unavailable_message(&leased_agent, &unavailable);
        assert!(message.contains("remote agent `leased-agent-1` requires MCPs"));
        assert!(message.contains(
            "Install the matching MCP definition in the worker project or user registry"
        ));
        assert!(message.contains("expose the home MCP as a home-proxy extension"));
        assert!(message.contains(
            "- filesystem expected hash hash-1: missing environment variable(s) on worker: FS_ROOT"
        ));
    }
}

impl<'a> RemoteLeaseRuntime<'a> {
    pub(crate) fn check_remote_mcp_availability(
        &mut self,
        context: RemoteMcpCheckContext,
        required_mcps: Vec<RequiredRemoteMcp>,
    ) -> Result<Vec<RemoteMcpAvailability>, DaemonError> {
        let leased_agent = self
            .app
            .leased_agents
            .get(&context.leased_agent_id)
            .cloned()
            .ok_or_else(|| DaemonError::LeasedAgentNotFound {
                leased_agent_id: context.leased_agent_id.clone(),
            })?;
        self.validate_mcp_check_context(&leased_agent, &context)?;
        self.ensure_isolated_required_mcp_definitions(&leased_agent, &required_mcps)?;
        Ok(self.remote_mcp_availability_for_leased_agent(&leased_agent, &required_mcps))
    }

    pub(super) fn ensure_required_remote_mcps_available(
        &mut self,
        leased_agent: &LeasedAgent,
        required_mcps: &[RequiredRemoteMcp],
    ) -> Result<(), DaemonError> {
        self.ensure_isolated_required_mcp_definitions(leased_agent, required_mcps)?;
        let unavailable = self
            .remote_mcp_availability_for_leased_agent(leased_agent, required_mcps)
            .into_iter()
            .filter(|result| !matches!(result.status, RemoteMcpAvailabilityStatus::Available))
            .collect::<Vec<_>>();
        if unavailable.is_empty() {
            return Ok(());
        }
        Err(DaemonError::LocalTransport {
            operation: "remote mcp availability",
            message: format_remote_mcp_unavailable_message(leased_agent, &unavailable),
        })
    }

    fn ensure_isolated_required_mcp_definitions(
        &mut self,
        leased_agent: &LeasedAgent,
        required_mcps: &[RequiredRemoteMcp],
    ) -> Result<(), DaemonError> {
        if required_mcps.is_empty()
            || std::env::var_os("ARROBA_CAPABILITY_ISOLATION_ROOT")
                .filter(|value| !value.is_empty())
                .is_none()
        {
            return Ok(());
        }
        let session = self
            .app
            .sessions
            .get_session(&leased_agent.backing_session_id)?;
        let registry =
            crate::mcp::ArrobaMcpRegistry::new(vec![crate::mcp::ArrobaMcpRegistry::project_root(
                session.worktree_id(),
            )]);
        for required in required_mcps {
            registry.install(&required.config)?;
        }
        Ok(())
    }

    fn validate_mcp_check_context(
        &self,
        leased_agent: &LeasedAgent,
        context: &RemoteMcpCheckContext,
    ) -> Result<(), DaemonError> {
        let lease = self
            .app
            .execution_leases
            .get(&leased_agent.lease_id)
            .cloned()
            .ok_or_else(|| DaemonError::ExecutionLeaseNotFound {
                lease_id: leased_agent.lease_id.clone(),
            })?;
        if lease.home_kernel_id != context.home_kernel_id
            || lease.home_session_id != context.home_session_id
            || lease.home_agent_id != context.home_agent_id
        {
            return Err(DaemonError::LocalTransport {
                operation: "check remote MCP availability",
                message: "remote MCP check context does not match leased agent".to_string(),
            });
        }
        Ok(())
    }

    fn remote_mcp_availability_for_leased_agent(
        &self,
        leased_agent: &LeasedAgent,
        required_mcps: &[RequiredRemoteMcp],
    ) -> Vec<RemoteMcpAvailability> {
        let session = match self
            .app
            .sessions
            .get_session(&leased_agent.backing_session_id)
        {
            Ok(session) => session,
            Err(error) => {
                return required_mcps
                    .iter()
                    .map(|required| RemoteMcpAvailability {
                        name: required.config.name.clone(),
                        expected_hash: required.definition_hash.clone(),
                        status: RemoteMcpAvailabilityStatus::Invalid {
                            reason: error.to_string(),
                        },
                    })
                    .collect();
            }
        };
        let mut roots = vec![crate::mcp::ArrobaMcpRegistry::project_root(
            session.worktree_id(),
        )];
        if let Some(user_root) = crate::mcp::ArrobaMcpRegistry::user_root() {
            roots.push(user_root);
        }
        let registry = crate::mcp::ArrobaMcpRegistry::new(roots);
        required_mcps
            .iter()
            .map(|required| {
                let status = match registry.get(&required.config.name) {
                    Ok(Some(worker_config)) => match worker_config.definition_hash() {
                        Ok(worker_hash) if worker_hash == required.definition_hash => {
                            validate_worker_mcp_runtime(&worker_config)
                        }
                        Ok(worker_hash) => {
                            RemoteMcpAvailabilityStatus::DefinitionMismatch { worker_hash }
                        }
                        Err(error) => RemoteMcpAvailabilityStatus::Invalid {
                            reason: error.to_string(),
                        },
                    },
                    Ok(None) => RemoteMcpAvailabilityStatus::Missing,
                    Err(error) => RemoteMcpAvailabilityStatus::Invalid {
                        reason: error.to_string(),
                    },
                };
                RemoteMcpAvailability {
                    name: required.config.name.clone(),
                    expected_hash: required.definition_hash.clone(),
                    status,
                }
            })
            .collect()
    }
}
