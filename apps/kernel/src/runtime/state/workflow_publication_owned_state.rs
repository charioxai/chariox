//! Workflow publication mutations.
//!
//! This module owns public endpoint publication administration. Workflow graph design and run
//! administration stay in `workflow_admin`.

use base64::Engine;
use flate2::{write::GzEncoder, Compression};
use sha2::{Digest, Sha256};

use super::*;

impl KernelRuntimeOwnedState {
    pub(super) fn workflow_create_publication(
        &self,
        request: crate::local::CreateWorkflowPublicationRequest,
        caller_user_id: &str,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        self.ensure_workflow_endpoint_owner(
            &request.session_id,
            &request.workflow_ref,
            &request.endpoint_ref,
            caller_user_id,
            "publish workflow endpoint",
        )?;
        let publication = self.session_store.write().create_workflow_publication(
            &request.session_id,
            &request.workflow_ref,
            &request.endpoint_ref,
            request.queue_ref,
            request.alias,
            request.route,
            request.methods,
            request.transport,
            request.parser,
            request.input_schema,
            request.trace_exposure,
            request.mode,
            request.sync_timeout_ms,
            request.poll_ms,
            caller_user_id.to_string(),
        )?;
        let session = self.workflow_session(&request.session_id)?;
        Ok(LocalDaemonResponse::WorkflowPublicationCreated {
            publication,
            session,
        })
    }

    pub(super) fn workflow_list_publications(
        &self,
        request: crate::local::ListWorkflowPublicationsRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        Ok(LocalDaemonResponse::WorkflowPublicationsListed {
            publications: self
                .session_store
                .read()
                .list_workflow_publications(&request.session_id)?,
        })
    }

    pub(super) fn workflow_get_publication(
        &self,
        request: crate::local::GetWorkflowPublicationRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        Ok(LocalDaemonResponse::WorkflowPublication {
            publication: self
                .session_store
                .read()
                .resolve_workflow_publication_ref(&request.session_id, &request.publication_ref)?,
        })
    }

    pub(super) fn workflow_export_publication_package(
        &self,
        request: crate::local::ExportWorkflowPublicationPackageRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let publication = self
            .session_store
            .read()
            .resolve_workflow_publication_ref(&request.session_id, &request.publication_ref)?;
        let session = self.workflow_session(&request.session_id)?;
        let package_files = workflow_publication_package_files(
            &publication,
            &session,
            request.kernel_url.as_deref(),
            request.agent_app.as_ref(),
            request.agent_app_assets_dir.as_deref(),
        )?;
        let package_version = workflow_publication_package_version(request.agent_app.as_ref());
        let package_digest = workflow_publication_package_digest(&package_files);
        let package_archive_base64 = workflow_publication_package_archive_base64(&package_files)?;
        Ok(LocalDaemonResponse::WorkflowPublicationPackageExported {
            publication,
            package_version,
            package_digest,
            package_archive_base64,
            package_files,
        })
    }

    pub(super) fn workflow_disable_publication(
        &self,
        request: crate::local::DisableWorkflowPublicationRequest,
        caller_user_id: &str,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let publication = self
            .session_store
            .read()
            .resolve_workflow_publication_ref(&request.session_id, &request.publication_ref)?;
        if publication.created_by_user_id() != caller_user_id {
            return Err(Self::deny_owner(
                caller_user_id,
                publication.created_by_user_id(),
                format!("workflow publication `{}`", request.publication_ref),
                "disable workflow publication",
            ));
        }
        let publication = self
            .session_store
            .write()
            .disable_workflow_publication(&request.session_id, &request.publication_ref)?;
        let session = self.workflow_session(&request.session_id)?;
        Ok(LocalDaemonResponse::WorkflowPublicationDisabled {
            publication,
            session,
        })
    }

    pub(super) fn workflow_register_publication_endpoint(
        &self,
        request: crate::local::RegisterWorkflowPublicationEndpointRequest,
        caller_user_id: &str,
        open_url: String,
        access: String,
        expires_at_ms: Option<u64>,
        deployment: serde_json::Value,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let publication = self
            .session_store
            .read()
            .resolve_workflow_publication_ref(&request.session_id, &request.publication_ref)?;
        if publication.created_by_user_id() != caller_user_id {
            return Err(Self::deny_owner(
                caller_user_id,
                publication.created_by_user_id(),
                format!("workflow publication `{}`", request.publication_ref),
                "register workflow publication endpoint",
            ));
        }
        let publication = self
            .session_store
            .write()
            .register_workflow_publication_endpoint(
                &request.session_id,
                &request.publication_ref,
                "running",
                open_url.clone(),
                deployment,
            )?;
        Ok(LocalDaemonResponse::WorkflowPublicationEndpointRegistered {
            publication,
            open_url,
            access,
            expires_at_ms,
        })
    }

