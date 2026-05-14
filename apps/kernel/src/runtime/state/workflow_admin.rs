//! Workflow administrative mutations.
//!
//! Owns workflow CRUD, endpoint edits, watchdog updates, and queue-facing commands that alter
//! workflow definitions rather than executing an individual node.

use super::*;

impl KernelRuntimeOwnedState {
    pub(super) fn deny_owner(
        user_id: &str,
        owner_user_id: &str,
        resource: String,
        operation: &'static str,
    ) -> DaemonError {
        DaemonError::OwnershipAccessDenied {
            user_id: user_id.to_string(),
            owner_user_id: owner_user_id.to_string(),
            resource,
            operation,
        }
    }

    fn ensure_workflow_node_owner(
        &self,
        session_id: &str,
        workflow_ref: &str,
        node_id: &str,
        user_id: &str,
        operation: &'static str,
    ) -> Result<(), DaemonError> {
        let workflow = self
            .session_store
            .read()
            .resolve_workflow_ref(session_id, workflow_ref)?;
        let node = workflow
            .node(node_id)
            .ok_or_else(|| DaemonError::WorkflowNodeNotFound {
                session_id: session_id.to_string(),
                workflow_id: workflow.id().to_string(),
                node_id: node_id.to_string(),
            })?;
        if node.owner_user_id() == user_id {
            Ok(())
        } else {
            Err(Self::deny_owner(
                user_id,
                node.owner_user_id(),
                format!("workflow node `{node_id}`"),
                operation,
            ))
        }
    }

    pub(super) fn ensure_workflow_endpoint_owner(
        &self,
        session_id: &str,
        workflow_ref: &str,
        endpoint_ref: &str,
        user_id: &str,
        operation: &'static str,
    ) -> Result<(), DaemonError> {
        let endpoint = self.session_store.read().resolve_workflow_endpoint_ref(
            session_id,
            workflow_ref,
            endpoint_ref,
        )?;
        if endpoint.owner_user_id() == user_id {
            Ok(())
        } else {
            Err(Self::deny_owner(
                user_id,
                endpoint.owner_user_id(),
                format!("workflow endpoint `{endpoint_ref}`"),
                operation,
            ))
        }
    }

    pub(super) fn ensure_workflow_revision(
        &self,
        session_id: &str,
        workflow_ref: &str,
        expected_revision: Option<u64>,
    ) -> Result<(), DaemonError> {
        let Some(expected_revision) = expected_revision else {
            return Ok(());
        };
        let workflow = self
            .session_store
            .read()
            .resolve_workflow_ref(session_id, workflow_ref)?;
        let current_revision = workflow.revision();
        if current_revision == expected_revision {
            Ok(())
        } else {
            Err(DaemonError::WorkflowRevisionConflict {
                session_id: session_id.to_string(),
                workflow_id: workflow.id().to_string(),
                expected_revision,
                current_revision,
            })
        }
    }

    fn ensure_workflow_edge_incident_to_owner(
        &self,
        session_id: &str,
        workflow_ref: &str,
        edge_id: &str,
        user_id: &str,
        operation: &'static str,
    ) -> Result<(), DaemonError> {
        let workflow = self
            .session_store
            .read()
            .resolve_workflow_ref(session_id, workflow_ref)?;
        let edge = workflow
            .edge(edge_id)
            .ok_or_else(|| DaemonError::WorkflowEdgeNotFound {
                session_id: session_id.to_string(),
                workflow_id: workflow.id().to_string(),
                edge_id: edge_id.to_string(),
            })?;
        let from_owner = workflow
            .node(edge.from_node_id())
            .map(|node| node.owner_user_id());
        let to_owner = workflow
            .node(edge.to_node_id())
            .map(|node| node.owner_user_id());
        if from_owner == Some(user_id) || to_owner == Some(user_id) {
            Ok(())
        } else {
            Err(Self::deny_owner(
                user_id,
                edge.created_by_user_id(),
                format!("workflow edge `{edge_id}`"),
                operation,
            ))
        }
    }

    pub(super) fn workflow_session(
        &self,
        session_id: &str,
    ) -> Result<crate::session::RuntimeSession, DaemonError> {
        self.session_snapshot(session_id)
    }

