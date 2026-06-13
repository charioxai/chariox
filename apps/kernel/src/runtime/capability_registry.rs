use std::path::PathBuf;

use crate::error::DaemonError;
use crate::local::{
    GetConnectorAdapterRequest, GetConnectorRequest, GetCredentialRequest, GetEnvironmentRequest,
    GetMcpServerRequest, GetScriptRequest, GetSkillRequest, ImportMcpServersRequest,
    ImportSkillsRequest, InstallMcpServerRequest, InstallSkillRequest,
    ListConnectorAdaptersRequest, ListConnectorsRequest, ListCredentialsRequest,
    ListEnvironmentsRequest, ListMcpServersRequest, ListScriptsRequest, ListSkillsRequest,
    LocalDaemonRequest, LocalDaemonResponse, RegisterConnectorAdapterRequest,
    RegisterConnectorRequest, RegisterCredentialRequest, RegisterEnvironmentRequest,
    RegisterScriptRequest, RemoveConnectorAdapterRequest, RemoveConnectorRequest,
    RemoveCredentialRequest, RemoveEnvironmentRequest, RemoveScriptRequest, TestConnectorRequest,
    UninstallMcpServerRequest, UninstallSkillRequest, UpdateMcpServerRequest, UpdateSkillRequest,
    UpsertConnectorRequest, UpsertCredentialRequest, UpsertSkillRequest, ValidateScriptRequest,
};

pub(crate) fn execute_capability_registry_request(
    request: LocalDaemonRequest,
    credential_vault: crate::config::UserCredentialVaultConfig,
) -> Result<LocalDaemonResponse, DaemonError> {
    match request {
        LocalDaemonRequest::InstallMcpServer(request) => {
            execute_install_mcp_server_request(request)
        }
        LocalDaemonRequest::UpdateMcpServer(request) => execute_update_mcp_server_request(request),
        LocalDaemonRequest::UninstallMcpServer(request) => {
            execute_uninstall_mcp_server_request(request)
        }
        LocalDaemonRequest::ImportMcpServers(request) => {
            execute_import_mcp_servers_request(request)
        }
        LocalDaemonRequest::GetMcpServer(request) => execute_get_mcp_server_request(request),
        LocalDaemonRequest::ListMcpServers(request) => execute_list_mcp_servers_request(request),
        LocalDaemonRequest::RegisterEnvironment(request) => {
            execute_register_environment_request(request)
        }
        LocalDaemonRequest::RemoveEnvironment(request) => {
            execute_remove_environment_request(request)
        }
        LocalDaemonRequest::GetEnvironment(request) => execute_get_environment_request(request),
        LocalDaemonRequest::ListEnvironments(request) => execute_list_environments_request(request),
        LocalDaemonRequest::ValidateScript(request) => execute_validate_script_request(request),
        LocalDaemonRequest::RegisterScript(request) => execute_register_script_request(request),
        LocalDaemonRequest::RemoveScript(request) => execute_remove_script_request(request),
        LocalDaemonRequest::GetScript(request) => execute_get_script_request(request),
        LocalDaemonRequest::ListScripts(request) => execute_list_scripts_request(request),
        LocalDaemonRequest::RegisterCredential(request) => {
            execute_register_credential_request(request)
        }
        LocalDaemonRequest::UpsertCredential(request) => execute_upsert_credential_request(request),
        LocalDaemonRequest::RemoveCredential(request) => execute_remove_credential_request(request),
        LocalDaemonRequest::GetCredential(request) => execute_get_credential_request(request),
        LocalDaemonRequest::ListCredentials(request) => execute_list_credentials_request(request),
        LocalDaemonRequest::RegisterConnector(request) => {
            execute_register_connector_request(request)
        }
        LocalDaemonRequest::UpsertConnector(request) => execute_upsert_connector_request(request),
        LocalDaemonRequest::RegisterConnectorAdapter(request) => {
            execute_register_connector_adapter_request(request)
        }
        LocalDaemonRequest::RemoveConnectorAdapter(request) => {
            execute_remove_connector_adapter_request(request)
        }
        LocalDaemonRequest::GetConnectorAdapter(request) => {
            execute_get_connector_adapter_request(request)
        }
        LocalDaemonRequest::ListConnectorAdapters(request) => {
            execute_list_connector_adapters_request(request)
        }
        LocalDaemonRequest::RemoveConnector(request) => execute_remove_connector_request(request),
        LocalDaemonRequest::GetConnector(request) => execute_get_connector_request(request),
        LocalDaemonRequest::ListConnectors(request) => execute_list_connectors_request(request),
        LocalDaemonRequest::TestConnector(request) => {
            execute_test_connector_request(request, credential_vault)
        }
        LocalDaemonRequest::UpsertSkill(request) => execute_upsert_skill_request(request),
        LocalDaemonRequest::InstallSkill(request) => execute_install_skill_request(request),
        LocalDaemonRequest::UpdateSkill(request) => execute_update_skill_request(request),
        LocalDaemonRequest::UninstallSkill(request) => execute_uninstall_skill_request(request),
        LocalDaemonRequest::ImportSkills(request) => execute_import_skills_request(request),
        LocalDaemonRequest::GetSkill(request) => execute_get_skill_request(request),
        LocalDaemonRequest::ListSkills(request) => execute_list_skills_request(request),
        _ => Err(DaemonError::LocalTransport {
            operation: "capability registry request",
            message: "unsupported capability registry request".to_string(),
        }),
    }
}