    pub(super) fn workflow_materialize_publication(
        &self,
        request: crate::local::MaterializeWorkflowPublicationRequest,
        caller_user_id: &str,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        if request.snapshot.schema_version != 1 {
            return Err(DaemonError::LocalTransport {
                operation: "materialize workflow publication",
                message: format!(
                    "unsupported workflow snapshot schema_version {}",
                    request.snapshot.schema_version
                ),
            });
        }
        let Some(source_session) = request.snapshot.source_session.as_ref() else {
            return Err(DaemonError::LocalTransport {
                operation: "materialize workflow publication",
                message: "workflow snapshot is missing source_session".to_string(),
            });
        };
        let workflow_id = request.snapshot.workflow.id().to_string();
        if let Some(endpoint) = request.snapshot.endpoint.as_ref() {
            if !request
                .snapshot
                .workflow
                .endpoints()
                .iter()
                .any(|candidate| candidate.id() == endpoint.id())
            {
                return Err(DaemonError::LocalTransport {
                    operation: "materialize workflow publication",
                    message: format!(
                        "snapshot endpoint `{}` is not present in workflow `{workflow_id}`",
                        endpoint.id()
                    ),
                });
            }
        }
        let endpoint_id = request
            .snapshot
            .endpoint
            .as_ref()
            .map(|endpoint| endpoint.id().to_string())
            .or_else(|| {
                request
                    .snapshot
                    .workflow
                    .endpoints()
                    .first()
                    .map(|endpoint| endpoint.id().to_string())
            })
            .ok_or_else(|| DaemonError::LocalTransport {
                operation: "materialize workflow publication",
                message: "workflow snapshot is missing a publication endpoint".to_string(),
            })?;
        if let Some(queue) = request
            .snapshot
            .queues
            .iter()
            .find(|queue| queue.workflow_id() != workflow_id)
        {
            return Err(DaemonError::LocalTransport {
                operation: "materialize workflow publication",
                message: format!(
                    "snapshot queue `{}` belongs to workflow `{}` instead of `{workflow_id}`",
                    queue.id(),
                    queue.workflow_id()
                ),
            });
        }
        if let Some(schedule) = request
            .snapshot
            .schedules
            .iter()
            .find(|schedule| schedule.workflow_id() != workflow_id)
        {
            return Err(DaemonError::LocalTransport {
                operation: "materialize workflow publication",
                message: format!(
                    "snapshot schedule `{}` belongs to workflow `{}` instead of `{workflow_id}`",
                    schedule.id(),
                    schedule.workflow_id()
                ),
            });
        }
        if let Some(schedule) = request.snapshot.schedules.iter().find(|schedule| {
            request
                .snapshot
                .workflow
                .endpoint(schedule.endpoint_id())
                .is_none()
        }) {
            return Err(DaemonError::LocalTransport {
                operation: "materialize workflow publication",
                message: format!(
                    "snapshot schedule `{}` references missing endpoint `{}`",
                    schedule.id(),
                    schedule.endpoint_id()
                ),
            });
        }

        let captured_agents = request
            .snapshot
            .agents
            .into_iter()
            .map(|agent| (agent.id().to_string(), agent))
            .collect::<BTreeMap<_, _>>();
        let missing_agent_ids = request
            .snapshot
            .workflow
            .nodes()
            .iter()
            .filter_map(|node| {
                if captured_agents.contains_key(node.agent_id()) {
                    None
                } else {
                    Some(node.agent_id().to_string())
                }
            })
            .collect::<Vec<_>>();
        if !missing_agent_ids.is_empty() {
            return Err(DaemonError::LocalTransport {
                operation: "materialize workflow publication",
                message: format!(
                    "workflow snapshot is missing agents for nodes: {}",
                    missing_agent_ids.join(", ")
                ),
            });
        }

        let session = self.session_store.create_session(
            crate::session::CreateSessionRequest::new(
                source_session.workspace_id.clone(),
                source_session.worktree_id.clone(),
            )
            .with_owner_user_id(caller_user_id)
            .with_hidden(true),
        )?;
        let session_id = session.id().to_string();
        let mut agent_id_map = BTreeMap::new();
        for (captured_agent_id, agent) in captured_agents {
            let materialized = self.agent_store.materialize_publication_agent(
                agent,
                &session_id,
                Some(caller_user_id),
            );
            agent_id_map.insert(captured_agent_id, materialized.id().to_string());
        }

        let mut workflow = request.snapshot.workflow;
        let node_ids = workflow
            .nodes()
            .iter()
            .map(|node| node.id().to_string())
            .collect::<Vec<_>>();
        for node_id in node_ids {
            let Some(node) = workflow.node_mut(&node_id) else {
                continue;
            };
            let Some(materialized_agent_id) = agent_id_map.get(node.agent_id()) else {
                continue;
            };
            node.set_agent_id(materialized_agent_id.clone());
            node.set_owner_user_id(caller_user_id);
            node.set_created_by_user_id(caller_user_id);
        }
        let edge_ids = workflow
            .edges()
            .iter()
            .map(|edge| edge.id().to_string())
            .collect::<Vec<_>>();
        for edge_id in edge_ids {
            if let Some(edge) = workflow.edge_mut(&edge_id) {
                edge.set_created_by_user_id(caller_user_id);
            }
        }
        let endpoint_ids = workflow
            .endpoints()
            .iter()
            .map(|endpoint| endpoint.id().to_string())
            .collect::<Vec<_>>();
        for endpoint_id in endpoint_ids {
            if let Some(endpoint) = workflow.endpoint_mut(&endpoint_id) {
                endpoint.set_owner_user_id(caller_user_id);
            }
        }
        self.session_store.replace_publication_runtime_workflows(
            &session_id,
            vec![workflow],
            request.snapshot.queues,
            request.snapshot.schedules,
        )?;
        let publication = crate::session::WorkflowPublicationDefinition::new(
            request.publication_id.clone(),
            session_id.clone(),
            workflow_id.clone(),
            endpoint_id,
            Some("default".to_string()),
            None,
            None,
            Vec::new(),
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            caller_user_id.to_string(),
        );
        self.session_store
            .write()
            .restore_workflow_publication(&session_id, publication)?;
        let session = self.workflow_session(&session_id)?;
        Ok(LocalDaemonResponse::WorkflowPublicationMaterialized {
            publication_id: request.publication_id,
            session,
            agent_id_map,
        })
    }
}

