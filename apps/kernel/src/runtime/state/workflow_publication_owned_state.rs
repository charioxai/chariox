//! Workflow publication mutations.
//!
//! This module owns public endpoint publication administration. Workflow graph design and run
//! administration stay in `workflow_admin`.

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
        if let Some(watchdog) = request
            .snapshot
            .watchdogs
            .iter()
            .find(|watchdog| watchdog.workflow_id() != workflow_id)
        {
            return Err(DaemonError::LocalTransport {
                operation: "materialize workflow publication",
                message: format!(
                    "snapshot watchdog `{}` belongs to workflow `{}` instead of `{workflow_id}`",
                    watchdog.id(),
                    watchdog.workflow_id()
                ),
            });
        }
        if let Some(watchdog) = request.snapshot.watchdogs.iter().find(|watchdog| {
            request
                .snapshot
                .workflow
                .endpoint(watchdog.endpoint_id())
                .is_none()
        }) {
            return Err(DaemonError::LocalTransport {
                operation: "materialize workflow publication",
                message: format!(
                    "snapshot watchdog `{}` references missing endpoint `{}`",
                    watchdog.id(),
                    watchdog.endpoint_id()
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
            let materialized = self
                .agent_store
                .materialize_publication_agent(agent, &session_id, Some(caller_user_id));
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
            request.snapshot.watchdogs,
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