pub(crate) fn execute_register_credential_request(
    request: RegisterCredentialRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    let registry = crate::credential::ArrobaCredentialRegistry::user()?;
    let (credential, path) = registry.install_from_file(&request.source_path)?;
    Ok(LocalDaemonResponse::CredentialRegistered { credential, path })
}

pub(crate) fn execute_upsert_credential_request(
    request: UpsertCredentialRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    let registry = crate::credential::ArrobaCredentialRegistry::user()?;
    let (credential, path) = registry.upsert(request.credential)?;
    Ok(LocalDaemonResponse::CredentialUpserted { credential, path })
}

pub(crate) fn execute_remove_credential_request(
    request: RemoveCredentialRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    let registry = crate::credential::ArrobaCredentialRegistry::user()?;
    let (credential, path) = registry.remove(&request.id)?;
    Ok(LocalDaemonResponse::CredentialRemoved { credential, path })
}

pub(crate) fn execute_get_credential_request(
    request: GetCredentialRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    let registry = crate::credential::ArrobaCredentialRegistry::user()?;
    let Some(credential) = registry.get(&request.id)? else {
        return Err(DaemonError::LocalTransport {
            operation: "credential.get",
            message: format!("credential `{}` was not found", request.id),
        });
    };
    Ok(LocalDaemonResponse::Credential { credential })
}

pub(crate) fn execute_list_credentials_request(
    _request: ListCredentialsRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    let registry = crate::credential::ArrobaCredentialRegistry::user()?;
    Ok(LocalDaemonResponse::CredentialsListed {
        credentials: registry.list()?,
    })
}

pub(crate) fn execute_register_connector_request(
    request: RegisterConnectorRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    let registry = crate::connector::ArrobaConnectorRegistry::user()?;
    let adapters = crate::connector::ArrobaConnectorAdapterRegistry::user()?;
    let (connector, path) = registry.install_from_file(&request.source_path, &adapters)?;
    Ok(LocalDaemonResponse::ConnectorRegistered { connector, path })
}

pub(crate) fn execute_upsert_connector_request(
    request: UpsertConnectorRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    let registry = crate::connector::ArrobaConnectorRegistry::user()?;
    let adapters = crate::connector::ArrobaConnectorAdapterRegistry::user()?;
    let (connector, path) = registry.upsert_definition(&request.connector, &adapters)?;
    Ok(LocalDaemonResponse::ConnectorUpserted { connector, path })
}

pub(crate) fn execute_register_connector_adapter_request(
    request: RegisterConnectorAdapterRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    let registry = crate::connector::ArrobaConnectorAdapterRegistry::user()?;
    let (adapter, path) = registry.install_from_file(&request.source_path)?;
    Ok(LocalDaemonResponse::ConnectorAdapterRegistered { adapter, path })
}

pub(crate) fn execute_remove_connector_adapter_request(
    request: RemoveConnectorAdapterRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    let registry = crate::connector::ArrobaConnectorAdapterRegistry::user()?;
    let (adapter, path) = registry.remove(&request.name)?;
    Ok(LocalDaemonResponse::ConnectorAdapterRemoved { adapter, path })
}

pub(crate) fn execute_get_connector_adapter_request(
    request: GetConnectorAdapterRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    let registry = crate::connector::ArrobaConnectorAdapterRegistry::user()?;
    let Some(adapter) = registry.get(&request.name)? else {
        return Err(DaemonError::LocalTransport {
            operation: "connector.adapter.get",
            message: format!("connector adapter `{}` was not found", request.name),
        });
    };
    Ok(LocalDaemonResponse::ConnectorAdapter { adapter })
}

