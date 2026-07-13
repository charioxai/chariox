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
    let package_digest = super::workflow_publication_package_digest(package_files);
    let provider_requirements = provider_requirements(snapshot);
    let provider_slots = provider_requirements
        .iter()
        .filter_map(provider_credential_slot)
        .collect::<Vec<_>>();
    let mut credential_slots = provider_slots;
    credential_slots.extend(integration_credential_slots(snapshot));
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
        },
        "compatibility": {
            "package_version": 3,
            "minimum_kernel_version": env!("CARGO_PKG_VERSION"),
            "minimum_local_daemon_protocol_version": crate::local::LOCAL_DAEMON_PROTOCOL_VERSION,
        },
        "routes": [route],
        "provider_requirements": provider_requirements,
        "credential_slots": credential_slots,
        "configuration": [],
        "capabilities": capability_ceiling(agent_app, requirements),
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

fn deployment_route(
    publication: &crate::session::WorkflowPublicationDefinition,
    publication_value: &serde_json::Value,
    agent_app: Option<&serde_json::Value>,
) -> serde_json::Value {
    let transport = super::hook_transport(publication_value);
    let path = super::string_field(publication_value, "route")
        .unwrap_or_else(|| super::default_publication_route(publication_value));
    let methods = publication_value
        .get("methods")
        .filter(|value| value.as_array().is_some_and(|values| !values.is_empty()))
        .cloned()
        .unwrap_or_else(|| super::default_publication_methods(publication_value));
    let app_route = agent_app
        .and_then(|value| value.get("routes"))
        .and_then(serde_json::Value::as_array)
        .and_then(|routes| {
            routes.iter().find(|candidate| {
                candidate.get("hook_id").and_then(serde_json::Value::as_str)
                    == Some(&format!("{}-hook", publication.id()))
                    || candidate.get("path").and_then(serde_json::Value::as_str) == Some(path)
            })
        });
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
        "id": format!("{}-hook", publication.id()),
        "transport": transport,
        "methods": methods,
        "input_schema": publication_value.get("input_schema").cloned().unwrap_or(serde_json::Value::Null),
        "required_roles": [required_role],
        "session": {
            "scope": scope,
            "per_caller_ordering": per_caller_ordering,
        },
        "streaming": matches!(transport.as_str(), Some("human_http" | "api_sse_json" | "websocket_json")),
        "timeout_ms": timeout_ms.unwrap_or(serde_json::Value::Null),
        "idempotency": "unspecified",
    });
    if transport.as_str() != Some("schedule_only") {
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
        let entry = by_provider.entry(agent.provider().to_string()).or_default();
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

fn provider_credential_slot(requirement: &serde_json::Value) -> Option<serde_json::Value> {
    let provider = requirement.get("provider")?.as_str()?;
    Some(serde_json::json!({
        "slot_id": requirement.get("slot_id")?,
        "kind": "provider",
        "label": format!("{provider} account"),
        "provider": provider,
        "required": true,
        "scope": "environment",
        "uses": requirement.get("node_ids").cloned().unwrap_or_else(|| serde_json::json!([])),
        "test_method": "provider_native",
    }))
}

fn integration_credential_slots(
    snapshot: &crate::local::WorkflowPublicationSnapshot,
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
            serde_json::json!({
                "slot_id": format!("integration:{}", stable_slot_component(&credential)),
                "kind": "integration",
                "label": credential,
                "integration": credential,
                "required": true,
                "scope": "environment",
                "uses": uses,
                "test_method": "runtime_requirement",
            })
        })
        .collect()
}

fn capability_ceiling(
    agent_app: Option<&serde_json::Value>,
    requirements: &serde_json::Value,
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
        "network": { "egress_policy": "deployment_tightens" },
    })
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