fn workflow_publication_package_files(
    publication: &crate::session::WorkflowPublicationDefinition,
    session: &crate::session::RuntimeSession,
    kernel_url: Option<&str>,
    agent_app: Option<&serde_json::Value>,
    agent_app_assets_dir: Option<&str>,
) -> Result<Vec<crate::local::WorkflowPublicationPackageFile>, DaemonError> {
    let publication_value =
        serde_json::to_value(publication).map_err(|error| DaemonError::LocalTransport {
            operation: "export workflow publication package",
            message: format!("failed to encode publication: {error}"),
        })?;
    let workflow = session
        .workflows()
        .iter()
        .find(|candidate| candidate.id() == publication.workflow_id())
        .cloned()
        .ok_or_else(|| DaemonError::LocalTransport {
            operation: "export workflow publication package",
            message: format!(
                "workflow `{}` was not found in session `{}`",
                publication.workflow_id(),
                session.id()
            ),
        })?;
    let endpoint = workflow
        .endpoints()
        .iter()
        .find(|candidate| candidate.id() == publication.endpoint_id())
        .cloned()
        .ok_or_else(|| DaemonError::LocalTransport {
            operation: "export workflow publication package",
            message: format!(
                "endpoint `{}` was not found in workflow `{}`",
                publication.endpoint_id(),
                workflow.id()
            ),
        })?;
    let node_agent_ids = workflow
        .nodes()
        .iter()
        .map(|node| node.agent_id().to_string())
        .collect::<std::collections::BTreeSet<_>>();
    let agents = session
        .agents()
        .iter()
        .filter(|agent| node_agent_ids.contains(agent.id()))
        .cloned()
        .collect::<Vec<_>>();
    let missing_agent_ids = node_agent_ids
        .iter()
        .filter(|agent_id| !agents.iter().any(|agent| agent.id() == agent_id.as_str()))
        .cloned()
        .collect::<Vec<_>>();
    if !missing_agent_ids.is_empty() {
        return Err(DaemonError::LocalTransport {
            operation: "export workflow publication package",
            message: format!(
                "workflow publication snapshot is missing agents: {}",
                missing_agent_ids.join(", ")
            ),
        });
    }
    let snapshot = crate::local::WorkflowPublicationSnapshot {
        schema_version: 1,
        captured_at_ms: Some(crate::session::unix_epoch_ms()),
        source_session: Some(crate::local::WorkflowPublicationSourceSessionSnapshot {
            id: Some(session.id().to_string()),
            alias: session.alias().map(str::to_string),
            workspace_id: session.workspace_id().to_string(),
            worktree_id: session.worktree_id().to_string(),
        }),
        workflow: workflow.clone(),
        endpoint: Some(endpoint),
        queues: session
            .workflow_prompt_queues()
            .iter()
            .filter(|queue| queue.workflow_id() == workflow.id())
            .cloned()
            .collect(),
        schedules: session
            .workflow_schedules()
            .iter()
            .filter(|schedule| schedule.workflow_id() == workflow.id())
            .cloned()
            .collect(),
        agents,
    };
    let publication_package =
        workflow_publication_package_json(publication, &publication_value, agent_app);
    let requirements = workflow_publication_requirements_json(&snapshot.agents);
    let bindings = workflow_publication_bindings_json(&snapshot);
    let config =
        workflow_publication_gateway_config_json(publication, &publication_value, kernel_url);
    let mut files = vec![
        package_file(
            "publication.json",
            pretty_json(&publication_package)?,
            false,
        ),
        package_file("workflow.snapshot.json", pretty_json(&snapshot)?, false),
        package_file("requirements.json", pretty_json(&requirements)?, false),
        package_file("bindings.example.json", pretty_json(&bindings)?, false),
        package_file("publication.config.json", pretty_json(&config)?, false),
        package_file(
            ".env.example",
            workflow_publication_env_template(publication, kernel_url),
            false,
        ),
        package_file("run.sh", workflow_publication_launcher_script(), true),
        package_file(
            "README.md",
            workflow_publication_readme(publication, &publication_package, &config),
            false,
        ),
        package_file(
            "public/index.html",
            workflow_publication_index_html(publication),
            false,
        ),
        package_file("public/app.js", workflow_publication_app_js(), false),
        package_file(
            "public/styles.css",
            workflow_publication_styles_css(),
            false,
        ),
    ];
    if workflow_publication_package_version(agent_app) == 2 {
        if let Some(assets_dir) = agent_app_assets_dir {
            files.extend(workflow_publication_agent_app_asset_files(assets_dir)?);
        }
    }
    Ok(files)
}

