use std::collections::{BTreeMap, BTreeSet};

use sha2::{Digest, Sha256};

use crate::DaemonError;

#[derive(Clone)]
struct ExtensionUse {
    agent_id: String,
    node_ids: Vec<String>,
    workspace_id: Option<String>,
    has_grant_credential: bool,
}

pub(crate) fn capture_workflow_publication_requirements(
    workflow: &crate::session::WorkflowDefinition,
    agents: &[crate::agent::AgentInstance],
) -> Result<serde_json::Value, DaemonError> {
    let uses = collect_extension_uses(workflow, agents);
    let mut extensions = Vec::with_capacity(uses.len());
    let mut credentials = BTreeMap::<String, serde_json::Value>::new();
    let mut network_destinations = BTreeMap::<String, serde_json::Value>::new();

    for ((kind, name), extension_uses) in uses {
        let requirement = match kind {
            crate::extension::ExtensionKind::Mcp => mcp_requirement(&name, &extension_uses)?,
            crate::extension::ExtensionKind::Skill => skill_requirement(&name, &extension_uses)?,
            crate::extension::ExtensionKind::Script => script_requirement(&name, &extension_uses)?,
            crate::extension::ExtensionKind::Connector => {
                connector_requirement(&name, &extension_uses)?
            }
        };
        let Some(requirement) = requirement else {
            return Err(extension_error(
                &name,
                format!(
                    "is granted to a workflow agent but has no installed {} definition",
                    kind.as_str()
                ),
            ));
        };
        for credential in requirement["credential_slots"]
            .as_array()
            .into_iter()
            .flatten()
        {
            if let Some(slot_id) = credential
                .get("slot_id")
                .and_then(serde_json::Value::as_str)
            {
                credentials.insert(slot_id.to_string(), credential.clone());
            }
        }
        for destination in requirement["network_destinations"]
            .as_array()
            .into_iter()
            .flatten()
        {
            if let Some(id) = destination.get("id").and_then(serde_json::Value::as_str) {
                network_destinations.insert(id.to_string(), destination.clone());
            }
        }
        extensions.push(requirement);
    }

    Ok(serde_json::json!({
        "schema_version": 2,
        "extensions": extensions,
        "credential_slots": credentials.into_values().collect::<Vec<_>>(),
        "network_destinations": network_destinations.into_values().collect::<Vec<_>>(),
    }))
}

fn collect_extension_uses(
    workflow: &crate::session::WorkflowDefinition,
    agents: &[crate::agent::AgentInstance],
) -> BTreeMap<(crate::extension::ExtensionKind, String), Vec<ExtensionUse>> {
    let mut uses = BTreeMap::new();
    for agent in agents {
        let node_ids = workflow
            .nodes()
            .iter()
            .filter(|node| node.agent_id() == agent.id())
            .map(|node| node.id().to_string())
            .collect::<Vec<_>>();
        if node_ids.is_empty() {
            continue;
        }
        for grant in agent.extension_grants() {
            uses.entry((grant.kind.clone(), grant.name.clone()))
                .or_insert_with(Vec::new)
                .push(ExtensionUse {
                    agent_id: agent.id().to_string(),
                    node_ids: node_ids.clone(),
                    workspace_id: agent.workspace_id().map(str::to_string),
                    has_grant_credential: grant
                        .credential
                        .as_deref()
                        .is_some_and(|value| !value.trim().is_empty()),
                });
        }
    }
    uses
}