    pub(super) fn workflow_create_workflow(
        &self,
        request: crate::local::CreateWorkflowRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let workflow = self
            .session_store
            .write()
            .create_workflow(&request.session_id, request.alias)?;
        let session = self.workflow_session(&request.session_id)?;
        Ok(LocalDaemonResponse::WorkflowCreated { workflow, session })
    }

    pub(super) fn workflow_apply_design_op(
        &self,
        request: crate::local::ApplyWorkflowDesignOpRequest,
        caller_user_id: &str,
    ) -> Result<crate::session::RuntimeSession, DaemonError> {
        self.session_store.write().apply_workflow_design_op(
            &request.session_id,
            request.op,
            caller_user_id.to_string(),
        )?;
        self.workflow_session(&request.session_id)
    }

    pub(super) fn workflow_alias_workflow(
        &self,
        request: crate::local::AliasWorkflowRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        self.ensure_workflow_revision(
            &request.session_id,
            &request.workflow_ref,
            request.expected_workflow_revision,
        )?;
        let workflow = self.session_store.write().assign_workflow_alias(
            &request.session_id,
            &request.workflow_ref,
            request.alias,
        )?;
        let session = self.workflow_session(&request.session_id)?;
        Ok(LocalDaemonResponse::WorkflowAliased { workflow, session })
    }

    pub(super) fn workflow_list_workflows(
        &self,
        request: crate::local::ListWorkflowsRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        Ok(LocalDaemonResponse::WorkflowsListed {
            workflows: self
                .session_store
                .read()
                .list_workflows(&request.session_id)?,
        })
    }

    pub(super) fn workflow_resolve_workflow(
        &self,
        request: crate::local::ResolveWorkflowRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        Ok(LocalDaemonResponse::WorkflowResolved {
            workflow: self
                .session_store
                .read()
                .resolve_workflow_ref(&request.session_id, &request.workflow_ref)?,
        })
    }

    pub(super) fn workflow_create_endpoint(
        &self,
        request: crate::local::CreateWorkflowEndpointRequest,
        caller_user_id: &str,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        self.ensure_workflow_revision(
            &request.session_id,
            &request.workflow_ref,
            request.expected_workflow_revision,
        )?;
        self.ensure_workflow_node_owner(
            &request.session_id,
            &request.workflow_ref,
            &request.entry_node_id,
            caller_user_id,
            "create workflow endpoint",
        )?;
        let endpoint = self.session_store.write().create_workflow_endpoint(
            &request.session_id,
            &request.workflow_ref,
            &request.entry_node_id,
            request.alias,
        )?;
        let endpoint = self.session_store.write().set_workflow_endpoint_owner(
            &request.session_id,
            &request.workflow_ref,
            endpoint.id(),
            caller_user_id.to_string(),
        )?;
        let workflow = self
            .session_store
            .read()
            .resolve_workflow_ref(&request.session_id, &request.workflow_ref)?;
        let session = self.workflow_session(&request.session_id)?;
        Ok(LocalDaemonResponse::WorkflowEndpointCreated {
            endpoint,
            workflow,
            session,
        })
    }

    pub(super) fn workflow_alias_endpoint(
        &self,
        request: crate::local::AliasWorkflowEndpointRequest,
        caller_user_id: &str,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        self.ensure_workflow_revision(
            &request.session_id,
            &request.workflow_ref,
            request.expected_workflow_revision,
        )?;
        self.ensure_workflow_endpoint_owner(
            &request.session_id,
            &request.workflow_ref,
            &request.endpoint_ref,
            caller_user_id,
            "alias workflow endpoint",
        )?;
        let endpoint = self.session_store.write().assign_workflow_endpoint_alias(
            &request.session_id,
            &request.workflow_ref,
            &request.endpoint_ref,
            request.alias,
        )?;
        let workflow = self
            .session_store
            .read()
            .resolve_workflow_ref(&request.session_id, &request.workflow_ref)?;
        let session = self.workflow_session(&request.session_id)?;
        Ok(LocalDaemonResponse::WorkflowEndpointAliased {
            endpoint,
            workflow,
            session,
        })
    }