pub(crate) fn execute_list_connector_adapters_request(
    _request: ListConnectorAdaptersRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    let registry = crate::connector::ArrobaConnectorAdapterRegistry::user()?;
    Ok(LocalDaemonResponse::ConnectorAdaptersListed {
        adapters: registry.list()?,
    })
}

pub(crate) fn execute_remove_connector_request(
    request: RemoveConnectorRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    let registry = crate::connector::ArrobaConnectorRegistry::user()?;
    let (connector, path) = registry.remove(&request.name)?;
    Ok(LocalDaemonResponse::ConnectorRemoved { connector, path })
}

pub(crate) fn execute_get_connector_request(
    request: GetConnectorRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    let registry = crate::connector::ArrobaConnectorRegistry::user()?;
    let Some(connector) = registry.get(&request.name)? else {
        return Err(DaemonError::LocalTransport {
            operation: "connector.get",
            message: format!("connector `{}` was not found", request.name),
        });
    };
    Ok(LocalDaemonResponse::Connector { connector })
}

pub(crate) fn execute_list_connectors_request(
    _request: ListConnectorsRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    let registry = crate::connector::ArrobaConnectorRegistry::user()?;
    Ok(LocalDaemonResponse::ConnectorsListed {
        connectors: registry.list()?,
    })
}

pub(crate) fn execute_test_connector_request(
    request: TestConnectorRequest,
    vault_config: crate::config::UserCredentialVaultConfig,
) -> Result<LocalDaemonResponse, DaemonError> {
    let registry = crate::connector::ArrobaConnectorRegistry::user()?;
    let adapters = crate::connector::ArrobaConnectorAdapterRegistry::user()?;
    let max_safety = crate::connector::ConnectorSafety::parse(request.allow.as_deref())?;
    let execution = registry.execute_once(
        &adapters,
        &request.name,
        &request.operation,
        request.credential.as_deref(),
        max_safety,
        request.input,
        vault_config,
    )?;
    Ok(LocalDaemonResponse::ConnectorTested { execution })
}

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

pub(crate) fn execute_register_environment_request(
    request: RegisterEnvironmentRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    let registry = crate::script::ArrobaEnvironmentRegistry::new(environment_registry_roots(
        request.workspace_id.as_deref(),
    )?);
    let path = registry.install(&request.config)?;
    Ok(LocalDaemonResponse::EnvironmentRegistered {
        environment: request.config,
        path,
    })
}

pub(crate) fn execute_remove_environment_request(
    request: RemoveEnvironmentRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    let registry = crate::script::ArrobaEnvironmentRegistry::new(environment_registry_roots(
        request.workspace_id.as_deref(),
    )?);
    let path = registry.uninstall(&request.name)?;
    Ok(LocalDaemonResponse::EnvironmentRemoved {
        name: request.name,
        path,
    })
}

pub(crate) fn execute_get_environment_request(
    request: GetEnvironmentRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    let registry = crate::script::ArrobaEnvironmentRegistry::new(environment_registry_roots(
        request.workspace_id.as_deref(),
    )?);
    let Some(environment) = registry.get(&request.name)? else {
        return Err(DaemonError::LocalTransport {
            operation: "env.get",
            message: format!("environment `{}` was not found", request.name),
        });
    };
    Ok(LocalDaemonResponse::Environment { environment })
}

pub(crate) fn execute_list_environments_request(
    request: ListEnvironmentsRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    let registry = crate::script::ArrobaEnvironmentRegistry::new(environment_registry_roots(
        request.workspace_id.as_deref(),
    )?);
    Ok(LocalDaemonResponse::EnvironmentsListed {
        environments: registry.list()?,
    })
}

pub(crate) fn execute_validate_script_request(
    request: ValidateScriptRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    let (script_registry, env, source_path) = script_validation_context(
        request.workspace_id.as_deref(),
        &request.environment,
        request.source_path,
    )?;
    let script = script_registry.validate_script(&source_path, request.name.as_deref(), &env)?;
    Ok(LocalDaemonResponse::ScriptValidated { script })
}

pub(crate) fn execute_register_script_request(
    request: RegisterScriptRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    let (script_registry, env, source_path) = script_validation_context(
        request.workspace_id.as_deref(),
        &request.environment,
        request.source_path,
    )?;
    let (script, path) = script_registry.install(&source_path, request.name.as_deref(), &env)?;
    Ok(LocalDaemonResponse::ScriptRegistered { script, path })
}