fn mcp_requirement(
    name: &str,
    uses: &[ExtensionUse],
) -> Result<Option<serde_json::Value>, DaemonError> {
    let registry = crate::mcp::CharioxMcpRegistry::new(extension_registry_roots(
        uses,
        |workspace| crate::mcp::CharioxMcpRegistry::project_root(workspace),
        crate::mcp::CharioxMcpRegistry::user_root(),
        "MCP registry root",
    )?);
    let Some(config) = registry.get(name)? else {
        return Ok(None);
    };
    let source_digest = format!("sha256:{}", config.definition_hash()?);
    let usage = usage_json(uses);
    let mut credential_slots = Vec::new();
    let mut network_destinations = Vec::new();
    let mut local_reason = None;
    let mut launch_definition = serde_json::Value::Null;
    let mut readiness = serde_json::json!({ "kind": "mcp_initialize" });

    match &config.transport {
        crate::mcp::CharioxMcpTransportConfig::Stdio { .. } => {
            local_reason = Some(
                "stdio MCP launch depends on source-machine executables or files and has no trusted portable package reference"
                    .to_string(),
            );
        }
        crate::mcp::CharioxMcpTransportConfig::StreamableHttp {
            url,
            bearer_token_env_var,
            bearer_token_credential,
            http_headers,
            credential_http_headers,
            env_http_headers,
        } => {
            let parsed = url::Url::parse(url).map_err(|error| {
                extension_error(name, format!("has an invalid streamable HTTP URL: {error}"))
            })?;
            if parsed.scheme() != "https"
                || parsed.host_str().is_none()
                || parsed.port_or_known_default() != Some(443)
                || !parsed.username().is_empty()
                || parsed.password().is_some()
                || parsed.query().is_some()
                || parsed.fragment().is_some()
            {
                local_reason = Some(
                    "streamable HTTP MCP must use a credential-free HTTPS URL on port 443 without userinfo, query, or fragment"
                        .to_string(),
                );
            } else if !http_headers.is_empty()
                || bearer_token_env_var.is_some()
                || !env_http_headers.is_empty()
            {
                local_reason = Some(
                    "streamable HTTP MCP contains literal or environment-derived authentication that cannot be transferred safely"
                        .to_string(),
                );
            } else {
                if bearer_token_credential.is_some() {
                    credential_slots.push(integration_credential_slot(
                        "mcp",
                        name,
                        "bearer",
                        "OAuth or bearer token",
                        "oauth_or_api_key",
                        uses,
                    ));
                }
                for header in credential_http_headers.keys() {
                    credential_slots.push(integration_credential_slot(
                        "mcp",
                        name,
                        &format!("header-{header}"),
                        &format!("{header} header"),
                        "api_key",
                        uses,
                    ));
                }
                if uses.iter().any(|usage| usage.has_grant_credential)
                    && credential_slots.is_empty()
                {
                    credential_slots.push(integration_credential_slot(
                        "mcp",
                        name,
                        "credential",
                        "Integration credential",
                        "oauth_or_api_key",
                        uses,
                    ));
                }
                let credential_bindings = credential_slots
                    .iter()
                    .map(|slot| {
                        serde_json::json!({
                            "slot_id": slot["slot_id"],
                            "role": slot["role"],
                        })
                    })
                    .collect::<Vec<_>>();
                launch_definition = serde_json::json!({
                    "kind": "streamable_http",
                    "url": url,
                    "enabled": config.enabled,
                    "required": config.required,
                    "startup_timeout_sec": config.startup_timeout_sec,
                    "tool_timeout_sec": config.tool_timeout_sec,
                    "enabled_tools": config.enabled_tools,
                    "disabled_tools": config.disabled_tools,
                    "tools": config.tools,
                    "credential_bindings": credential_bindings,
                });
                if !config.enabled {
                    local_reason = Some(
                        "MCP is disabled in the immutable source definition; enable it and publish a new release"
                            .to_string(),
                    );
                    readiness = serde_json::json!({ "kind": "blocked", "reason": "disabled" });
                    launch_definition = serde_json::Value::Null;
                    credential_slots.clear();
                } else {
                    let host = parsed.host_str().expect("validated URL should have a host");
                    let port = parsed.port_or_known_default().unwrap_or(443);
                    let destination_id = format!(
                        "extension:{}",
                        stable_component(&format!("mcp-{name}-{host}-{port}"))
                    );
                    network_destinations.push(serde_json::json!({
                        "id": destination_id,
                        "host": { "kind": "exact_dns", "value": host },
                        "ports": [port],
                        "protocols": ["tls"],
                        "credential_slot_ids": credential_slots.iter().map(|slot| slot["slot_id"].clone()).collect::<Vec<_>>(),
                    }));
                }
            }
        }
    }

    let portable = local_reason.is_none();
    let content_digest = if portable {
        digest_json(&launch_definition)?
    } else {
        source_digest.clone()
    };
    Ok(Some(extension_requirement(
        "mcp",
        name,
        &content_digest,
        launch_definition,
        credential_slots,
        network_destinations,
        usage,
        readiness,
        portability(portable, local_reason),
    )))
}