    pub(super) fn workflow_bind_endpoint(
        &self,
        request: crate::local::BindWorkflowEndpointRequest,
        caller_user_id: &str,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        self.ensure_workflow_revision(
            &request.session_id,
            &request.workflow_ref,
            request.expected_workflow_revision,
        )?;
        self.ensure_workflow_endpoint_owner(
            &request.session_id,
            &request.workflow_ref,
            &request.endpoint_ref,
            caller_user_id,
            "bind workflow endpoint",
        )?;
        self.ensure_workflow_node_owner(
            &request.session_id,
            &request.workflow_ref,
            &request.entry_node_id,
            caller_user_id,
            "bind workflow endpoint",
        )?;
        let endpoint = self.session_store.write().bind_workflow_endpoint(
            &request.session_id,
            &request.workflow_ref,
            &request.endpoint_ref,
            &request.entry_node_id,
        )?;
        let workflow = self
            .session_store
            .read()
            .resolve_workflow_ref(&request.session_id, &request.workflow_ref)?;
        let session = self.workflow_session(&request.session_id)?;
        Ok(LocalDaemonResponse::WorkflowEndpointBound {
            endpoint,
            workflow,
            session,
        })
    }

    pub(super) fn workflow_add_node(
        &self,
        request: crate::local::AddWorkflowNodeRequest,
        caller_user_id: &str,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        self.ensure_workflow_revision(
            &request.session_id,
            &request.workflow_ref,
            request.expected_workflow_revision,
        )?;
        let agent = if let Some(agent) = self
            .agent_store
            .get_session_agents(&request.session_id)
            .into_iter()
            .find(|agent| agent.id() == request.agent_id)
        {
            agent
        } else {
            return Err(DaemonError::AgentNotFound {
                agent_id: request.agent_id,
            });
        };
        if agent.owner_user_id() != caller_user_id {
            return Err(Self::deny_owner(
                caller_user_id,
                agent.owner_user_id(),
                format!("agent `{}`", request.agent_id),
                "add workflow node",
            ));
        }
        let node = self.session_store.write().add_workflow_node_owned(
            &request.session_id,
            &request.workflow_ref,
            &request.agent_id,
            caller_user_id.to_string(),
            request.agent_id.clone(),
        )?;
        let workflow = self
            .session_store
            .read()
            .resolve_workflow_ref(&request.session_id, &request.workflow_ref)?;
        let session = self.workflow_session(&request.session_id)?;
        Ok(LocalDaemonResponse::WorkflowNodeAdded {
            node,
            workflow,
            session,
        })
    }

    pub(super) fn workflow_remove_node(
        &self,
        request: crate::local::RemoveWorkflowNodeRequest,
        caller_user_id: &str,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        self.ensure_workflow_revision(
            &request.session_id,
            &request.workflow_ref,
            request.expected_workflow_revision,
        )?;
        self.ensure_workflow_node_owner(
            &request.session_id,
            &request.workflow_ref,
            &request.node_id,
            caller_user_id,
            "remove workflow node",
        )?;
        let node = self.session_store.write().remove_workflow_node(
            &request.session_id,
            &request.workflow_ref,
            &request.node_id,
        )?;
        let workflow = self
            .session_store
            .read()
            .resolve_workflow_ref(&request.session_id, &request.workflow_ref)?;
        let session = self.workflow_session(&request.session_id)?;
        Ok(LocalDaemonResponse::WorkflowNodeRemoved {
            node,
            workflow,
            session,
        })
    }

    pub(super) fn workflow_update_node_instructions(
        &self,
        request: crate::local::UpdateWorkflowNodeInstructionsRequest,
        caller_user_id: &str,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        self.ensure_workflow_revision(
            &request.session_id,
            &request.workflow_ref,
            request.expected_workflow_revision,
        )?;
        self.ensure_workflow_node_owner(
            &request.session_id,
            &request.workflow_ref,
            &request.node_id,
            caller_user_id,
            "update workflow node instructions",
        )?;
        let node = self
            .session_store
            .write()
            .update_workflow_node_instructions(
                &request.session_id,
                &request.workflow_ref,
                &request.node_id,
                request.instructions,
            )?;
        let workflow = self
            .session_store
            .read()
            .resolve_workflow_ref(&request.session_id, &request.workflow_ref)?;
        let session = self.workflow_session(&request.session_id)?;
        Ok(LocalDaemonResponse::WorkflowNodeInstructionsUpdated {
            node,
            workflow,
            session,
        })
    }

