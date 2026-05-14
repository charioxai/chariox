use std::path::PathBuf;

use crate::error::DaemonError;
use crate::local::{
    GetMcpServerRequest, GetSkillRequest, ImportMcpServersRequest, ImportSkillsRequest,
    InstallMcpServerRequest, InstallSkillRequest, ListMcpServersRequest, ListSkillsRequest,
    LocalDaemonResponse, UninstallMcpServerRequest, UninstallSkillRequest, UpdateMcpServerRequest,
    UpdateSkillRequest,
};

pub(crate) fn execute_install_mcp_server_request(
    request: InstallMcpServerRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    let registry =
        crate::mcp::ArrobaMcpRegistry::new(mcp_registry_roots(request.workspace_id.as_deref())?);
    let path = registry.install(&request.config)?;
    Ok(LocalDaemonResponse::McpServerInstalled {
        mcp: request.config,
        path,
    })
}

pub(crate) fn execute_update_mcp_server_request(
    request: UpdateMcpServerRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    let registry =
        crate::mcp::ArrobaMcpRegistry::new(mcp_registry_roots(request.workspace_id.as_deref())?);
    let path = registry.update(&request.config)?;
    Ok(LocalDaemonResponse::McpServerUpdated {
        mcp: request.config,
        path,
    })
}

pub(crate) fn execute_uninstall_mcp_server_request(
    request: UninstallMcpServerRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    let registry =
        crate::mcp::ArrobaMcpRegistry::new(mcp_registry_roots(request.workspace_id.as_deref())?);
    let path = registry.uninstall(&request.name)?;
    Ok(LocalDaemonResponse::McpServerUninstalled {
        name: request.name,
        path,
    })
}

pub(crate) fn execute_import_mcp_servers_request(
    request: ImportMcpServersRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    let registry =
        crate::mcp::ArrobaMcpRegistry::new(mcp_registry_roots(request.workspace_id.as_deref())?);
    let outcome = match request.provider.as_str() {
        "codex" => crate::mcp::import_codex_mcp_servers(&registry, request.name.as_deref())?,
        "opencode" => {
            let workspace = registry_workspace_root(request.workspace_id.as_deref())?;
            crate::mcp::import_opencode_mcp_servers(&registry, &workspace, request.name.as_deref())?
        }
        _ => {
            return Err(DaemonError::InvalidConfig {
                field: "provider",
                message: "only Codex and OpenCode MCP import are supported",
            });
        }
    };
    Ok(LocalDaemonResponse::McpServersImported { outcome })
}

pub(crate) fn execute_get_mcp_server_request(
    request: GetMcpServerRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    let registry =
        crate::mcp::ArrobaMcpRegistry::new(mcp_registry_roots(request.workspace_id.as_deref())?);
    let Some(mcp) = registry.get(&request.name)? else {
        return Err(DaemonError::LocalTransport {
            operation: "mcp.get",
            message: format!("MCP `{}` was not found", request.name),
        });
    };
    Ok(LocalDaemonResponse::McpServer { mcp })
}

pub(crate) fn execute_list_mcp_servers_request(
    request: ListMcpServersRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    let registry =
        crate::mcp::ArrobaMcpRegistry::new(mcp_registry_roots(request.workspace_id.as_deref())?);
    Ok(LocalDaemonResponse::McpServersListed {
        mcps: registry.list()?,
    })
}

pub(crate) fn execute_install_skill_request(
    request: InstallSkillRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    let workspace = registry_workspace_root(request.workspace_id.as_deref())?;
    let source_path = if request.source_path.is_absolute() {
        request.source_path
    } else {
        workspace.join(request.source_path)
    };
    let registry = crate::skill::ArrobaSkillRegistry::new(skill_registry_roots(
        request.workspace_id.as_deref(),
    )?);
    let (skill, path) = registry.install_from_path(&source_path)?;
    Ok(LocalDaemonResponse::SkillInstalled { skill, path })
}

pub(crate) fn execute_update_skill_request(
    request: UpdateSkillRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    let workspace = registry_workspace_root(request.workspace_id.as_deref())?;
    let source_path = if request.source_path.is_absolute() {
        request.source_path
    } else {
        workspace.join(request.source_path)
    };
    let registry = crate::skill::ArrobaSkillRegistry::new(skill_registry_roots(
        request.workspace_id.as_deref(),
    )?);
    let (skill, path) = registry.update_from_path(&source_path)?;
    Ok(LocalDaemonResponse::SkillUpdated { skill, path })
}

pub(crate) fn execute_uninstall_skill_request(
    request: UninstallSkillRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    let registry = crate::skill::ArrobaSkillRegistry::new(skill_registry_roots(
        request.workspace_id.as_deref(),
    )?);
    let (skill, path) = registry.uninstall(&request.name)?;
    Ok(LocalDaemonResponse::SkillUninstalled { skill, path })
}