fn skill_requirement(
    name: &str,
    uses: &[ExtensionUse],
) -> Result<Option<serde_json::Value>, DaemonError> {
    let registry = crate::skill::CharioxSkillRegistry::new(extension_registry_roots(
        uses,
        |workspace| crate::skill::CharioxSkillRegistry::project_root(workspace),
        crate::skill::CharioxSkillRegistry::user_root(),
        "skill registry root",
    )?);
    let Some(package) = registry.package(name)? else {
        return Ok(None);
    };
    let digest = format!("sha256:{}", package.version_hash);
    let launch = serde_json::json!({
        "kind": "skill_package",
        "package": {
            "name": package.metadata.name,
            "description": package.metadata.description,
            "short_description": package.metadata.short_description,
            "version_hash": package.version_hash,
            "files": package.files,
        },
    });
    Ok(Some(extension_requirement(
        "skill",
        name,
        &digest,
        launch,
        grant_credential_slots("skill", name, uses),
        Vec::new(),
        usage_json(uses),
        serde_json::json!({ "kind": "skill_materialized" }),
        portability(true, None),
    )))
}

fn script_requirement(
    name: &str,
    uses: &[ExtensionUse],
) -> Result<Option<serde_json::Value>, DaemonError> {
    let registry = crate::script::CharioxScriptRegistry::new(extension_registry_roots(
        uses,
        |workspace| crate::script::CharioxScriptRegistry::project_root(workspace),
        crate::script::CharioxScriptRegistry::user_root(),
        "script registry root",
    )?);
    let Some(script) = registry.get(name)? else {
        return Ok(None);
    };
    let digest = format!("sha256:{}", script.definition_hash);
    Ok(Some(extension_requirement(
        "script",
        name,
        &digest,
        serde_json::Value::Null,
        grant_credential_slots("script", name, uses),
        Vec::new(),
        usage_json(uses),
        serde_json::json!({ "kind": "source_runtime" }),
        portability(
            false,
            Some(
                "script depends on a source-machine runtime environment and has no trusted portable package reference"
                    .to_string(),
            ),
        ),
    )))
}

fn connector_requirement(
    name: &str,
    uses: &[ExtensionUse],
) -> Result<Option<serde_json::Value>, DaemonError> {
    let registry = crate::connector::CharioxConnectorRegistry::user()?;
    let Some(connector) = registry.get(name)? else {
        return Ok(None);
    };
    let digest = format!("sha256:{}", connector.definition_hash()?);
    let needs_credential = connector
        .credential
        .as_ref()
        .is_some_and(|policy| policy.required)
        || uses.iter().any(|usage| usage.has_grant_credential);
    let credential_slots = if needs_credential {
        vec![integration_credential_slot(
            "connector",
            name,
            "credential",
            "Connector credential",
            "api_key_or_service_account",
            uses,
        )]
    } else {
        Vec::new()
    };
    Ok(Some(extension_requirement(
        "connector",
        name,
        &digest,
        serde_json::Value::Null,
        credential_slots,
        Vec::new(),
        usage_json(uses),
        serde_json::json!({ "kind": "connector_adapter" }),
        portability(
            false,
            Some(
                "connector depends on a source-machine adapter package and has no trusted portable package reference"
                    .to_string(),
            ),
        ),
    )))
}

#[allow(clippy::too_many_arguments)]
fn extension_requirement(
    kind: &str,
    name: &str,
    content_digest: &str,
    launch_definition: serde_json::Value,
    credential_slots: Vec<serde_json::Value>,
    network_destinations: Vec<serde_json::Value>,
    uses: Vec<serde_json::Value>,
    readiness_test: serde_json::Value,
    portability: serde_json::Value,
) -> serde_json::Value {
    serde_json::json!({
        "id": format!("{kind}:{name}"),
        "kind": kind,
        "name": name,
        "version": content_digest,
        "content_digest": content_digest,
        "launch_definition": launch_definition,
        "credential_slots": credential_slots,
        "network_destinations": network_destinations,
        "uses": uses,
        "readiness_test": readiness_test,
        "portability": portability,
    })
}

fn integration_credential_slot(
    kind: &str,
    name: &str,
    role: &str,
    label: &str,
    authentication_method: &str,
    uses: &[ExtensionUse],
) -> serde_json::Value {
    let slot_id = format!(
        "integration:{}",
        stable_component(&format!("{kind}-{name}-{role}"))
    );
    let agent_ids = uses
        .iter()
        .map(|usage| usage.agent_id.clone())
        .collect::<BTreeSet<_>>();
    let node_ids = uses
        .iter()
        .flat_map(|usage| usage.node_ids.iter().cloned())
        .collect::<BTreeSet<_>>();
    serde_json::json!({
        "slot_id": slot_id,
        "kind": "integration",
        "label": format!("{name}: {label}"),
        "integration": name,
        "extension_id": format!("{kind}:{name}"),
        "role": role,
        "authentication_method": authentication_method,
        "required": true,
        "agent_ids": agent_ids,
        "node_ids": node_ids,
        "readiness_test": "integration_native",
    })
}