    pub(super) fn workflow_set_node_can_complete_run(
        &self,
        request: crate::local::SetWorkflowNodeCanCompleteRunRequest,
        caller_user_id: &str,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        self.ensure_workflow_revision(
            &request.session_id,
            &request.workflow_ref,
            request.expected_workflow_revision,
        )?;
        self.ensure_workflow_node_owner(
            &request.session_id,
            &request.workflow_ref,
            &request.node_id,
            caller_user_id,
            "set workflow node completion policy",
        )?;
        let node = self
            .session_store
            .write()
            .set_workflow_node_can_complete_run(
                &request.session_id,
                &request.workflow_ref,
                &request.node_id,
                request.can_complete_workflow_run,
            )?;
        let workflow = self
            .session_store
            .read()
            .resolve_workflow_ref(&request.session_id, &request.workflow_ref)?;
        let session = self.workflow_session(&request.session_id)?;
        Ok(LocalDaemonResponse::WorkflowNodeCanCompleteRunUpdated {
            node,
            workflow,
            session,
        })
    }

    pub(super) fn workflow_set_node_can_emit_intermediate_output(
        &self,
        request: crate::local::SetWorkflowNodeCanEmitIntermediateOutputRequest,
        caller_user_id: &str,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        self.ensure_workflow_revision(
            &request.session_id,
            &request.workflow_ref,
            request.expected_workflow_revision,
        )?;
        self.ensure_workflow_node_owner(
            &request.session_id,
            &request.workflow_ref,
            &request.node_id,
            caller_user_id,
            "set workflow node intermediate output policy",
        )?;
        let node = self
            .session_store
            .write()
            .set_workflow_node_can_emit_intermediate_output(
                &request.session_id,
                &request.workflow_ref,
                &request.node_id,
                request.can_emit_intermediate_workflow_run_output,
            )?;
        let workflow = self
            .session_store
            .read()
            .resolve_workflow_ref(&request.session_id, &request.workflow_ref)?;
        let session = self.workflow_session(&request.session_id)?;
        Ok(
            LocalDaemonResponse::WorkflowNodeCanEmitIntermediateOutputUpdated {
                node,
                workflow,
                session,
            },
        )
    }

    pub(super) fn workflow_set_node_intermediate_output_schema(
        &self,
        request: crate::local::SetWorkflowNodeIntermediateOutputSchemaRequest,
        caller_user_id: &str,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        self.ensure_workflow_revision(
            &request.session_id,
            &request.workflow_ref,
            request.expected_workflow_revision,
        )?;
        self.ensure_workflow_node_owner(
            &request.session_id,
            &request.workflow_ref,
            &request.node_id,
            caller_user_id,
            "set workflow node intermediate output schema",
        )?;
        let node = self
            .session_store
            .write()
            .set_workflow_node_intermediate_output_schema_ref(
                &request.session_id,
                &request.workflow_ref,
                &request.node_id,
                request.intermediate_output_schema_ref,
            )?;
        let workflow = self
            .session_store
            .read()
            .resolve_workflow_ref(&request.session_id, &request.workflow_ref)?;
        let session = self.workflow_session(&request.session_id)?;
        Ok(
            LocalDaemonResponse::WorkflowNodeIntermediateOutputSchemaUpdated {
                node,
                workflow,
                session,
            },
        )
    }

    pub(super) fn workflow_set_node_max_turns(
        &self,
        request: crate::local::SetWorkflowNodeMaxTurnsRequest,
        caller_user_id: &str,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        self.ensure_workflow_revision(
            &request.session_id,
            &request.workflow_ref,
            request.expected_workflow_revision,
        )?;
        self.ensure_workflow_node_owner(
            &request.session_id,
            &request.workflow_ref,
            &request.node_id,
            caller_user_id,
            "set workflow node max turns",
        )?;
        let node = self.session_store.write().set_workflow_node_max_turns(
            &request.session_id,
            &request.workflow_ref,
            &request.node_id,
            request.max_turns,
        )?;
        let workflow = self
            .session_store
            .read()
            .resolve_workflow_ref(&request.session_id, &request.workflow_ref)?;
        let session = self.workflow_session(&request.session_id)?;
        Ok(LocalDaemonResponse::WorkflowNodeMaxTurnsUpdated {
            node,
            workflow,
            session,
        })
    }

