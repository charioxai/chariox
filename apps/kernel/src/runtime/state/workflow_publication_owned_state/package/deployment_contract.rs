use base64::Engine;
use sha2::{Digest, Sha256};

use super::*;

pub(super) fn workflow_publication_deployment_contract_json(
    publication: &crate::session::WorkflowPublicationDefinition,
    publication_value: &serde_json::Value,
    snapshot: &crate::local::WorkflowPublicationSnapshot,
    agent_app: Option<&serde_json::Value>,
    requirements: &serde_json::Value,
    package_files: &[crate::local::WorkflowPublicationPackageFile],
) -> Result<serde_json::Value, DaemonError> {
    let package_digest = super::workflow_publication_package_digest(package_files)?;
    let provider_requirements = provider_requirements(snapshot);
    let network_destinations = network_destinations(agent_app)?;
    let provider_slots = provider_requirements
        .iter()
        .filter_map(|requirement| provider_credential_slot(requirement, &network_destinations))
        .collect::<Vec<_>>();
    let mut credential_slots = provider_slots;
    credential_slots.extend(integration_credential_slots(
        snapshot,
        &network_destinations,
    ));
    validate_network_destination_slots(&network_destinations, &credential_slots)?;
    let assets = package_asset_manifest(package_files)?;
    let route = deployment_route(publication, publication_value, agent_app);
    let enabled_agent_app = agent_app
        .and_then(|value| value.get("enabled"))
        .and_then(serde_json::Value::as_bool)
        == Some(true);

    Ok(serde_json::json!({
        "schema_version": 1,
        "package_id": package_digest,
        "artifact": {
            "content_digest": package_digest,
            "digest_algorithm": "sha256",
            "digest_scope": "package_files_excluding_deployment_contract",
        },
        "source": {
            "publication_id": publication.id(),
            "session_id": publication.session_id(),
            "workflow_id": publication.workflow_id(),
            "endpoint_id": publication.endpoint_id(),
            "creator_user_id": publication.created_by_user_id(),
            "captured_at_ms": snapshot.captured_at_ms,
            "workflow_revision": publication.source_workflow_revision(),
            "snapshot_digest": publication.source_snapshot_digest(),
        },
        "compatibility": {
            "package_version": 3,
            "minimum_kernel_version": env!("CARGO_PKG_VERSION"),
            "minimum_local_daemon_protocol_version": crate::local::LOCAL_DAEMON_PROTOCOL_VERSION,
        },
        "routes": [route],
        "provider_requirements": provider_requirements,
        "credential_slots": credential_slots,
        "configuration": deployment_configuration(snapshot),
        "capabilities": capability_ceiling(agent_app, requirements, &provider_requirements, &network_destinations),
        "resources": resource_hints(snapshot, agent_app),
        "presentation": {
            "kind": if enabled_agent_app { "agent_app" } else { "workflow_endpoint" },
            "display_name": publication.alias().unwrap_or(publication.id()),
            "entry_path": presentation_entry_path(publication_value, agent_app),
            "assets": assets,
        },
        "signatures": [],
    }))
}

fn deployment_configuration(
    snapshot: &crate::local::WorkflowPublicationSnapshot,
) -> Vec<serde_json::Value> {
    // The deployment operator owns the runtime provider credentials and may
    // recover an unavailable captured provider by rebinding an agent to any
    // provider already declared by this immutable package. Consumers still
    // reject providers outside the packaged requirements.
    let allowed_providers = snapshot
        .agents
        .iter()
        .map(|agent| agent.provider().to_string())
        .collect::<std::collections::BTreeSet<_>>();
    snapshot
        .agents
        .iter()
        .map(|agent| {
            let node_ids = snapshot
                .workflow
                .nodes()
                .iter()
                .filter(|node| node.agent_id() == agent.id())
                .map(|node| node.id().to_string())
                .collect::<Vec<_>>();
            serde_json::json!({
                "key": format!("provider_profile:{}", agent.id()),
                "kind": "provider_profile",
                "label": format!("Provider profile for {}", agent.id()),
                "required": true,
                "secret": false,
                "agent_id": agent.id(),
                "node_ids": node_ids,
                "allowed_providers": allowed_providers,
                "captured": {
                    "provider": agent.provider(),
                    "model": agent.model(),
                    "effort": agent.effort(),
                },
            })
        })
        .collect()
}