pub(crate) fn execute_remove_script_request(
    request: RemoveScriptRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    let registry = crate::script::ArrobaScriptRegistry::new(script_registry_roots(
        request.workspace_id.as_deref(),
    )?);
    let (script, path) = registry.uninstall(&request.name)?;
    Ok(LocalDaemonResponse::ScriptRemoved { script, path })
}

pub(crate) fn execute_get_script_request(
    request: GetScriptRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    let registry = crate::script::ArrobaScriptRegistry::new(script_registry_roots(
        request.workspace_id.as_deref(),
    )?);
    let Some(script) = registry.get(&request.name)? else {
        return Err(DaemonError::LocalTransport {
            operation: "script.get",
            message: format!("script `{}` was not found", request.name),
        });
    };
    Ok(LocalDaemonResponse::Script { script })
}

pub(crate) fn execute_list_scripts_request(
    request: ListScriptsRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    let registry = crate::script::ArrobaScriptRegistry::new(script_registry_roots(
        request.workspace_id.as_deref(),
    )?);
    Ok(LocalDaemonResponse::ScriptsListed {
        scripts: registry.list()?,
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

pub(crate) fn execute_upsert_skill_request(
    request: UpsertSkillRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    let registry = crate::skill::ArrobaSkillRegistry::new(skill_registry_roots(
        request.workspace_id.as_deref(),
    )?);
    let (skill, path) = match request.source {
        crate::local::SkillInstallSource::Content { skill_md } => {
            registry.upsert_from_content(&skill_md)?
        }
        crate::local::SkillInstallSource::Url { url } => registry.upsert_from_url(&url)?,
    };
    Ok(LocalDaemonResponse::SkillUpserted { skill, path })
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
            operation: "agent.extension.grant",
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
            operation: "agent.extension.grant",
            message: format!("skill `{name}` is not installed"),
        });
    }
    Ok(())
}

pub(crate) fn ensure_script_exists(
    workspace_id: Option<&str>,
    name: &str,
) -> Result<(), DaemonError> {
    let registry = crate::script::ArrobaScriptRegistry::new(script_registry_roots(workspace_id)?);
    if registry.get(name)?.is_none() {
        return Err(DaemonError::LocalTransport {
            operation: "agent.extension.grant",
            message: format!("script `{name}` is not registered"),
        });
    }
    Ok(())
}

pub(crate) fn ensure_connector_exists(name: &str) -> Result<(), DaemonError> {
    let registry = crate::connector::ArrobaConnectorRegistry::user()?;
    if registry.get(name)?.is_none() {
        return Err(DaemonError::LocalTransport {
            operation: "agent.extension.grant",
            message: format!("connector `{name}` is not registered"),
        });
    }
    Ok(())
}

pub(crate) fn ensure_credential_exists(name: &str) -> Result<(), DaemonError> {
    let registry = crate::credential::ArrobaCredentialRegistry::user()?;
    if registry.get(name)?.is_none() {
        return Err(DaemonError::LocalTransport {
            operation: "agent.extension.grant",
            message: format!("credential `{name}` is not registered"),
        });
    }
    Ok(())
}

pub(crate) fn ensure_environment_exists(
    workspace_id: Option<&str>,
    name: &str,
) -> Result<(), DaemonError> {
    let registry =
        crate::script::ArrobaEnvironmentRegistry::new(environment_registry_roots(workspace_id)?);
    if registry.get(name)?.is_none() {
        return Err(DaemonError::LocalTransport {
            operation: "agent.extension.grant",
            message: format!("environment `{name}` is not registered"),
        });
    }
    Ok(())
}

fn mcp_registry_roots(workspace_id: Option<&str>) -> Result<Vec<PathBuf>, DaemonError> {
    let workspace = registry_workspace_root(workspace_id)?;
    let mut roots = Vec::new();
    #[cfg(not(test))]
    let _ = workspace;
    #[cfg(test)]
    if workspace_id.is_some_and(|value| !value.trim().is_empty()) {
        roots.push(crate::mcp::ArrobaMcpRegistry::project_root(&workspace));
    }
    roots.extend(user_only_root(
        crate::mcp::ArrobaMcpRegistry::user_root(),
        "MCP registry root",
        "HOME must be set to resolve ~/.arroba/mcps",
    )?);
    Ok(roots)
}