    pub(super) fn workflow_add_edge(
        &self,
        request: crate::local::AddWorkflowEdgeRequest,
        caller_user_id: &str,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        self.ensure_workflow_revision(
            &request.session_id,
            &request.workflow_ref,
            request.expected_workflow_revision,
        )?;
        let workflow = self
            .session_store
            .read()
            .resolve_workflow_ref(&request.session_id, &request.workflow_ref)?;
        let from_owner = workflow
            .node(&request.from_node_id)
            .map(|node| node.owner_user_id())
            .ok_or_else(|| DaemonError::WorkflowNodeNotFound {
                session_id: request.session_id.clone(),
                workflow_id: workflow.id().to_string(),
                node_id: request.from_node_id.clone(),
            })?;
        let to_owner = workflow
            .node(&request.to_node_id)
            .map(|node| node.owner_user_id())
            .ok_or_else(|| DaemonError::WorkflowNodeNotFound {
                session_id: request.session_id.clone(),
                workflow_id: workflow.id().to_string(),
                node_id: request.to_node_id.clone(),
            })?;
        if from_owner != caller_user_id && to_owner != caller_user_id {
            return Err(Self::deny_owner(
                caller_user_id,
                from_owner,
                format!(
                    "workflow edge `{} -> {}`",
                    request.from_node_id, request.to_node_id
                ),
                "add workflow edge",
            ));
        }
        let edge = self.session_store.write().add_workflow_edge_owned(
            &request.session_id,
            &request.workflow_ref,
            &request.from_node_id,
            &request.to_node_id,
            caller_user_id.to_string(),
            request.output_schema_ref,
            request.validation_policy,
        )?;
        let workflow = self
            .session_store
            .read()
            .resolve_workflow_ref(&request.session_id, &request.workflow_ref)?;
        let session = self.workflow_session(&request.session_id)?;
        Ok(LocalDaemonResponse::WorkflowEdgeAdded {
            edge,
            workflow,
            session,
        })
    }

    pub(super) fn workflow_remove_edge(
        &self,
        request: crate::local::RemoveWorkflowEdgeRequest,
        caller_user_id: &str,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        self.ensure_workflow_revision(
            &request.session_id,
            &request.workflow_ref,
            request.expected_workflow_revision,
        )?;
        self.ensure_workflow_edge_incident_to_owner(
            &request.session_id,
            &request.workflow_ref,
            &request.edge_id,
            caller_user_id,
            "remove workflow edge",
        )?;
        let edge = self.session_store.write().remove_workflow_edge(
            &request.session_id,
            &request.workflow_ref,
            &request.edge_id,
        )?;
        let workflow = self
            .session_store
            .read()
            .resolve_workflow_ref(&request.session_id, &request.workflow_ref)?;
        let session = self.workflow_session(&request.session_id)?;
        Ok(LocalDaemonResponse::WorkflowEdgeRemoved {
            edge,
            workflow,
            session,
        })
    }

    pub(super) fn workflow_update_canvas_layout(
        &self,
        request: crate::local::UpdateWorkflowCanvasLayoutRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let layout = self.session_store.write().update_workflow_canvas_layout(
            &request.session_id,
            &request.workflow_ref,
            request.patches,
        )?;
        let workflow = self
            .session_store
            .read()
            .resolve_workflow_ref(&request.session_id, &request.workflow_ref)?;
        let session = self.workflow_session(&request.session_id)?;
        Ok(LocalDaemonResponse::WorkflowCanvasLayoutUpdated {
            layout,
            workflow,
            session,
        })
    }