fn deployment_route(
    publication: &crate::session::WorkflowPublicationDefinition,
    publication_value: &serde_json::Value,
    agent_app: Option<&serde_json::Value>,
) -> serde_json::Value {
    let transport = super::hook_transport(publication_value);
    let publication_hook_id = format!("{}-hook", publication.id());
    let publication_path = super::string_field(publication_value, "route")
        .unwrap_or_else(|| super::default_publication_route(publication_value));
    let app_route = agent_app
        .filter(|value| value.get("enabled").and_then(serde_json::Value::as_bool) == Some(true))
        .and_then(|value| value.get("routes"))
        .and_then(serde_json::Value::as_array)
        .and_then(|routes| {
            routes
                .iter()
                .find(|candidate| {
                    candidate.get("hook_id").and_then(serde_json::Value::as_str)
                        == Some(publication_hook_id.as_str())
                        || candidate.get("path").and_then(serde_json::Value::as_str)
                            == Some(publication_path)
                })
                .or_else(|| routes.first())
        });
    let path = app_route
        .and_then(|value| value.get("path"))
        .and_then(serde_json::Value::as_str)
        .unwrap_or(publication_path);
    let methods = if !super::publication_uses_http_ingress(publication_value) {
        serde_json::json!([])
    } else if app_route.is_some() {
        serde_json::json!(["GET"])
    } else {
        publication_value
            .get("methods")
            .filter(|value| value.as_array().is_some_and(|values| !values.is_empty()))
            .cloned()
            .unwrap_or_else(|| super::default_publication_methods(publication_value))
    };
    let required_role = app_route
        .and_then(|value| value.get("required_role"))
        .and_then(serde_json::Value::as_str)
        .unwrap_or("public");
    let scope = app_route
        .and_then(|value| value.pointer("/manipulation/scope"))
        .and_then(serde_json::Value::as_str)
        .unwrap_or("invocation");
    let per_caller_ordering = agent_app
        .and_then(|value| value.pointer("/replicas/per_caller_ordering"))
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);
    let timeout_ms = publication_value
        .get("sync_timeout_ms")
        .cloned()
        .or_else(|| {
            agent_app
                .and_then(|value| value.pointer("/replicas/timeout_ms"))
                .cloned()
        });

    let mut route = serde_json::json!({
        "id": publication_hook_id,
        "transport": transport,
        "methods": methods,
        "input_schema": publication_value.get("input_schema").cloned().unwrap_or(serde_json::Value::Null),
        "required_roles": [required_role],
        "session": {
            "scope": scope,
            "per_caller_ordering": per_caller_ordering,
        },
        "streaming": transport.as_str() == Some("human_http"),
        "timeout_ms": timeout_ms.unwrap_or(serde_json::Value::Null),
        "idempotency": "unspecified",
    });
    if super::publication_uses_http_ingress(publication_value) {
        route["path"] = serde_json::Value::String(path.to_string());
    }
    route
}

fn provider_requirements(
    snapshot: &crate::local::WorkflowPublicationSnapshot,
) -> Vec<serde_json::Value> {
    let mut by_provider = std::collections::BTreeMap::<
        String,
        (std::collections::BTreeSet<String>, Vec<String>, Vec<String>),
    >::new();
    for agent in &snapshot.agents {
        let entry = by_provider
            .entry(deployment_provider_family(agent.provider()))
            .or_default();
        if let Some(model) = agent.model() {
            entry.0.insert(model.to_string());
        }
        entry.1.push(agent.id().to_string());
        entry.2.extend(
            snapshot
                .workflow
                .nodes()
                .iter()
                .filter(|node| node.agent_id() == agent.id())
                .map(|node| node.id().to_string()),
        );
    }
    by_provider
        .into_iter()
        .map(|(provider, (models, agent_ids, node_ids))| {
            serde_json::json!({
                "slot_id": format!("provider:{}", stable_slot_component(&provider)),
                "provider": provider,
                "models": models,
                "agent_ids": agent_ids,
                "node_ids": node_ids,
                "allowed_substitutions": false,
                "readiness_test": "provider_native",
            })
        })
        .collect()
}

