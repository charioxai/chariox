use crate::error::DaemonError;
use crate::extension::{
    AgentExtensionCatalog, ExtensionCatalogEntry, ExtensionCatalogSource, ExtensionKind,
    ExtensionSource,
};
use crate::runtime::state::KernelRuntimeState;
use crate::transport::relay_peer::{RelayPeerRequest, RelayPeerResponse};
use arroba_relay::protocol::ClientTarget;

pub(crate) fn local_extension_catalog_entries(
    workspace_id: Option<&str>,
    source: ExtensionSource,
    kernel_id: &str,
) -> Result<Vec<ExtensionCatalogEntry>, DaemonError> {
    let mut entries = Vec::new();

    let mcp_registry = crate::mcp::ArrobaMcpRegistry::new(
        crate::runtime::capability_registry::mcp_registry_roots(workspace_id)?,
    );
    for mcp in mcp_registry.list()? {
        let credentials = mcp_credential_ids(&mcp);
        entries.push(ExtensionCatalogEntry {
            source,
            resolved_kernel_id: kernel_id.to_string(),
            kind: ExtensionKind::Mcp,
            name: mcp.name.clone(),
            description: None,
            definition_hash: Some(mcp.definition_hash()?),
            environments: Vec::new(),
            credential_required: !credentials.is_empty(),
            credentials,
            max_safety: Vec::new(),
        });
    }

    let skill_registry = crate::skill::ArrobaSkillRegistry::new(
        crate::runtime::capability_registry::skill_registry_roots(workspace_id)?,
    );
    for skill in skill_registry.list()? {
        let definition_hash = skill_registry
            .package(&skill.name)?
            .map(|package| package.version_hash);
        entries.push(ExtensionCatalogEntry {
            source,
            resolved_kernel_id: kernel_id.to_string(),
            kind: ExtensionKind::Skill,
            name: skill.name,
            description: Some(skill.description),
            definition_hash,
            environments: Vec::new(),
            credentials: Vec::new(),
            credential_required: false,
            max_safety: Vec::new(),
        });
    }

    let environments = crate::script::ArrobaEnvironmentRegistry::new(
        crate::runtime::capability_registry::environment_registry_roots(workspace_id)?,
    )
    .list()?
    .into_iter()
    .map(|environment| environment.name)
    .collect::<Vec<_>>();
    let script_registry = crate::script::ArrobaScriptRegistry::new(
        crate::runtime::capability_registry::script_registry_roots(workspace_id)?,
    );
    for script in script_registry.list()? {
        entries.push(ExtensionCatalogEntry {
            source,
            resolved_kernel_id: kernel_id.to_string(),
            kind: ExtensionKind::Script,
            name: script.name,
            description: Some(script.description),
            definition_hash: Some(script.definition_hash),
            environments: environments.clone(),
            credentials: Vec::new(),
            credential_required: false,
            max_safety: Vec::new(),
        });
    }

    let credentials = crate::credential::ArrobaCredentialRegistry::user()?.list()?;
    for connector in crate::connector::ArrobaConnectorRegistry::user()?.list()? {
        let definition_hash = connector.definition_hash()?;
        let connector_credentials = connector_credential_ids(&connector, &credentials);
        let credential_required = connector_credential_required(&connector);
        entries.push(ExtensionCatalogEntry {
            source,
            resolved_kernel_id: kernel_id.to_string(),
            kind: ExtensionKind::Connector,
            name: connector.name,
            description: Some(connector.description),
            definition_hash: Some(definition_hash),
            environments: Vec::new(),
            credentials: connector_credentials,
            credential_required,
            max_safety: vec![
                "read".to_string(),
                "write".to_string(),
                "destructive".to_string(),
            ],
        });
    }

    entries.sort_by(|left, right| {
        (&left.source, &left.kind, &left.name).cmp(&(&right.source, &right.kind, &right.name))
    });
    Ok(entries)
}