fn package_file(
    path: impl Into<String>,
    content: String,
    executable: bool,
) -> crate::local::WorkflowPublicationPackageFile {
    package_file_bytes(path, content.as_bytes(), executable)
}

fn package_file_bytes(
    path: impl Into<String>,
    content: &[u8],
    executable: bool,
) -> crate::local::WorkflowPublicationPackageFile {
    crate::local::WorkflowPublicationPackageFile {
        path: path.into(),
        content_base64: base64::engine::general_purpose::STANDARD.encode(content),
        executable,
    }
}

fn pretty_json<T: serde::Serialize>(value: &T) -> Result<String, DaemonError> {
    serde_json::to_string_pretty(value)
        .map(|value| format!("{value}\n"))
        .map_err(|error| DaemonError::LocalTransport {
            operation: "export workflow publication package",
            message: format!("failed to encode package JSON: {error}"),
        })
}

fn workflow_publication_package_json(
    publication: &crate::session::WorkflowPublicationDefinition,
    publication_value: &serde_json::Value,
    agent_app: Option<&serde_json::Value>,
) -> serde_json::Value {
    let mut hook = serde_json::json!({
        "id": format!("{}-hook", publication.id()),
        "publication_id": publication.id(),
        "transport": hook_transport(publication_value),
        "endpoint_id": publication.endpoint_id(),
        "queue_ref": publication.queue_ref().unwrap_or("default"),
        "route": string_field(publication_value, "route")
            .map(str::to_string)
            .unwrap_or_else(|| default_publication_route(publication_value).to_string()),
        "methods": publication_value
            .get("methods")
            .cloned()
            .unwrap_or_else(|| default_publication_methods(publication_value)),
        "input_schema": publication_value.get("input_schema").cloned().unwrap_or(serde_json::Value::Null),
        "trace_exposure": publication.trace_exposure().cloned().unwrap_or(serde_json::Value::Null),
        "mode": string_field(publication_value, "mode").unwrap_or_else(|| default_publication_mode(publication_value)),
        "response_mode": "accepted",
    });
    if let Some(parser) = publication_value
        .get("parser")
        .cloned()
        .or_else(|| default_publication_parser(publication_value))
    {
        hook["parser"] = parser;
    }
    let mut package = serde_json::json!({
        "schema_version": 1,
        "package_version": workflow_publication_package_version(agent_app),
        "publication_id": publication.id(),
        "alias": publication.alias(),
        "source_session_id": publication.session_id(),
        "workflow_id": publication.workflow_id(),
        "default_bindings_path": "bindings.local.json",
        "hooks": [hook],
        "assets": {
            "public_dir": "public",
            "scripts_dir": "scripts",
        },
    });
    if workflow_publication_package_version(agent_app) == 2 {
        package["agent_app"] = workflow_publication_agent_app_json(agent_app);
    }
    package
}