fn deployment_provider_family(provider: &str) -> String {
    crate::provider::canonical_provider_family(provider)
        .map(str::to_string)
        .unwrap_or_else(|| crate::provider::adapter_key_for_provider(provider).to_string())
}

fn provider_credential_slot(
    requirement: &serde_json::Value,
    network_destinations: &[serde_json::Value],
) -> Option<serde_json::Value> {
    let provider = requirement.get("provider")?.as_str()?;
    let slot_id = requirement.get("slot_id")?;
    Some(serde_json::json!({
        "slot_id": slot_id,
        "kind": "provider",
        "label": format!("{provider} account"),
        "provider": provider,
        "required": true,
        "scope": "environment",
        "uses": requirement.get("node_ids").cloned().unwrap_or_else(|| serde_json::json!([])),
        "allowed_destination_ids": destination_ids_for_slot(network_destinations, slot_id.as_str()?),
        "test_method": "provider_native",
    }))
}

fn integration_credential_slots(
    snapshot: &crate::local::WorkflowPublicationSnapshot,
    network_destinations: &[serde_json::Value],
) -> Vec<serde_json::Value> {
    let mut uses = std::collections::BTreeMap::<String, std::collections::BTreeSet<String>>::new();
    for agent in &snapshot.agents {
        for grant in agent.extension_grants() {
            let Some(credential) = grant
                .credential
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
            else {
                continue;
            };
            uses.entry(credential.to_string())
                .or_default()
                .insert(grant.name.clone());
        }
    }
    uses.into_iter()
        .map(|(credential, uses)| {
            let slot_id = format!("integration:{}", stable_slot_component(&credential));
            serde_json::json!({
                "slot_id": slot_id,
                "kind": "integration",
                "label": credential,
                "integration": credential,
                "required": true,
                "scope": "environment",
                "uses": uses,
                "allowed_destination_ids": destination_ids_for_slot(network_destinations, &slot_id),
                "test_method": "runtime_requirement",
            })
        })
        .collect()
}

fn capability_ceiling(
    agent_app: Option<&serde_json::Value>,
    requirements: &serde_json::Value,
    provider_requirements: &[serde_json::Value],
    network_destinations: &[serde_json::Value],
) -> serde_json::Value {
    let mut actions = std::collections::BTreeSet::new();
    if let Some(configured) = agent_app
        .and_then(|value| value.get("actions"))
        .and_then(serde_json::Value::as_object)
    {
        actions.extend(configured.keys().cloned());
    }
    let manipulation = agent_app
        .and_then(|value| value.get("routes"))
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|route| {
            route
                .pointer("/manipulation/level")
                .and_then(serde_json::Value::as_str)
        })
        .collect::<std::collections::BTreeSet<_>>();
    serde_json::json!({
        "actions": actions,
        "manipulation_levels": manipulation,
        "extensions": {
            "mcps": requirements.get("mcps").cloned().unwrap_or_else(|| serde_json::json!([])),
            "skills": requirements.get("skills").cloned().unwrap_or_else(|| serde_json::json!([])),
            "scripts": requirements.get("scripts").cloned().unwrap_or_else(|| serde_json::json!([])),
            "connectors": requirements.get("connectors").cloned().unwrap_or_else(|| serde_json::json!([])),
        },
        "filesystem": { "write_policy": "ephemeral_runtime_only" },
        "network": {
            "policy_version": 1,
            "default_action": "deny",
            "destinations": network_destinations,
            "provider_access": provider_requirements.iter().filter_map(provider_access).collect::<Vec<_>>(),
        },
    })
}