    pub(super) fn workflow_set_flush_context(
        &self,
        request: crate::local::SetWorkflowFlushContextRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        self.ensure_workflow_revision(
            &request.session_id,
            &request.workflow_ref,
            request.expected_workflow_revision,
        )?;
        let workflow = self
            .session_store
            .write()
            .set_workflow_flush_agent_context_before_run(
                &request.session_id,
                &request.workflow_ref,
                request.flush_agent_context_before_run,
            )?;
        let session = self.workflow_session(&request.session_id)?;
        Ok(LocalDaemonResponse::WorkflowFlushContextUpdated { workflow, session })
    }

    pub(super) fn workflow_set_run_output_schema(
        &self,
        request: crate::local::SetWorkflowRunOutputSchemaRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        self.ensure_workflow_revision(
            &request.session_id,
            &request.workflow_ref,
            request.expected_workflow_revision,
        )?;
        let workflow = self
            .session_store
            .write()
            .set_workflow_run_output_schema_ref(
                &request.session_id,
                &request.workflow_ref,
                request.run_output_schema_ref,
            )?;
        let session = self.workflow_session(&request.session_id)?;
        Ok(LocalDaemonResponse::WorkflowRunOutputSchemaUpdated { workflow, session })
    }

    pub(super) fn workflow_set_intermediate_output_schema(
        &self,
        request: crate::local::SetWorkflowIntermediateOutputSchemaRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        self.ensure_workflow_revision(
            &request.session_id,
            &request.workflow_ref,
            request.expected_workflow_revision,
        )?;
        let workflow = self
            .session_store
            .write()
            .set_workflow_intermediate_output_schema_ref(
                &request.session_id,
                &request.workflow_ref,
                request.intermediate_output_schema_ref,
            )?;
        let session = self.workflow_session(&request.session_id)?;
        Ok(LocalDaemonResponse::WorkflowIntermediateOutputSchemaUpdated { workflow, session })
    }

    pub(super) fn workflow_set_launch_policy(
        &self,
        request: crate::local::SetWorkflowLaunchPolicyRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let session = self
            .session_store
            .write()
            .set_workflow_launch_policy(&request.session_id, request.policy)?;
        let mut session = session;
        session.set_agents(self.agent_store.get_session_agents(&request.session_id));
        self.project_session_runtime_view(&mut session);
        self.session_projection.update(session.clone());
        Ok(LocalDaemonResponse::WorkflowLaunchPolicyUpdated { session })
    }

    pub(super) fn workflow_list_runs(
        &self,
        request: crate::local::ListWorkflowRunsRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        Ok(LocalDaemonResponse::WorkflowRunsListed {
            workflow_runs: self
                .session_store
                .read()
                .list_workflow_runs(&request.session_id, request.workflow_ref.as_deref())?,
        })
    }

    pub(super) fn workflow_get_run(
        &self,
        request: crate::local::GetWorkflowRunRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        Ok(LocalDaemonResponse::WorkflowRun {
            workflow_run: self
                .session_store
                .read()
                .resolve_workflow_run_ref(&request.session_id, &request.workflow_run_ref)?,
        })
    }

    pub(super) fn workflow_create_watchdog(
        &self,
        request: crate::local::CreateWorkflowWatchdogRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let watchdog = self.session_store.write().create_workflow_watchdog(
            &request.session_id,
            &request.workflow_ref,
            &request.endpoint_ref,
            request.interval_seconds,
            request.invocation_prompt,
            request.policy,
            if request.max_wakeups_configured {
                Some(request.max_wakeups)
            } else {
                None
            },
        )?;
        let workflow = self
            .session_store
            .read()
            .resolve_workflow_ref(&request.session_id, &request.workflow_ref)?;
        let endpoint = self.session_store.read().resolve_workflow_endpoint_ref(
            &request.session_id,
            &request.workflow_ref,
            &request.endpoint_ref,
        )?;
        let session = self.workflow_session(&request.session_id)?;
        Ok(LocalDaemonResponse::WorkflowWatchdogCreated {
            watchdog,
            workflow,
            endpoint,
            session,
        })
    }

    pub(super) fn workflow_list_watchdogs(
        &self,
        request: crate::local::ListWorkflowWatchdogsRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        Ok(LocalDaemonResponse::WorkflowWatchdogsListed {
            watchdogs: self
                .session_store
                .read()
                .list_workflow_watchdogs(&request.session_id, request.workflow_ref.as_deref())?,
        })
    }