fn grant_credential_slots(kind: &str, name: &str, uses: &[ExtensionUse]) -> Vec<serde_json::Value> {
    uses.iter()
        .any(|usage| usage.has_grant_credential)
        .then(|| {
            integration_credential_slot(
                kind,
                name,
                "credential",
                "Integration credential",
                "api_key_or_service_account",
                uses,
            )
        })
        .into_iter()
        .collect()
}

fn usage_json(uses: &[ExtensionUse]) -> Vec<serde_json::Value> {
    uses.iter()
        .map(|usage| {
            serde_json::json!({
                "agent_id": usage.agent_id,
                "node_ids": usage.node_ids,
            })
        })
        .collect()
}

fn extension_registry_roots(
    uses: &[ExtensionUse],
    project_root: impl Fn(&str) -> std::path::PathBuf,
    user_root: Option<std::path::PathBuf>,
    field: &'static str,
) -> Result<Vec<std::path::PathBuf>, DaemonError> {
    let mut roots = uses
        .iter()
        .filter_map(|usage| usage.workspace_id.as_deref())
        .filter(|workspace| !workspace.trim().is_empty())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .map(project_root)
        .collect::<Vec<_>>();
    roots.extend(required_user_root(user_root, field)?);
    Ok(roots)
}

fn portability(portable: bool, reason: Option<String>) -> serde_json::Value {
    if portable {
        serde_json::json!({ "classification": "portable" })
    } else {
        serde_json::json!({
            "classification": "local_only",
            "reason": reason.unwrap_or_else(|| "extension is not portable".to_string()),
            "recommendation": "Use connected ingress or replace this extension with a portable package.",
        })
    }
}

fn required_user_root(
    root: Option<std::path::PathBuf>,
    field: &'static str,
) -> Result<Vec<std::path::PathBuf>, DaemonError> {
    root.map(|root| vec![root])
        .ok_or(DaemonError::InvalidConfig {
            field,
            message:
                "CHARIOX_HOME or the normal user home must resolve the global extension registry",
        })
}

fn extension_error(name: &str, message: String) -> DaemonError {
    DaemonError::LocalTransport {
        operation: "export workflow publication package",
        message: format!("extension `{name}` {message}"),
    }
}

fn digest_json(value: &serde_json::Value) -> Result<String, DaemonError> {
    let encoded = serde_json::to_vec(value).map_err(|error| DaemonError::LocalTransport {
        operation: "export workflow publication package",
        message: format!("failed to hash extension launch definition: {error}"),
    })?;
    Ok(format!("sha256:{:x}", Sha256::digest(encoded)))
}

fn stable_component(value: &str) -> String {
    let normalized = value
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() {
                character.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect::<String>()
        .split('-')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("-");
    let digest = format!("{:x}", Sha256::digest(value.as_bytes()));
    let stem = if normalized.is_empty() {
        "extension".to_string()
    } else {
        normalized.chars().take(48).collect()
    };
    format!("{stem}-{}", &digest[..12])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stable_slots_do_not_expose_source_credential_names() {
        let uses = vec![ExtensionUse {
            agent_id: "agent-1".to_string(),
            node_ids: vec!["node-1".to_string()],
            workspace_id: Some("/workspace".to_string()),
            has_grant_credential: true,
        }];
        let slot =
            integration_credential_slot("mcp", "github", "bearer", "OAuth token", "oauth", &uses);
        assert_eq!(slot["agent_ids"], serde_json::json!(["agent-1"]));
        assert_eq!(slot["node_ids"], serde_json::json!(["node-1"]));
        assert!(slot["slot_id"]
            .as_str()
            .is_some_and(|value| value.starts_with("integration:mcp-github-bearer-")));
        assert!(slot.get("credential").is_none());
    }

    #[test]
    fn local_only_portability_recommends_connected_ingress() {
        let value = portability(false, Some("local command".to_string()));
        assert_eq!(value["classification"], "local_only");
        assert!(value["recommendation"]
            .as_str()
            .is_some_and(|message| message.contains("connected ingress")));
    }
}