fn network_destinations(
    agent_app: Option<&serde_json::Value>,
) -> Result<Vec<serde_json::Value>, DaemonError> {
    let Some(destinations) = agent_app
        .and_then(|value| value.pointer("/network/destinations"))
        .and_then(serde_json::Value::as_array)
    else {
        return Ok(Vec::new());
    };
    if destinations.len() > 256 {
        return Err(invalid_network_policy(
            "network destinations must contain at most 256 entries",
        ));
    }
    let mut ids = std::collections::BTreeSet::new();
    let mut hosts = std::collections::BTreeSet::new();
    let mut normalized = Vec::with_capacity(destinations.len());
    for destination in destinations {
        let id = destination
            .get("id")
            .and_then(serde_json::Value::as_str)
            .filter(|value| is_network_destination_id(value))
            .ok_or_else(|| invalid_network_policy("network destination id is invalid"))?;
        let host = destination
            .get("host")
            .and_then(serde_json::Value::as_str)
            .filter(|value| is_canonical_dns_name(value))
            .ok_or_else(|| {
                invalid_network_policy(
                    "network destination host must be an exact canonical DNS name",
                )
            })?;
        if !ids.insert(id.to_string()) || !hosts.insert(host.to_string()) {
            return Err(invalid_network_policy(
                "network destination ids and hosts must be unique",
            ));
        }
        let credential_slot_ids = destination
            .get("credential_slot_ids")
            .and_then(serde_json::Value::as_array)
            .map(|values| {
                values
                    .iter()
                    .filter_map(serde_json::Value::as_str)
                    .map(str::to_string)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        if credential_slot_ids
            .iter()
            .any(|slot_id| !is_credential_slot_id(slot_id))
            || credential_slot_ids
                .iter()
                .collect::<std::collections::BTreeSet<_>>()
                .len()
                != credential_slot_ids.len()
        {
            return Err(invalid_network_policy(
                "network destination credential slot ids are invalid",
            ));
        }
        normalized.push(serde_json::json!({
            "id": id,
            "host": { "kind": "exact_dns", "value": host },
            "ports": [443],
            "protocols": ["tls"],
            "credential_slot_ids": credential_slot_ids,
        }));
    }
    normalized.sort_by(|left, right| left["id"].as_str().cmp(&right["id"].as_str()));
    Ok(normalized)
}

fn provider_access(requirement: &serde_json::Value) -> Option<serde_json::Value> {
    let provider = requirement.get("provider")?.as_str()?;
    let slot_id = requirement.get("slot_id")?.as_str()?;
    let (bundle_kind, bundle_id) = match provider {
        "codex" => ("platform_managed", "codex-official-v1"),
        "claude" | "claude-code" => ("platform_managed", "claude-official-v1"),
        "opencode" => ("platform_managed", "opencode-official-v1"),
        "dev-stub" => ("development_stub", "dev-stub-v1"),
        _ => ("unsupported", "unsupported-provider-v1"),
    };
    Some(serde_json::json!({
        "slot_id": slot_id,
        "bundle_kind": bundle_kind,
        "bundle_id": bundle_id,
    }))
}

fn destination_ids_for_slot(destinations: &[serde_json::Value], slot_id: &str) -> Vec<String> {
    destinations
        .iter()
        .filter(|destination| {
            destination["credential_slot_ids"]
                .as_array()
                .is_some_and(|values| values.iter().any(|value| value.as_str() == Some(slot_id)))
        })
        .filter_map(|destination| destination["id"].as_str().map(str::to_string))
        .collect()
}

fn validate_network_destination_slots(
    destinations: &[serde_json::Value],
    credential_slots: &[serde_json::Value],
) -> Result<(), DaemonError> {
    let slot_ids = credential_slots
        .iter()
        .filter_map(|slot| slot["slot_id"].as_str())
        .collect::<std::collections::BTreeSet<_>>();
    if destinations.iter().any(|destination| {
        destination["credential_slot_ids"]
            .as_array()
            .is_some_and(|values| {
                values
                    .iter()
                    .filter_map(serde_json::Value::as_str)
                    .any(|slot_id| !slot_ids.contains(slot_id))
            })
    }) {
        return Err(invalid_network_policy(
            "network destination references an undeclared credential slot",
        ));
    }
    Ok(())
}

fn is_network_destination_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value.as_bytes()[0].is_ascii_lowercase()
        && value.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b':' | b'_' | b'-')
        })
}