pub(crate) async fn list_agent_extension_catalog(
    runtime_state: &KernelRuntimeState,
    agent_ref: &str,
    source: ExtensionCatalogSource,
    caller_user_id: &str,
) -> Result<AgentExtensionCatalog, DaemonError> {
    let agent = runtime_state
        .list_agents()
        .into_iter()
        .find(|agent| agent.id() == agent_ref || agent.agent_ref() == agent_ref)
        .ok_or_else(|| DaemonError::AgentNotFound {
            agent_id: agent_ref.to_string(),
        })?;
    if agent.owner_user_id() != caller_user_id {
        return Err(DaemonError::OwnershipAccessDenied {
            user_id: caller_user_id.to_string(),
            owner_user_id: agent.owner_user_id().to_string(),
            resource: format!("extension catalog for agent `{}`", agent.id()),
            operation: "list agent extension catalog",
        });
    }
    let (session, _) = runtime_state.session_agent_snapshot(agent.session_id(), agent.id())?;
    let config = runtime_state.config_snapshot().await;
    let mut entries = if source.includes(ExtensionSource::Home) {
        local_extension_catalog_entries(
            Some(session.workspace_id()),
            ExtensionSource::Home,
            &config.daemon_id,
        )?
    } else {
        Vec::new()
    };

    let mut worker_available = false;
    let mut worker_error = None;
    let worker_kernel_id = agent
        .remote_execution()
        .map(|remote| remote.worker_kernel_id.clone());
    if source.includes(ExtensionSource::Worker) {
        if let Some(remote) = agent.remote_execution() {
            let mut relay_config = config.clone();
            if let (Some(relay_url), Some(relay_token)) =
                (remote.relay_url.clone(), remote.relay_token.clone())
            {
                relay_config.apply_remote_relay_override(relay_url, relay_token);
            }
            let response =
                crate::transport::relay_client::send_peer_request_via_temporary_connection(
                    &relay_config,
                    ClientTarget {
                        daemon_id: Some(remote.worker_kernel_id.clone()),
                        daemon_alias: None,
                    },
                    RelayPeerRequest::ListLeasedAgentExtensionCatalog {
                        leased_agent_id: remote.leased_agent_id.clone(),
                    },
                )
                .await;
            let binding_is_current = runtime_state
                .list_agents()
                .into_iter()
                .find(|current| current.id() == agent.id())
                .and_then(|current| current.remote_execution().cloned())
                .is_some_and(|current| {
                    current.worker_kernel_id == remote.worker_kernel_id
                        && current.worker_machine_id == remote.worker_machine_id
                        && current.execution_lease_id == remote.execution_lease_id
                        && current.leased_agent_id == remote.leased_agent_id
                });
            if !binding_is_current {
                return Err(DaemonError::LocalTransport {
                    operation: "list agent extension catalog",
                    message: "agent worker placement changed while the catalog was loading; retry"
                        .to_string(),
                });
            }
            match response {
                Ok(RelayPeerResponse::LeasedAgentExtensionCatalogListed {
                    leased_agent_id,
                    worker_kernel_id,
                    entries: worker_entries,
                }) => {
                    let provenance_matches = leased_agent_id == remote.leased_agent_id
                        && worker_kernel_id == remote.worker_kernel_id
                        && worker_entries.iter().all(|entry| {
                            entry.source == ExtensionSource::Worker
                                && entry.resolved_kernel_id == remote.worker_kernel_id
                        });
                    if provenance_matches {
                        entries.extend(worker_entries);
                        worker_available = true;
                    } else {
                        worker_error = Some(
                            "worker extension catalog response provenance did not match the active lease"
                                .to_string(),
                        );
                    }
                }
                Ok(other) => {
                    worker_error = Some(format!("unexpected worker catalog response: {other:?}"));
                }
                Err(error) => worker_error = Some(error.to_string()),
            }
        } else {
            worker_error = Some("agent is not assigned to a worker kernel".to_string());
        }
    }
    entries.sort_by(|left, right| {
        (&left.source, &left.kind, &left.name).cmp(&(&right.source, &right.kind, &right.name))
    });

    Ok(AgentExtensionCatalog {
        agent_id: agent.id().to_string(),
        home_kernel_id: config.daemon_id,
        worker_kernel_id,
        worker_available,
        worker_error,
        entries,
    })
}

