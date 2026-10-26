//! Workflow publication mutations.
//!
//! This module owns public endpoint publication administration. Workflow graph design and run
//! administration stay in `workflow_admin`.

use super::*;

mod materialization;
mod package;
mod reconfiguration;

pub(super) use package::workflow_publication_release_inputs_digest;
use package::{
    workflow_publication_package_archive_base64, workflow_publication_package_digest,
    workflow_publication_package_files, workflow_publication_package_version,
};

/// The App plan an export packages.
pub(super) enum ExportAppPlan<'a> {
    /// The publication's latest recorded plan, if any.
    Latest,
    /// This plan: the owner's current one, or a bound release's.
    Plan(&'a serde_json::Value),
    /// None: a release exported while the publication used no App.
    NoApps,
}

impl KernelRuntimeOwnedState {
    pub(super) fn workflow_create_publication(
        &self,
        request: crate::local::CreateWorkflowPublicationRequest,
        caller_user_id: &str,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let owned_idempotent_replay =
            request
                .operation_key
                .as_deref()
                .is_some_and(|operation_key| {
                    let operation_key = operation_key.trim();
                    !operation_key.is_empty()
                        && self
                            .session_store
                            .read()
                            .get_session(&request.session_id)
                            .ok()
                            .is_some_and(|session| {
                                session.workflow_publications().iter().any(|publication| {
                                    publication.creation_operation_key() == Some(operation_key)
                                        && publication.created_by_user_id() == caller_user_id
                                })
                            })
                });
        if !owned_idempotent_replay {
            self.ensure_workflow_endpoint_owner(
                &request.session_id,
                &request.workflow_ref,
                &request.endpoint_ref,
                caller_user_id,
                "publish workflow endpoint",
            )?;
        }
        let source_agents = self.agent_store.get_session_agents(&request.session_id);
        let publication = self
            .session_store
            .write()
            .create_workflow_publication_idempotent(
                &request.session_id,
                &request.workflow_ref,
                &request.endpoint_ref,
                request.expected_workflow_revision,
                request.operation_key,
                request.queue_ref,
                request.alias,
                request.kind,
                request.route,
                request.methods,
                request.transport,
                request.parser,
                request.input_schema,
                request.trace_exposure,
                request.mode,
                request.sync_timeout_ms,
                request.poll_ms,
                source_agents,
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

    /// Exports the publication's package with the App plan `apps` names.
    pub(super) fn workflow_export_publication_package(
        &self,
        request: crate::local::ExportWorkflowPublicationPackageRequest,
        apps: ExportAppPlan<'_>,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let mut publication = self
            .session_store
            .read()
            .resolve_workflow_publication_ref(&request.session_id, &request.publication_ref)?;
        let no_apps = matches!(apps, ExportAppPlan::NoApps);
        match apps {
            ExportAppPlan::Latest => {}
            ExportAppPlan::Plan(plan) => publication.use_apps(plan.clone()),
            ExportAppPlan::NoApps => publication.clear_apps(),
        }
        let snapshot = self
            .session_store
            .read()
            .resolve_workflow_publication_snapshot(&request.session_id, publication.id())?
            .ok_or_else(|| DaemonError::LocalTransport {
                operation: "export workflow publication package",
                message: format!(
                    "workflow trigger `{}` is missing its immutable source snapshot",
                    publication.id()
                ),
            })?;
        // Requirements follow the immutable snapshot's grants, not the source
        // agents' current ones; definitions resolve in the source workspaces.
        let workspaces = self
            .agent_store
            .get_session_agents(&request.session_id)
            .into_iter()
            .filter_map(|agent| Some((agent.id().to_string(), agent.workspace_id()?.to_string())))
            .collect();
        let extension_requirements =
            crate::workflow_publication_requirements::capture_workflow_publication_requirements(
                &snapshot.workflow,
                &snapshot.agents,
                &workspaces,
            )?;
        // App grants and the owner's App automations feeding the publication
        // both make it App-bound (a release exported before it used any App
        // re-exports without one, for its bind's digest check).
        if !no_apps
            && publication.apps().is_none()
            && (!crate::workflow_publication_requirements::app_grant_uses(
                &snapshot.workflow,
                &snapshot.agents,
            )
            .is_empty()
                || !self
                    .durable_state_store
                    .app_installations_feeding_publication(
                        publication.created_by_user_id(),
                        publication.session_id(),
                        publication.id(),
                    )?
                    .is_empty())
        {
            return Err(DaemonError::LocalTransport {
                operation: "export workflow publication package",
                message: format!(
                    "workflow trigger `{}` uses Apps but has no App plan; its owner prepares the deployment on this kernel",
                    publication.id()
                ),
            });
        }
        let package_files = workflow_publication_package_files(
            &publication,
            &snapshot,
            &extension_requirements,
            request.kernel_url.as_deref(),
            request.agent_app.as_ref(),
            request.agent_app_assets_dir.as_deref(),
        )?;
        let package_version = workflow_publication_package_version(request.agent_app.as_ref());
        let package_digest = workflow_publication_package_digest(&package_files)?;
        let package_archive_base64 = workflow_publication_package_archive_base64(&package_files)?;
        Ok(LocalDaemonResponse::WorkflowPublicationPackageExported {
            publication,
            package_version,
            package_digest,
            package_archive_base64,
            package_files,
        })
    }

    /// Protocols 377 and 378: records what a successful export packaged as
    /// the release with its package digest: the inputs digest of its files
    /// and, for an App-bound publication, the App plan.
    pub(super) fn record_workflow_publication_release(
        &self,
        session_id: &str,
        publication_id: &str,
        package_digest: &str,
        package_files: &[crate::local::WorkflowPublicationPackageFile],
        plan: Option<serde_json::Value>,
    ) -> Result<crate::session::RuntimeSession, DaemonError> {
        let inputs_digest = workflow_publication_release_inputs_digest(package_files)?;
        self.session_store
            .write()
            .record_workflow_publication_release(
                session_id,
                publication_id,
                package_digest,
                &inputs_digest,
                plan,
            )?;
        self.session_snapshot_without_projection_update(session_id)
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
            open_url: open_url.clone(),
            viewer_url: open_url,
            access,
            expires_at_ms,
        })
    }