fn is_credential_slot_id(value: &str) -> bool {
    let Some((kind, name)) = value.split_once(':') else {
        return false;
    };
    matches!(kind, "provider" | "integration")
        && !name.is_empty()
        && name
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
}

fn is_canonical_dns_name(value: &str) -> bool {
    value.len() <= 253
        && value.contains('.')
        && value.parse::<std::net::IpAddr>().is_err()
        && value == value.to_ascii_lowercase()
        && !value.ends_with('.')
        && value.split('.').all(|label| {
            !label.is_empty()
                && label.len() <= 63
                && label.as_bytes()[0].is_ascii_alphanumeric()
                && label.as_bytes()[label.len() - 1].is_ascii_alphanumeric()
                && label
                    .bytes()
                    .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
        })
}

fn invalid_network_policy(message: impl Into<String>) -> DaemonError {
    DaemonError::LocalTransport {
        operation: "export workflow publication package",
        message: message.into(),
    }
}

fn resource_hints(
    snapshot: &crate::local::WorkflowPublicationSnapshot,
    agent_app: Option<&serde_json::Value>,
) -> serde_json::Value {
    serde_json::json!({
        "expected_concurrency": snapshot.workflow.max_concurrent(),
        "replicas": numeric_agent_app_value(agent_app, "/replicas/count").unwrap_or(1),
        "max_queue_depth": numeric_agent_app_value(agent_app, "/replicas/max_queue_depth"),
        "timeout_ms": numeric_agent_app_value(agent_app, "/replicas/timeout_ms"),
        "storage": "ephemeral",
        "schedule_count": snapshot.schedules.len(),
        "region_hint": null,
        "data_sensitivity": "unspecified",
    })
}

fn numeric_agent_app_value(agent_app: Option<&serde_json::Value>, pointer: &str) -> Option<u64> {
    agent_app
        .and_then(|value| value.pointer(pointer))
        .and_then(serde_json::Value::as_u64)
}

fn presentation_entry_path(
    publication_value: &serde_json::Value,
    agent_app: Option<&serde_json::Value>,
) -> String {
    if let Some(index) = agent_app
        .and_then(|value| value.pointer("/assets/index"))
        .and_then(serde_json::Value::as_str)
    {
        return index.to_string();
    }
    super::string_field(publication_value, "route")
        .unwrap_or_else(|| super::default_publication_route(publication_value))
        .to_string()
}

fn package_asset_manifest(
    files: &[crate::local::WorkflowPublicationPackageFile],
) -> Result<Vec<serde_json::Value>, DaemonError> {
    files
        .iter()
        .filter(|file| file.path.starts_with("app/") || file.path.starts_with("public/"))
        .map(|file| {
            let content = base64::engine::general_purpose::STANDARD
                .decode(&file.content_base64)
                .map_err(|error| DaemonError::LocalTransport {
                    operation: "export workflow publication package",
                    message: format!("failed to hash package asset `{}`: {error}", file.path),
                })?;
            Ok(serde_json::json!({
                "path": file.path,
                "sha256": format!("sha256:{:x}", Sha256::digest(&content)),
                "byte_size": content.len(),
            }))
        })
        .collect()
}

fn stable_slot_component(value: &str) -> String {
    let normalized = value
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() {
                character.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect::<String>();
    normalized
        .split('-')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("-")
}

#[cfg(test)]
mod tests {
    use super::deployment_provider_family;

    #[test]
    fn deployment_provider_identity_collapses_runtime_aliases() {
        assert_eq!(deployment_provider_family("claude-headless"), "claude");
        assert_eq!(deployment_provider_family("claude-p"), "claude");
        assert_eq!(deployment_provider_family("default"), "opencode");
        assert_eq!(deployment_provider_family("codex"), "codex");
        assert_eq!(deployment_provider_family("dev-stub"), "dev-stub");
    }
}