fn workflow_publication_package_version(agent_app: Option<&serde_json::Value>) -> u32 {
    if agent_app
        .and_then(|value| value.get("enabled"))
        .and_then(|value| value.as_bool())
        == Some(true)
    {
        2
    } else {
        1
    }
}

fn workflow_publication_agent_app_json(agent_app: Option<&serde_json::Value>) -> serde_json::Value {
    let mut value = agent_app
        .cloned()
        .unwrap_or_else(|| serde_json::json!({ "enabled": true }));
    value["enabled"] = serde_json::Value::Bool(true);
    if !value.get("assets").is_some_and(|assets| assets.is_object()) {
        value["assets"] = serde_json::json!({
            "public_dir": "app",
            "index": "index.html",
        });
    }
    value
}

fn workflow_publication_agent_app_asset_files(
    assets_dir: &str,
) -> Result<Vec<crate::local::WorkflowPublicationPackageFile>, DaemonError> {
    let root = std::path::PathBuf::from(assets_dir);
    if !root.is_dir() {
        return Err(DaemonError::LocalTransport {
            operation: "export workflow publication package",
            message: format!("agent_app_assets_dir `{assets_dir}` is not a directory"),
        });
    }
    let mut files = Vec::new();
    collect_agent_app_asset_files(&root, &root, &mut files)?;
    files.sort_by(|left, right| left.path.cmp(&right.path));
    Ok(files)
}

fn collect_agent_app_asset_files(
    root: &std::path::Path,
    dir: &std::path::Path,
    files: &mut Vec<crate::local::WorkflowPublicationPackageFile>,
) -> Result<(), DaemonError> {
    let entries = std::fs::read_dir(dir).map_err(|error| DaemonError::LocalTransport {
        operation: "export workflow publication package",
        message: format!("failed to read agent app assets: {error}"),
    })?;
    for entry in entries {
        let entry = entry.map_err(|error| DaemonError::LocalTransport {
            operation: "export workflow publication package",
            message: format!("failed to read agent app asset entry: {error}"),
        })?;
        let path = entry.path();
        let metadata = entry
            .metadata()
            .map_err(|error| DaemonError::LocalTransport {
                operation: "export workflow publication package",
                message: format!(
                    "failed to stat agent app asset `{}`: {error}",
                    path.display()
                ),
            })?;
        if metadata.is_dir() {
            collect_agent_app_asset_files(root, &path, files)?;
            continue;
        }
        if !metadata.is_file() {
            continue;
        }
        let relative = path
            .strip_prefix(root)
            .map_err(|error| DaemonError::LocalTransport {
                operation: "export workflow publication package",
                message: format!(
                    "failed to relativize agent app asset `{}`: {error}",
                    path.display()
                ),
            })?;
        let relative = relative
            .to_str()
            .ok_or_else(|| DaemonError::LocalTransport {
                operation: "export workflow publication package",
                message: format!("agent app asset path is not UTF-8: {}", path.display()),
            })?;
        let content = std::fs::read(&path).map_err(|error| DaemonError::LocalTransport {
            operation: "export workflow publication package",
            message: format!(
                "failed to read agent app asset `{}`: {error}",
                path.display()
            ),
        })?;
        files.push(package_file_bytes(
            format!("app/{}", relative.replace(std::path::MAIN_SEPARATOR, "/")),
            &content,
            false,
        ));
    }
    Ok(())
}