    pub(super) fn workflow_materialize_publication(
        &self,
        request: crate::local::MaterializeWorkflowPublicationRequest,
        caller_user_id: &str,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        self.workflow_materialize_publication_as(
            request,
            caller_user_id,
            crate::session::WORKFLOW_PUBLICATION_KIND_INGRESS,
            Some("default".to_string()),
            None,
        )
    }

    /// Materializes a publication of `kind` with `queue_ref`. A hosted runtime
    /// serves ingress; a deployment copy (P1.20) keeps its source's kind, so an
    /// App-event trigger stays event-based. `location` (workspace, worktree)
    /// places the session and its agents instead of the snapshot's portable
    /// workspace.
    pub(super) fn workflow_materialize_publication_as(
        &self,
        request: crate::local::MaterializeWorkflowPublicationRequest,
        caller_user_id: &str,
        kind: &str,
        queue_ref: Option<String>,
        location: Option<(String, String)>,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let runtime_key = materialization::normalized_runtime_key(request.runtime_key.as_deref())?;
        // A retry must not race the first creation into a second session. This
        // lock also serializes the existing independent instance provisioning.
        let _provision_guard = self
            .workflow_instance_provision_lock
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if request.snapshot.schema_version != 1 {
            return Err(DaemonError::LocalTransport {
                operation: "materialize workflow publication",
                message: format!(
                    "unsupported workflow snapshot schema_version {}",
                    request.snapshot.schema_version
                ),
            });
        }
        let source_snapshot = request.snapshot.clone();
        let source_snapshot_digest =
            source_snapshot
                .digest()
                .map_err(|error| DaemonError::LocalTransport {
                    operation: "materialize workflow publication",
                    message: format!("failed to encode workflow snapshot: {error}"),
                })?;
        let Some(source_session) = request.snapshot.source_session.as_ref() else {
            return Err(DaemonError::LocalTransport {
                operation: "materialize workflow publication",
                message: "workflow snapshot is missing source_session".to_string(),
            });
        };
        if let Some(key) = runtime_key.as_deref() {
            if let Some(restored) = self.workflow_resume_publication_materialization(
                key,
                &request.publication_id,
                &source_snapshot_digest,
                &source_snapshot,
                caller_user_id,
            )? {
                return Ok(restored);
            }
        }
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

        let (workspace_id, worktree_id) = location.clone().unwrap_or_else(|| {
            (
                source_session.workspace_id.clone(),
                source_session.worktree_id.clone(),
            )
        });
        let session = self.session_store.create_session(
            crate::session::CreateSessionRequest::new(workspace_id, worktree_id)
                .with_owner_user_id(caller_user_id)
                .with_hidden(true),
        )?;
        let session_id = session.id().to_string();
        let mut agent_id_map = BTreeMap::new();
        for (captured_agent_id, mut agent) in captured_agents {
            if let Some((workspace_id, worktree_id)) = location.as_ref() {
                agent.set_workspace_id(Some(workspace_id.clone()));
                agent.set_worktree_id(Some(worktree_id.clone()));
            }
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
        let mut publication = crate::session::WorkflowPublicationDefinition::new_immutable(
            request.publication_id.clone(),
            session_id.clone(),
            workflow_id.clone(),
            endpoint_id,
            queue_ref,
            None,
            kind,
            None,
            Vec::new(),
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            source_snapshot.workflow.revision(),
            source_snapshot_digest,
            None,
            None,
            caller_user_id.to_string(),
        );
        if let Some(key) = runtime_key {
            publication.set_runtime_materialization(
                crate::session::WorkflowPublicationRuntimeMaterialization {
                    key,
                    agent_id_map: agent_id_map.clone(),
                },
            );
        }
        self.session_store.write().restore_workflow_publication(
            &session_id,
            publication,
            Some(source_snapshot),
        )?;
        let session = self.session_snapshot_without_projection_update(&session_id)?;
        // Workflow hot-state writes do not create durable sessions or agents.
        // Commit their creation together before accepting the materialization;
        // a retry after a crash can then recover the same runtime identity.
        if let Err(error) = self.durable_state_store.append_event(
            "workflow.publication.materialized",
            Some(session_id.clone()),
            serde_json::json!({ "session": &session }),
        ) {
            for agent in self
                .agent_store
                .list_agents()
                .iter()
                .filter(|a| a.session_id() == session_id)
            {
                self.forget_room_computer_access(agent.id());
            }
            self.agent_store.remove_session_agents(&session_id);
            let _ = self
                .session_store
                .write()
                .delete_session_with_project_cleanup(&session_id);
            self.session_projection.remove(&session_id);
            return Err(error);
        }
        Ok(LocalDaemonResponse::WorkflowPublicationMaterialized {
            publication_id: request.publication_id,
            session,
            agent_id_map,
        })
    }
}
