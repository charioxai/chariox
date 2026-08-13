//! Workflow publication mutations.
//!
//! This module owns public endpoint publication administration. Workflow graph design and run
//! administration stay in `workflow_admin`.

use super::*;

mod package;

use package::{
    workflow_publication_package_archive_base64, workflow_publication_package_digest,
    workflow_publication_package_files, workflow_publication_package_version,
};

impl KernelRuntimeOwnedState {
    pub(super) fn workflow_create_event_binding(
        &self,
        request: crate::local::CreateWorkflowEventBindingRequest,
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
                format!("workflow publication `{}`", publication.id()),
                "create workflow event binding",
            ));
        }
        let binding = self.session_store.write().create_workflow_event_binding(
            &request.session_id,
            &request.publication_ref,
            request.generator_id,
            request.generator_version,
            request.manifest_digest,
            request.connection_id,
            request.connection_scope,
            request.event_type,
            request.event_type_version,
            request.filter,
            request.environment_id,
            request.queue_ref,
        )?;
        Ok(LocalDaemonResponse::WorkflowEventBindingCreated {
            binding,
            session: self.workflow_session(&request.session_id)?,
        })
    }

    pub(super) fn workflow_list_event_bindings(
        &self,
        request: crate::local::ListWorkflowEventBindingsRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        Ok(LocalDaemonResponse::WorkflowEventBindingsListed {
            bindings: self.session_store.read().list_workflow_event_bindings(
                &request.session_id,
                request.publication_ref.as_deref(),
            )?,
        })
    }

    pub(super) fn workflow_set_event_binding_status(
        &self,
        request: crate::local::SetWorkflowEventBindingStatusRequest,
        caller_user_id: &str,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let binding = self
            .session_store
            .read()
            .get_session(&request.session_id)?
            .workflow_event_bindings()
            .iter()
            .find(|binding| binding.id == request.binding_id)
            .cloned()
            .ok_or_else(|| DaemonError::LocalTransport {
                operation: "set workflow event binding status",
                message: format!(
                    "workflow event binding `{}` was not found",
                    request.binding_id
                ),
            })?;
        let publication = self
            .session_store
            .read()
            .resolve_workflow_publication_ref(&request.session_id, &binding.publication_id)?;
        if publication.created_by_user_id() != caller_user_id {
            return Err(Self::deny_owner(
                caller_user_id,
                publication.created_by_user_id(),
                format!("workflow publication `{}`", publication.id()),
                "set workflow event binding status",
            ));
        }
        let binding = self
            .session_store
            .write()
            .set_workflow_event_binding_status(
                &request.session_id,
                &request.binding_id,
                request.status,
            )?;
        Ok(LocalDaemonResponse::WorkflowEventBindingUpdated {
            binding,
            session: self.workflow_session(&request.session_id)?,
        })
    }

    pub(super) fn workflow_transfer_event_binding(
        &self,
        request: crate::local::TransferWorkflowEventBindingRequest,
        caller_user_id: &str,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let source_before = self.session_snapshot(&request.source_session_id)?;
        let target_before = if request.source_session_id == request.target_session_id {
            None
        } else {
            Some(self.session_snapshot(&request.target_session_id)?)
        };
        let source_binding = self
            .session_store
            .read()
            .get_session(&request.source_session_id)?
            .workflow_event_bindings()
            .iter()
            .find(|binding| binding.id == request.binding_id)
            .cloned()
            .ok_or_else(|| DaemonError::LocalTransport {
                operation: "transfer workflow event binding",
                message: format!(
                    "workflow event binding `{}` was not found",
                    request.binding_id
                ),
            })?;
        let source_publication = self.session_store.read().resolve_workflow_publication_ref(
            &request.source_session_id,
            &source_binding.publication_id,
        )?;
        let target_publication = self.session_store.read().resolve_workflow_publication_ref(
            &request.target_session_id,
            &request.target_publication_ref,
        )?;
        for publication in [&source_publication, &target_publication] {
            if publication.created_by_user_id() != caller_user_id {
                return Err(Self::deny_owner(
                    caller_user_id,
                    publication.created_by_user_id(),
                    format!("workflow publication `{}`", publication.id()),
                    "transfer workflow event binding",
                ));
            }
        }
        let binding = self.session_store.write().transfer_workflow_event_binding(
            &request.source_session_id,
            &request.binding_id,
            &request.target_session_id,
            &request.target_publication_ref,
        )?;
        let target_session = self.workflow_session(&request.target_session_id)?;
        if request.source_session_id != request.target_session_id {
            let source_session = self.workflow_session(&request.source_session_id)?;
            if let Err(error) = self.durable_state_store.append_event(
                "sessions.updated",
                None,
                serde_json::json!({
                    "sessions": [&source_session, &target_session],
                    "reason": "workflow_event_binding_transferred",
                }),
            ) {
                let mut sessions = self.session_store.write();
                sessions.restore_session(source_before);
                if let Some(target_before) = target_before {
                    sessions.restore_session(target_before);
                }
                return Err(error);
            }
        } else if let Err(error) = self.persist_workflow_runtime_session(
            &request.source_session_id,
            "workflow_event_binding_transferred",
        ) {
            self.session_store.write().restore_session(source_before);
            return Err(error);
        }
        Ok(LocalDaemonResponse::WorkflowEventBindingTransferred {
            binding,
            session: target_session,
        })
    }

    pub(super) fn workflow_test_event_delivery_envelope(
        &self,
        request: crate::local::TestWorkflowEventBindingRequest,
        caller_user_id: &str,
    ) -> Result<chariox_event_protocol::EventDeliveryEnvelope, DaemonError> {
        let binding = self
            .session_store
            .read()
            .get_session(&request.session_id)?
            .workflow_event_bindings()
            .iter()
            .find(|binding| binding.id == request.binding_id)
            .cloned()
            .ok_or_else(|| DaemonError::LocalTransport {
                operation: "test workflow event binding",
                message: format!(
                    "workflow event binding `{}` was not found",
                    request.binding_id
                ),
            })?;
        let publication = self
            .session_store
            .read()
            .resolve_workflow_publication_ref(&request.session_id, &binding.publication_id)?;
        if publication.created_by_user_id() != caller_user_id {
            return Err(Self::deny_owner(
                caller_user_id,
                publication.created_by_user_id(),
                format!("workflow publication `{}`", publication.id()),
                "test workflow event binding",
            ));
        }
        if !binding.active() {
            let status = match binding.status {
                crate::session::WorkflowEventBindingStatus::Active => "active",
                crate::session::WorkflowEventBindingStatus::Paused => "paused",
                crate::session::WorkflowEventBindingStatus::Conflict => "in conflict",
                crate::session::WorkflowEventBindingStatus::Tombstoned => "tombstoned",
            };
            return Err(DaemonError::LocalTransport {
                operation: "test workflow event binding",
                message: format!("workflow event binding is {status}"),
            });
        }
        if !publication.enabled() {
            return Err(DaemonError::LocalTransport {
                operation: "test workflow event binding",
                message:
                    "the owning workflow publication is disabled; create a new event-based publication before testing"
                        .to_string(),
            });
        }
        let now_ms = crate::session::unix_epoch_ms();
        Ok(chariox_event_protocol::EventDeliveryEnvelope {
            delivery_id: format!("test-delivery-{}-{now_ms}", binding.id),
            binding_id: binding.id,
            event_type: binding.event_type,
            event_type_version: binding.event_type_version,
            occurrence_id: format!("test-occurrence-{now_ms}"),
            occurred_at: chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
            prompt: request
                .prompt
                .unwrap_or_else(|| "Process this Chariox event notification test.".to_string()),
            artifacts: Vec::new(),
            metadata: serde_json::json!({"test": true}),
            expires_at_ms: now_ms.saturating_add(60 * 60 * 1000),
        })
    }

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

    pub(super) fn workflow_export_publication_package(
        &self,
        request: crate::local::ExportWorkflowPublicationPackageRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let publication = self
            .session_store
            .read()
            .resolve_workflow_publication_ref(&request.session_id, &request.publication_ref)?;
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
        let event_bindings = self
            .session_store
            .read()
            .get_session(&request.session_id)?
            .workflow_event_bindings()
            .iter()
            .filter(|binding| {
                binding.publication_id == publication.id()
                    && binding.status != crate::session::WorkflowEventBindingStatus::Tombstoned
            })
            .cloned()
            .collect::<Vec<_>>();
        let package_files = workflow_publication_package_files(
            &publication,
            &snapshot,
            &event_bindings,
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
        let publication = crate::session::WorkflowPublicationDefinition::new_immutable(
            request.publication_id.clone(),
            session_id.clone(),
            workflow_id.clone(),
            endpoint_id,
            Some("default".to_string()),
            None,
            crate::session::WORKFLOW_PUBLICATION_KIND_INGRESS,
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
        self.session_store.write().restore_workflow_publication(
            &session_id,
            publication,
            Some(source_snapshot),
        )?;
        let session = self.workflow_session(&session_id)?;
        Ok(LocalDaemonResponse::WorkflowPublicationMaterialized {
            publication_id: request.publication_id,
            session,
            agent_id_map,
        })
    }
}