fn workflow_publication_requirements_json(
    agents: &[crate::agent::AgentInstance],
) -> serde_json::Value {
    let grants = agents
        .iter()
        .flat_map(|agent| agent.extension_grants().iter())
        .collect::<Vec<_>>();
    serde_json::json!({
        "schema_version": 1,
        "mcps": extension_requirements_json(&grants, crate::extension::ExtensionKind::Mcp),
        "skills": extension_requirements_json(&grants, crate::extension::ExtensionKind::Skill),
        "scripts": extension_requirements_json(&grants, crate::extension::ExtensionKind::Script),
        "connectors": extension_requirements_json(&grants, crate::extension::ExtensionKind::Connector),
        "credentials": credential_requirements_json(&grants),
    })
}

fn extension_requirements_json(
    grants: &[&crate::extension::ExtensionGrant],
    kind: crate::extension::ExtensionKind,
) -> Vec<serde_json::Value> {
    let mut seen = std::collections::BTreeSet::new();
    grants
        .iter()
        .filter(|grant| grant.kind == kind)
        .filter(|grant| seen.insert(grant.name.clone()))
        .map(|grant| serde_json::json!({ "name": grant.name }))
        .collect()
}

fn credential_requirements_json(
    grants: &[&crate::extension::ExtensionGrant],
) -> Vec<serde_json::Value> {
    let mut seen = std::collections::BTreeSet::new();
    grants
        .iter()
        .filter_map(|grant| {
            let credential = grant.credential.as_deref()?.trim();
            if credential.is_empty() || !seen.insert(credential.to_string()) {
                return None;
            }
            Some(serde_json::json!({ "name": credential, "used_by": grant.name }))
        })
        .collect()
}

fn workflow_publication_bindings_json(
    snapshot: &crate::local::WorkflowPublicationSnapshot,
) -> serde_json::Value {
    let overrides = snapshot
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
                "agent_id": agent.id(),
                "node_ids": node_ids,
                "captured": {
                    "provider": agent.provider(),
                    "model": agent.model(),
                    "effort": agent.effort(),
                },
                "replacement": null,
            })
        })
        .collect::<Vec<_>>();
    serde_json::json!({
        "schema_version": 1,
        "provider_model_overrides": overrides,
    })
}

fn workflow_publication_gateway_config_json(
    publication: &crate::session::WorkflowPublicationDefinition,
    publication_value: &serde_json::Value,
    kernel_url: Option<&str>,
) -> serde_json::Value {
    let mut config = serde_json::json!({
        "publication_id": publication.id(),
        "session_id": publication.session_id(),
        "workflow_ref": publication.workflow_id(),
        "endpoint_ref": publication.endpoint_id(),
        "route": string_field(publication_value, "route")
            .map(str::to_string)
            .unwrap_or_else(|| default_publication_route(publication_value).to_string()),
        "mode": string_field(publication_value, "mode").unwrap_or_else(|| default_publication_mode(publication_value)),
    });
    if let Some(parser) = publication_value
        .get("parser")
        .cloned()
        .or_else(|| default_publication_parser(publication_value))
    {
        config["parser"] = parser;
    }
    if let Some(kernel_url) = kernel_url {
        config["kernel_endpoint"] = serde_json::Value::String(kernel_url.to_string());
    }
    if let Some(methods) = publication_value
        .get("methods")
        .filter(|value| value.as_array().is_some_and(|values| !values.is_empty()))
    {
        config["methods"] = methods.clone();
    } else {
        let default_methods = default_publication_methods(publication_value);
        if default_methods
            .as_array()
            .is_some_and(|values| !values.is_empty())
        {
            config["methods"] = default_methods;
        }
    }
    if let Some(transport) = publication_value
        .get("transport")
        .filter(|value| !value.is_null())
    {
        config["transport"] = transport.clone();
    }
    if let Some(input_schema) = publication_value
        .get("input_schema")
        .filter(|value| !value.is_null())
    {
        config["input_schema"] = input_schema.clone();
    }
    if let Some(trace_exposure) = publication.trace_exposure() {
        config["trace_exposure"] = trace_exposure.clone();
    }
    config
}