pub(crate) fn execute_import_skills_request(
    request: ImportSkillsRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    let workspace = registry_workspace_root(request.workspace_id.as_deref())?;
    let registry = crate::skill::ArrobaSkillRegistry::new(skill_registry_roots(
        request.workspace_id.as_deref(),
    )?);
    let outcome = match request.provider.as_str() {
        "codex" => {
            crate::skill::import_codex_skills(&registry, &workspace, request.name.as_deref())?
        }
        "opencode" => {
            crate::skill::import_opencode_skills(&registry, &workspace, request.name.as_deref())?
        }
        _ => {
            return Err(DaemonError::InvalidConfig {
                field: "provider",
                message: "only Codex and OpenCode skill import are supported",
            });
        }
    };
    Ok(LocalDaemonResponse::SkillsImported { outcome })
}

pub(crate) fn execute_get_skill_request(
    request: GetSkillRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    let registry = crate::skill::ArrobaSkillRegistry::new(skill_registry_roots(
        request.workspace_id.as_deref(),
    )?);
    let Some(skill) = registry.get(&request.name)? else {
        return Err(DaemonError::LocalTransport {
            operation: "skill.get",
            message: format!("skill `{}` was not found", request.name),
        });
    };
    Ok(LocalDaemonResponse::Skill { skill })
}

pub(crate) fn execute_list_skills_request(
    request: ListSkillsRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    let registry = crate::skill::ArrobaSkillRegistry::new(skill_registry_roots(
        request.workspace_id.as_deref(),
    )?);
    Ok(LocalDaemonResponse::SkillsListed {
        skills: registry.list()?,
    })
}

pub(crate) fn ensure_mcp_exists(workspace_id: Option<&str>, name: &str) -> Result<(), DaemonError> {
    let registry = crate::mcp::ArrobaMcpRegistry::new(mcp_registry_roots(workspace_id)?);
    if registry.get(name)?.is_none() {
        return Err(DaemonError::LocalTransport {
            operation: "agent.capability.grant",
            message: format!("MCP `{name}` is not installed"),
        });
    }
    Ok(())
}

pub(crate) fn ensure_skill_exists(
    workspace_id: Option<&str>,
    name: &str,
) -> Result<(), DaemonError> {
    let registry = crate::skill::ArrobaSkillRegistry::new(skill_registry_roots(workspace_id)?);
    if registry.get(name)?.is_none() {
        return Err(DaemonError::LocalTransport {
            operation: "agent.capability.grant",
            message: format!("skill `{name}` is not installed"),
        });
    }
    Ok(())
}

fn mcp_registry_roots(workspace_id: Option<&str>) -> Result<Vec<PathBuf>, DaemonError> {
    let workspace = registry_workspace_root(workspace_id)?;
    let mut roots = vec![crate::mcp::ArrobaMcpRegistry::project_root(&workspace)];
    if let Some(root) = crate::mcp::ArrobaMcpRegistry::user_root() {
        roots.push(root);
    }
    Ok(roots)
}

fn skill_registry_roots(workspace_id: Option<&str>) -> Result<Vec<PathBuf>, DaemonError> {
    let workspace = registry_workspace_root(workspace_id)?;
    let mut roots = vec![crate::skill::ArrobaSkillRegistry::project_root(&workspace)];
    if let Some(root) = crate::skill::ArrobaSkillRegistry::user_root() {
        roots.push(root);
    }
    Ok(roots)
}

fn registry_workspace_root(workspace_id: Option<&str>) -> Result<PathBuf, DaemonError> {
    match workspace_id {
        Some(value) if !value.trim().is_empty() => Ok(PathBuf::from(value)),
        _ => std::env::current_dir().map_err(|error| DaemonError::LocalTransport {
            operation: "registry.roots",
            message: format!("failed to resolve current directory: {error}"),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_roots_start_with_workspace_project_roots() {
        let workspace = PathBuf::from("/tmp/arroba-capability-registry-workspace");
        let workspace_id = workspace.to_string_lossy();

        let mcp_roots = mcp_registry_roots(Some(workspace_id.as_ref())).unwrap();
        let skill_roots = skill_registry_roots(Some(workspace_id.as_ref())).unwrap();

        assert_eq!(
            mcp_roots.first(),
            Some(&crate::mcp::ArrobaMcpRegistry::project_root(&workspace))
        );
        assert_eq!(
            skill_roots.first(),
            Some(&crate::skill::ArrobaSkillRegistry::project_root(&workspace))
        );
    }

    #[test]
    fn registry_workspace_root_uses_current_directory_for_blank_workspace() {
        let current_dir = std::env::current_dir().unwrap();

        assert_eq!(registry_workspace_root(Some("   ")).unwrap(), current_dir);
        assert_eq!(registry_workspace_root(None).unwrap(), current_dir);
    }
}