    pub(super) fn workflow_set_watchdog_enabled(
        &self,
        request: crate::local::SetWorkflowWatchdogEnabledRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let watchdog = self.session_store.write().set_workflow_watchdog_enabled(
            &request.session_id,
            &request.watchdog_ref,
            request.enabled,
        )?;
        let session = self.workflow_session(&request.session_id)?;
        Ok(LocalDaemonResponse::WorkflowWatchdogUpdated { watchdog, session })
    }

    pub(super) fn workflow_remove_watchdog(
        &self,
        request: crate::local::RemoveWorkflowWatchdogRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let watchdog = self
            .session_store
            .write()
            .remove_workflow_watchdog(&request.session_id, &request.watchdog_ref)?;
        let session = self.workflow_session(&request.session_id)?;
        Ok(LocalDaemonResponse::WorkflowWatchdogRemoved { watchdog, session })
    }

    pub(super) fn workflow_list_queued_launches(
        &self,
        request: crate::local::ListQueuedWorkflowLaunchesRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        Ok(LocalDaemonResponse::QueuedWorkflowLaunchesListed {
            queued_launches: self
                .session_store
                .read()
                .list_queued_workflow_launches(&request.session_id)?,
        })
    }

    pub(super) fn workflow_remove_queued_launch(
        &self,
        request: crate::local::RemoveQueuedWorkflowLaunchRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let queued_launch = self
            .session_store
            .write()
            .remove_queued_workflow_launch(&request.session_id, &request.queue_item_ref)?;
        let session = self.workflow_session(&request.session_id)?;
        Ok(LocalDaemonResponse::QueuedWorkflowLaunchRemoved {
            queued_launch,
            session,
        })
    }

    pub(super) fn workflow_clear_queued_launches(
        &self,
        request: crate::local::ClearQueuedWorkflowLaunchesRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let queued_launches = self
            .session_store
            .write()
            .clear_queued_workflow_launches(&request.session_id)?;
        let session = self.workflow_session(&request.session_id)?;
        Ok(LocalDaemonResponse::QueuedWorkflowLaunchesCleared {
            queued_launches,
            session,
        })
    }

    pub(super) fn workflow_validate_output(
        &self,
        request: crate::local::ValidateWorkflowOutputRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let warning = crate::transport::runtime_tools::validate_workflow_output_schema(
            &request.output_schema_ref,
            &request.output_json,
        )
        .err();
        Ok(LocalDaemonResponse::WorkflowOutputValidated {
            valid: warning.is_none(),
            warning,
        })
    }

    pub(super) fn workflow_ack_turn(
        &self,
        request: crate::local::AckWorkflowTurnRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let workflow_run_id = self
            .session_store
            .read()
            .resolve_workflow_run_ref(&request.session_id, &request.workflow_run_ref)?
            .id()
            .to_string();
        let workflow_run = self.session_store.write().ack_workflow_turn(
            &request.session_id,
            &workflow_run_id,
            &request.workflow_node_run_id,
            &request.delivery_token,
        )?;
        let event = crate::session::WorkflowRuntimeToolCallEvent::new(
            crate::transport::runtime_tools::ACK_WORKFLOW_TURN_TOOL.to_string(),
            serde_json::json!({"delivery_token": request.delivery_token}).to_string(),
            Some(
                serde_json::json!({
                    "workflow_run_id": workflow_run.id(),
                    "workflow_node_run_id": request.workflow_node_run_id,
                    "state": "acknowledged",
                    "next_action": "Continue this same workflow turn. This acknowledgement is not the final answer. If this turn requires final workflow run output, call validate_and_submit_workflow_run_output before stopping; otherwise emit the required final fenced json block before stopping.",
                })
                .to_string(),
            ),
            true,
        );
        let _ = self
            .session_store
            .write()
            .record_workflow_runtime_tool_call(
                &request.session_id,
                &request.workflow_node_run_id,
                event,
            );
        let workflow_run = self
            .session_store
            .read()
            .resolve_workflow_run_ref(&request.session_id, &workflow_run_id)?;
        let session = self.workflow_session(&request.session_id)?;
        Ok(LocalDaemonResponse::WorkflowTurnAcknowledged {
            workflow_run,
            session,
        })
    }
}