fn workflow_publication_env_template(
    publication: &crate::session::WorkflowPublicationDefinition,
    kernel_url: Option<&str>,
) -> String {
    [
        "# Copy this file to .env or export these variables before running run.sh.",
        "HOST=0.0.0.0",
        "PORT=3000",
        &format!(
            "ARROBA_KERNEL_URL={}",
            kernel_url.unwrap_or("ws://127.0.0.1:43118")
        ),
        "ARROBA_PUBLICATION_PACKAGE=./publication.json",
        &format!("ARROBA_PUBLICATION_SESSION_ID={}", publication.session_id()),
        &format!("ARROBA_PUBLICATION_ID={}", publication.id()),
        "",
    ]
    .join("\n")
}

fn workflow_publication_launcher_script() -> String {
    [
        "#!/usr/bin/env bash",
        "set -euo pipefail",
        "DIR=\"$(cd \"$(dirname \"${BASH_SOURCE[0]}\")\" && pwd)\"",
        "if [ -f \"$DIR/.env\" ]; then",
        "  set -a",
        "  . \"$DIR/.env\"",
        "  set +a",
        "fi",
        "export ARROBA_PUBLICATION_PACKAGE=\"${ARROBA_PUBLICATION_PACKAGE:-$DIR/publication.json}\"",
        "exec arroba-workflow-gateway",
        "",
    ]
    .join("\n")
}

fn workflow_publication_readme(
    publication: &crate::session::WorkflowPublicationDefinition,
    publication_package: &serde_json::Value,
    config: &serde_json::Value,
) -> String {
    let route = config
        .get("route")
        .and_then(|value| value.as_str())
        .unwrap_or("/*");
    let example_path = if route.contains('*') {
        route.replace('*', "example")
    } else {
        route.to_string()
    };
    [
        &format!("# Workflow Publication {}", publication.alias().unwrap_or(publication.id())),
        "",
        "This directory is an Arroba workflow-gateway package. It runs only when an Arroba kernel is reachable.",
        "",
        "## Files",
        "",
        "- `publication.json`: published workflow package metadata",
        "- `workflow.snapshot.json`: captured workflow, endpoint, queues, schedules, and agents",
        "- `requirements.json`: required extensions and credential handles",
        "- `bindings.example.json`: provider/model override template",
        "- `publication.config.json`: gateway config for existing scripts",
        "- `.env.example`: environment template",
        "- `run.sh`: launcher for `arroba-workflow-gateway`",
        "- `public/`: editable browser assets",
        "",
        "## Invoke",
        "",
        "```bash",
        "BASE_URL=http://127.0.0.1:3000",
        &format!("curl -sS \"$BASE_URL{}\"", example_path),
        "```",
        "",
        "## Hooks",
        "",
        "```json",
        &serde_json::to_string_pretty(&publication_package["hooks"]).unwrap_or_else(|_| "[]".to_string()),
        "```",
        "",
    ]
    .join("\n")
}

fn workflow_publication_index_html(
    publication: &crate::session::WorkflowPublicationDefinition,
) -> String {
    format!(
        r#"<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <title>{}</title>
  <link rel="stylesheet" href="./styles.css">
</head>
<body>
  <main>
    <h1>{}</h1>
    <form id="invoke-form">
      <textarea name="prompt" rows="5" autofocus></textarea>
      <button type="submit">Run</button>
    </form>
    <pre id="output"></pre>
  </main>
  <script src="./app.js" type="module"></script>
</body>
</html>
"#,
        escape_html(publication.alias().unwrap_or(publication.id())),
        escape_html(publication.alias().unwrap_or(publication.id())),
    )
}

fn workflow_publication_app_js() -> String {
    [
        "const form = document.querySelector('#invoke-form')",
        "const output = document.querySelector('#output')",
        "form?.addEventListener('submit', (event) => {",
        "  event.preventDefault()",
        "  const data = new FormData(form)",
        "  const prompt = String(data.get('prompt') ?? '').trim()",
        "  if (!prompt) return",
        "  output.textContent = 'Opening workflow invocation...'",
        "  window.location.href = `/${encodeURIComponent(prompt)}`",
        "})",
        "",
    ]
    .join("\n")
}

fn workflow_publication_styles_css() -> String {
    [
        "body { margin: 0; font-family: ui-sans-serif, system-ui, -apple-system, BlinkMacSystemFont, \"Segoe UI\", sans-serif; background: #f7f7f4; color: #202124; }",
        "main { max-width: 760px; margin: 0 auto; padding: 32px 20px; }",
        "textarea { box-sizing: border-box; width: 100%; padding: 12px; border: 1px solid #b9b9b2; border-radius: 6px; font: inherit; }",
        "button { margin-top: 12px; padding: 10px 14px; border: 0; border-radius: 6px; background: #202124; color: white; font: inherit; }",
        "pre { white-space: pre-wrap; }",
        "",
    ]
    .join("\n")
}