pub(crate) fn script_registry_roots(
    workspace_id: Option<&str>,
) -> Result<Vec<PathBuf>, DaemonError> {
    let workspace = registry_workspace_root(workspace_id)?;
    let mut roots = Vec::new();
    #[cfg(not(test))]
    let _ = workspace;
    #[cfg(test)]
    if workspace_id.is_some_and(|value| !value.trim().is_empty()) {
        roots.push(crate::script::ArrobaScriptRegistry::project_root(
            &workspace,
        ));
    }
    roots.extend(user_only_root(
        crate::script::ArrobaScriptRegistry::user_root(),
        "script registry root",
        "HOME must be set to resolve ~/.arroba/scripts",
    )?);
    Ok(roots)
}

pub(crate) fn environment_registry_roots(
    workspace_id: Option<&str>,
) -> Result<Vec<PathBuf>, DaemonError> {
    let workspace = registry_workspace_root(workspace_id)?;
    let mut roots = Vec::new();
    #[cfg(not(test))]
    let _ = workspace;
    #[cfg(test)]
    if workspace_id.is_some_and(|value| !value.trim().is_empty()) {
        roots.push(crate::script::ArrobaEnvironmentRegistry::project_root(
            &workspace,
        ));
    }
    roots.extend(user_only_root(
        crate::script::ArrobaEnvironmentRegistry::user_root(),
        "environment registry root",
        "HOME must be set to resolve ~/.arroba/envs",
    )?);
    Ok(roots)
}

fn script_validation_context(
    workspace_id: Option<&str>,
    environment: &str,
    source_path: PathBuf,
) -> Result<
    (
        crate::script::ArrobaScriptRegistry,
        crate::script::ArrobaEnvironmentConfig,
        PathBuf,
    ),
    DaemonError,
> {
    let workspace = registry_workspace_root(workspace_id)?;
    let source_path = if source_path.is_absolute() {
        source_path
    } else {
        workspace.join(source_path)
    };
    let env_registry =
        crate::script::ArrobaEnvironmentRegistry::new(environment_registry_roots(workspace_id)?);
    let env = env_registry
        .get(environment)?
        .ok_or_else(|| DaemonError::LocalTransport {
            operation: "script.validate",
            message: format!("environment `{environment}` is not registered"),
        })?;
    let script_registry =
        crate::script::ArrobaScriptRegistry::new(script_registry_roots(workspace_id)?);
    Ok((script_registry, env, source_path))
}

fn skill_registry_roots(workspace_id: Option<&str>) -> Result<Vec<PathBuf>, DaemonError> {
    let workspace = registry_workspace_root(workspace_id)?;
    let mut roots = Vec::new();
    #[cfg(not(test))]
    let _ = workspace;
    #[cfg(test)]
    if workspace_id.is_some_and(|value| !value.trim().is_empty()) {
        roots.push(crate::skill::ArrobaSkillRegistry::project_root(&workspace));
    }
    roots.extend(user_only_root(
        crate::skill::ArrobaSkillRegistry::user_root(),
        "skill registry root",
        "HOME must be set to resolve ~/.arroba/skills",
    )?);
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

fn user_only_root(
    root: Option<PathBuf>,
    field: &'static str,
    message: &'static str,
) -> Result<Vec<PathBuf>, DaemonError> {
    root.map(|root| vec![root])
        .ok_or(DaemonError::InvalidConfig { field, message })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_roots_use_global_user_roots() {
        let workspace = PathBuf::from("/tmp/arroba-capability-registry-workspace");
        let workspace_id = workspace.to_string_lossy();

        let mcp_roots = mcp_registry_roots(Some(workspace_id.as_ref())).unwrap();
        let skill_roots = skill_registry_roots(Some(workspace_id.as_ref())).unwrap();

        assert_eq!(
            mcp_roots.first(),
            Some(&crate::mcp::ArrobaMcpRegistry::project_root(&workspace))
        );
        assert_eq!(
            mcp_roots.get(1),
            crate::mcp::ArrobaMcpRegistry::user_root().as_ref()
        );
        assert_eq!(
            skill_roots.first(),
            Some(&crate::skill::ArrobaSkillRegistry::project_root(&workspace))
        );
        assert_eq!(
            skill_roots.get(1),
            crate::skill::ArrobaSkillRegistry::user_root().as_ref()
        );
    }

    #[test]
    fn registry_workspace_root_uses_current_directory_for_blank_workspace() {
        let current_dir = std::env::current_dir().unwrap();

        assert_eq!(registry_workspace_root(Some("   ")).unwrap(), current_dir);
        assert_eq!(registry_workspace_root(None).unwrap(), current_dir);
    }
}
