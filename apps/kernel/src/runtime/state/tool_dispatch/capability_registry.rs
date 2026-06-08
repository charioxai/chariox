use crate::error::DaemonError;
use std::path::{Path, PathBuf};

pub(super) fn resolve_registration_path(
    session: &crate::session::RuntimeSession,
    path: &Path,
) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        PathBuf::from(session.worktree_id()).join(path)
    }
}

pub(super) fn global_skill_registry() -> Result<crate::skill::ArrobaSkillRegistry, DaemonError> {
    Ok(crate::skill::ArrobaSkillRegistry::new(user_only_root(
        crate::skill::ArrobaSkillRegistry::user_root(),
        "skill registry root",
        "HOME must be set to resolve ~/.arroba/skills",
    )?))
}

pub(super) fn global_mcp_registry() -> Result<crate::mcp::ArrobaMcpRegistry, DaemonError> {
    Ok(crate::mcp::ArrobaMcpRegistry::new(user_only_root(
        crate::mcp::ArrobaMcpRegistry::user_root(),
        "MCP registry root",
        "HOME must be set to resolve ~/.arroba/mcps",
    )?))
}

pub(super) fn global_script_registry() -> Result<crate::script::ArrobaScriptRegistry, DaemonError> {
    Ok(crate::script::ArrobaScriptRegistry::new(user_only_root(
        crate::script::ArrobaScriptRegistry::user_root(),
        "script registry root",
        "HOME must be set to resolve ~/.arroba/scripts",
    )?))
}

pub(super) fn global_environment_registry(
) -> Result<crate::script::ArrobaEnvironmentRegistry, DaemonError> {
    Ok(crate::script::ArrobaEnvironmentRegistry::new(
        user_only_root(
            crate::script::ArrobaEnvironmentRegistry::user_root(),
            "environment registry root",
            "HOME must be set to resolve ~/.arroba/envs",
        )?,
    ))
}

fn user_only_root(
    root: Option<PathBuf>,
    field: &'static str,
    message: &'static str,
) -> Result<Vec<PathBuf>, DaemonError> {
    root.map(|root| vec![root])
        .ok_or(DaemonError::InvalidConfig { field, message })
}

pub(super) fn skill_registry_for_workspace(workspace: &str) -> crate::skill::ArrobaSkillRegistry {
    let _ = workspace;
    let roots = crate::skill::ArrobaSkillRegistry::user_root()
        .map(|root| vec![root])
        .unwrap_or_default();
    crate::skill::ArrobaSkillRegistry::new(roots)
}

pub(super) fn mcp_registry_for_workspace(workspace: &str) -> crate::mcp::ArrobaMcpRegistry {
    let _ = workspace;
    let roots = crate::mcp::ArrobaMcpRegistry::user_root()
        .map(|root| vec![root])
        .unwrap_or_default();
    crate::mcp::ArrobaMcpRegistry::new(roots)
}

pub(super) fn script_registry_for_workspace(
    workspace: &str,
) -> crate::script::ArrobaScriptRegistry {
    let _ = workspace;
    let roots = crate::script::ArrobaScriptRegistry::user_root()
        .map(|root| vec![root])
        .unwrap_or_default();
    crate::script::ArrobaScriptRegistry::new(roots)
}

pub(super) fn environment_registry_for_workspace(
    workspace: &str,
) -> crate::script::ArrobaEnvironmentRegistry {
    let _ = workspace;
    let roots = crate::script::ArrobaEnvironmentRegistry::user_root()
        .map(|root| vec![root])
        .unwrap_or_default();
    crate::script::ArrobaEnvironmentRegistry::new(roots)
}

pub(super) fn connector_registry() -> Result<crate::connector::ArrobaConnectorRegistry, DaemonError>
{
    crate::connector::ArrobaConnectorRegistry::user()
}

pub(super) fn connector_adapter_registry(
) -> Result<crate::connector::ArrobaConnectorAdapterRegistry, DaemonError> {
    crate::connector::ArrobaConnectorAdapterRegistry::user()
}

pub(super) fn required_remote_mcps(
    registry: &crate::mcp::ArrobaMcpRegistry,
    grants: &[String],
) -> Result<Vec<crate::transport::relay_peer::RequiredRemoteMcp>, DaemonError> {
    grants
        .iter()
        .map(|grant| {
            let config = registry
                .get(grant)?
                .ok_or_else(|| DaemonError::LocalTransport {
                    operation: "required remote MCPs",
                    message: format!("MCP `{grant}` is granted but is not installed"),
                })?;
            let definition_hash = config.definition_hash()?;
            Ok(crate::transport::relay_peer::RequiredRemoteMcp {
                config,
                definition_hash,
            })
        })
        .collect()
}

pub(super) fn format_remote_mcp_unavailable(
    unavailable: &[&crate::transport::relay_peer::RemoteMcpAvailability],
) -> String {
    let details =
        unavailable
            .iter()
            .map(|entry| {
                let status = match &entry.status {
                crate::transport::relay_peer::RemoteMcpAvailabilityStatus::Available => {
                    "available".to_string()
                }
                crate::transport::relay_peer::RemoteMcpAvailabilityStatus::Missing => {
                    "missing on worker".to_string()
                }
                crate::transport::relay_peer::RemoteMcpAvailabilityStatus::DefinitionMismatch {
                    worker_hash,
                } => format!("definition mismatch; worker has {worker_hash}"),
                crate::transport::relay_peer::RemoteMcpAvailabilityStatus::MissingCommand {
                    command,
                } => format!("missing command `{command}` on worker"),
                crate::transport::relay_peer::RemoteMcpAvailabilityStatus::MissingEnv { names } => {
                    format!("missing environment variable(s) on worker: {}", names.join(", "))
                }
                crate::transport::relay_peer::RemoteMcpAvailabilityStatus::Invalid { reason } => {
                    reason.clone()
                }
            };
                format!(
                    "- {} expected hash {}: {}",
                    entry.name, entry.expected_hash, status
                )
            })
            .collect::<Vec<_>>()
            .join("\n");
    format!(
        "remote MCP unavailable on worker. Install the matching MCP definition in the worker global registry, or revoke the worker-local MCP grant and expose the home MCP as a home-proxy extension, then retry.\n{details}"
    )
}

pub(super) fn package_granted_skills(
    registry: &crate::skill::ArrobaSkillRegistry,
    grants: &[String],
) -> Result<Vec<crate::skill::ArrobaSkillPackage>, DaemonError> {
    grants
        .iter()
        .map(|grant| {
            registry
                .package(grant)?
                .ok_or_else(|| DaemonError::LocalTransport {
                    operation: "package granted skill",
                    message: format!("skill `{grant}` is granted but is not installed"),
                })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::format_remote_mcp_unavailable;
    use crate::transport::relay_peer::{RemoteMcpAvailability, RemoteMcpAvailabilityStatus};

    #[test]
    fn remote_mcp_unavailable_message_names_worker_local_and_home_proxy_recovery() {
        let entry = RemoteMcpAvailability {
            name: "filesystem".to_string(),
            expected_hash: "hash-1".to_string(),
            status: RemoteMcpAvailabilityStatus::MissingCommand {
                command: "fs-mcp".to_string(),
            },
        };
        let message = format_remote_mcp_unavailable(&[&entry]);
        assert!(
            message.contains("Install the matching MCP definition in the worker global registry")
        );
        assert!(message.contains("expose the home MCP as a home-proxy extension"));
        assert!(message
            .contains("- filesystem expected hash hash-1: missing command `fs-mcp` on worker"));
    }
}
