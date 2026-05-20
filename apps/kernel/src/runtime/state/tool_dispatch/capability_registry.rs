use crate::error::DaemonError;

pub(super) fn skill_registry_for_workspace(workspace: &str) -> crate::skill::ArrobaSkillRegistry {
    let workspace = std::path::PathBuf::from(workspace);
    let mut roots = vec![crate::skill::ArrobaSkillRegistry::project_root(&workspace)];
    if let Some(user_root) = crate::skill::ArrobaSkillRegistry::user_root() {
        roots.push(user_root);
    }
    crate::skill::ArrobaSkillRegistry::new(roots)
}

pub(super) fn mcp_registry_for_workspace(workspace: &str) -> crate::mcp::ArrobaMcpRegistry {
    let workspace = std::path::PathBuf::from(workspace);
    let mut roots = vec![crate::mcp::ArrobaMcpRegistry::project_root(&workspace)];
    if let Some(user_root) = crate::mcp::ArrobaMcpRegistry::user_root() {
        roots.push(user_root);
    }
    crate::mcp::ArrobaMcpRegistry::new(roots)
}

pub(super) fn script_registry_for_workspace(
    workspace: &str,
) -> crate::script::ArrobaScriptRegistry {
    let workspace = std::path::PathBuf::from(workspace);
    let mut roots = vec![crate::script::ArrobaScriptRegistry::project_root(
        &workspace,
    )];
    if let Some(user_root) = crate::script::ArrobaScriptRegistry::user_root() {
        roots.push(user_root);
    }
    crate::script::ArrobaScriptRegistry::new(roots)
}

pub(super) fn environment_registry_for_workspace(
    workspace: &str,
) -> crate::script::ArrobaEnvironmentRegistry {
    let workspace = std::path::PathBuf::from(workspace);
    let mut roots = vec![crate::script::ArrobaEnvironmentRegistry::project_root(
        &workspace,
    )];
    if let Some(user_root) = crate::script::ArrobaEnvironmentRegistry::user_root() {
        roots.push(user_root);
    }
    crate::script::ArrobaEnvironmentRegistry::new(roots)
}

pub(super) fn connector_registry() -> Result<crate::connector::ArrobaConnectorRegistry, DaemonError>
{
    crate::connector::ArrobaConnectorRegistry::user()
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
        "remote MCP unavailable on worker. Install the matching MCP definition in the worker project or user registry, then retry.\n{details}"
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