fn workflow_publication_package_digest(
    files: &[crate::local::WorkflowPublicationPackageFile],
) -> String {
    let mut hash = Sha256::new();
    let mut files = files.iter().collect::<Vec<_>>();
    files.sort_by(|left, right| left.path.cmp(&right.path));
    for file in files {
        hash.update(file.path.as_bytes());
        hash.update(file.content_base64.as_bytes());
        hash.update([u8::from(file.executable)]);
    }
    format!("sha256:{:x}", hash.finalize())
}

fn workflow_publication_package_archive_base64(
    files: &[crate::local::WorkflowPublicationPackageFile],
) -> Result<String, DaemonError> {
    let mut archive_bytes = Vec::new();
    {
        let encoder = GzEncoder::new(&mut archive_bytes, Compression::default());
        let mut builder = tar::Builder::new(encoder);
        let mut files = files.iter().collect::<Vec<_>>();
        files.sort_by(|left, right| left.path.cmp(&right.path));
        for file in files {
            let content = base64::engine::general_purpose::STANDARD
                .decode(&file.content_base64)
                .map_err(|error| DaemonError::LocalTransport {
                    operation: "export workflow publication package",
                    message: format!("failed to decode package file `{}`: {error}", file.path),
                })?;
            let mut header = tar::Header::new_gnu();
            header.set_size(content.len() as u64);
            header.set_mode(if file.executable { 0o755 } else { 0o644 });
            header.set_cksum();
            builder
                .append_data(&mut header, file.path.as_str(), content.as_slice())
                .map_err(|error| DaemonError::LocalTransport {
                    operation: "export workflow publication package",
                    message: format!(
                        "failed to add package file `{}` to archive: {error}",
                        file.path
                    ),
                })?;
        }
        builder
            .finish()
            .map_err(|error| DaemonError::LocalTransport {
                operation: "export workflow publication package",
                message: format!("failed to finish publication package archive: {error}"),
            })?;
        builder
            .into_inner()
            .map_err(|error| DaemonError::LocalTransport {
                operation: "export workflow publication package",
                message: format!("failed to flush publication package archive: {error}"),
            })?
            .finish()
            .map_err(|error| DaemonError::LocalTransport {
                operation: "export workflow publication package",
                message: format!(
                    "failed to finish compressed publication package archive: {error}"
                ),
            })?;
    }
    Ok(base64::engine::general_purpose::STANDARD.encode(archive_bytes))
}

fn hook_transport(publication_value: &serde_json::Value) -> serde_json::Value {
    let Some(transport) = publication_value.get("transport") else {
        return serde_json::Value::String("human_http".to_string());
    };
    if let Some(kind) = transport.get("kind").and_then(|value| value.as_str()) {
        serde_json::Value::String(kind.to_string())
    } else if transport.is_string() {
        transport.clone()
    } else {
        serde_json::Value::String("human_http".to_string())
    }
}

fn default_publication_route(publication_value: &serde_json::Value) -> &'static str {
    match hook_transport(publication_value).as_str() {
        Some("api_sse_json") => "/invoke",
        Some("websocket_json") => "/.well-known/arroba/publication/ws",
        Some("mcp") => "/mcp",
        _ => "/*",
    }
}

fn default_publication_methods(publication_value: &serde_json::Value) -> serde_json::Value {
    match hook_transport(publication_value).as_str() {
        Some("api_sse_json" | "mcp") => serde_json::json!(["POST"]),
        Some("websocket_json") => serde_json::json!([]),
        _ => serde_json::json!(["GET"]),
    }
}

fn default_publication_parser(publication_value: &serde_json::Value) -> Option<serde_json::Value> {
    match hook_transport(publication_value).as_str() {
        Some("websocket_json" | "mcp") => None,
        _ => Some(serde_json::json!({"kind": "json"})),
    }
}

fn default_publication_mode(publication_value: &serde_json::Value) -> &'static str {
    match hook_transport(publication_value).as_str() {
        Some("api_sse_json" | "websocket_json") => "async",
        _ => "sync",
    }
}

fn string_field<'a>(value: &'a serde_json::Value, field: &str) -> Option<&'a str> {
    value.get(field).and_then(|field| field.as_str())
}

fn escape_html(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}