fn mcp_credential_ids(config: &crate::mcp::ArrobaMcpServerConfig) -> Vec<String> {
    let mut ids = match &config.transport {
        crate::mcp::ArrobaMcpTransportConfig::Stdio { credential_env, .. } => credential_env
            .values()
            .map(|binding| binding.credential.clone())
            .collect::<Vec<_>>(),
        crate::mcp::ArrobaMcpTransportConfig::StreamableHttp {
            bearer_token_credential,
            credential_http_headers,
            ..
        } => bearer_token_credential
            .iter()
            .cloned()
            .chain(
                credential_http_headers
                    .values()
                    .map(|binding| binding.credential.clone()),
            )
            .collect::<Vec<_>>(),
    };
    ids.sort();
    ids.dedup();
    ids
}

fn connector_credential_ids(
    connector: &crate::connector::ArrobaConnectorDefinition,
    available: &[crate::config::UserCredentialConfig],
) -> Vec<String> {
    connector
        .credential
        .as_ref()
        .is_some_and(|policy| policy.required)
        .then(|| {
            available
                .iter()
                .filter(|credential| {
                    credential.allowed_uses.is_empty()
                        || credential
                            .allowed_uses
                            .contains(&crate::config::UserCredentialUse::Connector)
                })
                .map(|credential| credential.id.clone())
                .collect()
        })
        .unwrap_or_default()
}

fn connector_credential_required(connector: &crate::connector::ArrobaConnectorDefinition) -> bool {
    connector
        .credential
        .as_ref()
        .is_some_and(|policy| policy.required)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn credential(
        id: &str,
        allowed_uses: Vec<crate::config::UserCredentialUse>,
    ) -> crate::config::UserCredentialConfig {
        crate::config::UserCredentialConfig {
            id: id.to_string(),
            description: None,
            source: crate::config::UserCredentialSourceConfig::Env {
                name: format!("{}_TOKEN", id.to_uppercase()),
            },
            allowed_hosts: Vec::new(),
            allowed_uses,
            injection: crate::config::UserCredentialInjectionConfig::Basic {
                username: String::new(),
            },
            metadata: None,
        }
    }

    #[test]
    fn credential_free_connector_catalog_entry_does_not_offer_credentials() {
        let connector = crate::connector::ArrobaConnectorDefinition {
            kind: "connector".to_string(),
            name: "status".to_string(),
            description: "Status".to_string(),
            adapter: "http".to_string(),
            credential: None,
            timeout_ms: 1_000,
            max_response_bytes: 1_024,
            operations: Vec::new(),
        };

        assert!(
            connector_credential_ids(&connector, &[credential("secret-1", Vec::new())]).is_empty()
        );
        assert!(!connector_credential_required(&connector));
    }

    #[test]
    fn required_connector_without_worker_credentials_remains_marked_required() {
        let connector = crate::connector::ArrobaConnectorDefinition {
            kind: "connector".to_string(),
            name: "protected-status".to_string(),
            description: "Protected status".to_string(),
            adapter: "http".to_string(),
            credential: Some(crate::connector::ConnectorCredentialPolicy { required: true }),
            timeout_ms: 1_000,
            max_response_bytes: 1_024,
            operations: Vec::new(),
        };

        assert!(connector_credential_ids(&connector, &[]).is_empty());
        assert!(connector_credential_required(&connector));
    }

    #[test]
    fn connector_catalog_offers_only_connector_compatible_credentials() {
        let connector = crate::connector::ArrobaConnectorDefinition {
            kind: "connector".to_string(),
            name: "protected-status".to_string(),
            description: "Protected status".to_string(),
            adapter: "http".to_string(),
            credential: Some(crate::connector::ConnectorCredentialPolicy { required: true }),
            timeout_ms: 1_000,
            max_response_bytes: 1_024,
            operations: Vec::new(),
        };
        let credentials = vec![
            credential("unrestricted", Vec::new()),
            credential(
                "connector-only",
                vec![crate::config::UserCredentialUse::Connector],
            ),
            credential("http-only", vec![crate::config::UserCredentialUse::Http]),
        ];

        assert_eq!(
            connector_credential_ids(&connector, &credentials),
            vec!["unrestricted", "connector-only"]
        );
    }
}
