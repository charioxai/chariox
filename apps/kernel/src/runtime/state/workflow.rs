use super::*;

impl KernelRuntimeOwnedState {
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

    pub(super) fn workflow_alias_workflow(
        &self,
        request: crate::local::AliasWorkflowRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
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
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let endpoint = self.session_store.write().create_workflow_endpoint(
            &request.session_id,
            &request.workflow_ref,
            &request.entry_node_id,
            request.alias,
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
    ) -> Result<LocalDaemonResponse, DaemonError> {
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
    ) -> Result<LocalDaemonResponse, DaemonError> {
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
    ) -> Result<LocalDaemonResponse, DaemonError> {
        if self
            .agent_store
            .get_session_agents(&request.session_id)
            .into_iter()
            .all(|agent| agent.id() != request.agent_id)
        {
            return Err(DaemonError::AgentNotFound {
                agent_id: request.agent_id,
            });
        }
        let node = self.session_store.write().add_workflow_node(
            &request.session_id,
            &request.workflow_ref,
            &request.agent_id,
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
    ) -> Result<LocalDaemonResponse, DaemonError> {
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
    ) -> Result<LocalDaemonResponse, DaemonError> {
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
    ) -> Result<LocalDaemonResponse, DaemonError> {
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
    ) -> Result<LocalDaemonResponse, DaemonError> {
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
    ) -> Result<LocalDaemonResponse, DaemonError> {
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
    ) -> Result<LocalDaemonResponse, DaemonError> {
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
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let edge = self.session_store.write().add_workflow_edge(
            &request.session_id,
            &request.workflow_ref,
            &request.from_node_id,
            &request.to_node_id,
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
    ) -> Result<LocalDaemonResponse, DaemonError> {
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

    pub(super) fn workflow_set_flush_context(
        &self,
        request: crate::local::SetWorkflowFlushContextRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
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

    pub(super) fn workflow_start_prompt(
        &self,
        session_id: &str,
        prompt: &crate::session::PromptQueueItem,
    ) -> Result<(), DaemonError> {
        let (Some(workflow_run_id), Some(workflow_node_run_id)) =
            (prompt.workflow_run_id(), prompt.workflow_node_run_id())
        else {
            return Ok(());
        };
        let workflow_run = self.session_store.write().start_workflow_node_run(
            session_id,
            workflow_run_id,
            workflow_node_run_id,
        )?;
        let recipients = self
            .attachment_store
            .list_session_attachment_ids(session_id);
        let active_provider_run_id = self
            .session_store
            .get_session(session_id)?
            .active_provider_run_id()
            .map(str::to_string);
        self.record_notice(
            session_id,
            active_provider_run_id.as_deref(),
            recipients,
            format!(
                "Workflow run `{}` started on agent `{}`.",
                workflow_run.id(),
                prompt.target_agent_id()
            ),
        );
        let _ = self.session_snapshot(session_id)?;
        Ok(())
    }

    pub(super) fn workflow_ensure_provider_run(
        &self,
        session_id: &str,
        agent_id: &str,
    ) -> Result<String, DaemonError> {
        if let Some(run) = self.provider_store.get_run_for_agent(session_id, agent_id) {
            if run.state() == crate::provider::ProviderRunState::Parked {
                let resumed = self.resume_provider_run_for_session(session_id, run.id())?;
                self.session_store
                    .set_active_provider_run(session_id, Some(resumed.id().to_string()))?;
                return Ok(resumed.id().to_string());
            }
            if run.state() != crate::provider::ProviderRunState::Ended {
                self.session_store
                    .set_active_provider_run(session_id, Some(run.id().to_string()))?;
                return Ok(run.id().to_string());
            }
        }
        let agent = self.agent_store.get_agent(agent_id)?;
        let adapter_key = match agent.provider() {
            "default" => "opencode",
            value => value,
        };
        let provider = match agent.provider() {
            "default" => "opencode",
            value => value,
        };
        let mut request = crate::provider::LaunchProviderRequest::new(
            session_id,
            adapter_key,
            provider,
            "default",
            agent.model().unwrap_or("default"),
        )
        .with_agent_id(agent.id().to_string())
        .with_variant(agent.effort().map(str::to_string));
        if crate::provider::provider_requires_managed_io_by_default(provider) {
            request = request.with_managed_io_required();
        }
        if let Some(worktree_id) = agent.worktree_id() {
            request = request.with_working_directory(std::path::PathBuf::from(worktree_id));
        }
        let run = self.provider_store.launch_run_detached(request)?;
        self.session_store
            .set_active_provider_run(session_id, Some(run.id().to_string()))?;
        self.provider_run_projection.update(run.clone());
        Ok(run.id().to_string())
    }

    pub(super) fn workflow_dispatch_claim_id(
        &self,
        session_id: &str,
        agent_id: &str,
    ) -> Result<String, DaemonError> {
        let agent = self.agent_store.get_agent(agent_id)?;
        if let Some(remote_execution) = agent.remote_execution() {
            return Ok(format!(
                "remote-workflow:{}:{}",
                remote_execution.worker_kernel_id, remote_execution.leased_agent_id
            ));
        }
        self.workflow_ensure_provider_run(session_id, agent_id)
    }

    pub(super) fn workflow_submit_prepared_prompt(
        &self,
        prepared: crate::app::KernelPreparedPromptSubmission,
        workflow_run_id: &str,
        workflow_node_run_id: &str,
    ) -> Result<WorkflowPromptDispatches, DaemonError> {
        let mut dispatches = WorkflowPromptDispatches::default();
        let mut submission = match self.submit_local_prepared_prompt(&prepared)? {
            Some(submission) => submission,
            None => match self.submit_remote_prepared_prompt(&prepared)? {
                Some(submission) => submission,
                None => return Ok(dispatches),
            },
        };
        if let crate::session::PromptSubmissionOutcome::Started { prompt } = &submission.outcome {
            let _ = self.session_store.write().mark_workflow_turn_dispatched(
                &prepared.session_id,
                workflow_run_id,
                workflow_node_run_id,
            );
            let _ = self.workflow_start_prompt(&prepared.session_id, prompt);
        }
        if let Some(dispatch) = submission.dispatch.take() {
            dispatches.local.push(dispatch);
        }
        if let Some(mut dispatch) = submission.remote_dispatch.take() {
            if dispatch.workflow_context.is_none() {
                let prompt = match &submission.outcome {
                    crate::session::PromptSubmissionOutcome::Started { prompt }
                    | crate::session::PromptSubmissionOutcome::Queued { prompt } => prompt,
                };
                dispatch.workflow_context = Some(self.remote_workflow_turn_context_for_prompt(
                    &prepared.session_id,
                    prompt.target_agent_id(),
                    prompt,
                )?);
            }
            dispatches.remote.push(dispatch);
        }
        Ok(dispatches)
    }

    pub(super) fn remote_workflow_turn_context_for_prompt(
        &self,
        session_id: &str,
        target_agent_id: &str,
        prompt: &crate::session::PromptQueueItem,
    ) -> Result<crate::execution_lease::RemoteWorkflowTurnContext, DaemonError> {
        let workflow_run_id =
            prompt
                .workflow_run_id()
                .ok_or_else(|| DaemonError::LocalTransport {
                    operation: "dispatch remote workflow prompt",
                    message: "remote workflow prompt is missing workflow run id".to_string(),
                })?;
        let workflow_node_run_id =
            prompt
                .workflow_node_run_id()
                .ok_or_else(|| DaemonError::LocalTransport {
                    operation: "dispatch remote workflow prompt",
                    message: "remote workflow prompt is missing workflow node run id".to_string(),
                })?;
        let workflow_run = self
            .session_store
            .read()
            .resolve_workflow_run_ref(session_id, workflow_run_id)?;
        let delivery_token = workflow_run
            .node_runs()
            .iter()
            .find(|node_run| node_run.id() == workflow_node_run_id)
            .and_then(|node_run| node_run.turn_envelope())
            .map(|envelope| envelope.delivery_token().to_string())
            .ok_or_else(|| DaemonError::LocalTransport {
                operation: "dispatch remote workflow prompt",
                message: format!(
                    "workflow node run `{workflow_node_run_id}` has no prepared turn envelope"
                ),
            })?;
        Ok(crate::execution_lease::RemoteWorkflowTurnContext {
            home_kernel_id: self.config_projection.snapshot().daemon_id,
            home_session_id: session_id.to_string(),
            home_agent_id: target_agent_id.to_string(),
            workflow_run_id: workflow_run.id().to_string(),
            workflow_node_run_id: workflow_node_run_id.to_string(),
            delivery_token,
        })
    }

    pub(super) fn workflow_validate_agents(
        &self,
        session_id: &str,
        workflow: &crate::session::WorkflowDefinition,
    ) -> Result<(), DaemonError> {
        let agents = self.agent_store.get_session_agents(session_id);
        let agent_ids = agents
            .iter()
            .map(|agent| agent.id().to_string())
            .collect::<std::collections::BTreeSet<_>>();
        for node in workflow.nodes() {
            if !agent_ids.contains(node.agent_id()) {
                return Err(DaemonError::WorkflowNodeAgentMissing {
                    session_id: session_id.to_string(),
                    workflow_id: workflow.id().to_string(),
                    node_id: node.id().to_string(),
                    agent_id: node.agent_id().to_string(),
                });
            }
            let Some(agent) = agents.iter().find(|agent| agent.id() == node.agent_id()) else {
                continue;
            };
            let capabilities = self
                .provider_store
                .get_run_for_agent(session_id, node.agent_id())
                .unwrap_or_else(|| {
                    crate::provider::RuntimeProviderRun::from_control_capability_inference(
                        format!("inferred-{session_id}-{}", node.agent_id()),
                        session_id.to_string(),
                        Some(node.agent_id().to_string()),
                        agent.provider().to_string(),
                    )
                });
            if !capabilities
                .supports_control_operation(crate::provider::ControlOperation::AckWorkflowTurn)
            {
                return Err(DaemonError::WorkflowNodeControlUnsupported {
                    session_id: session_id.to_string(),
                    workflow_id: workflow.id().to_string(),
                    node_id: node.id().to_string(),
                    agent_id: node.agent_id().to_string(),
                    operation: "ack_workflow_turn",
                });
            }
            let requires_validation = workflow
                .edges()
                .iter()
                .any(|edge| edge.from_node_id() == node.id() && edge.output_schema_ref().is_some());
            if requires_validation
                && !capabilities.supports_control_operation(
                    crate::provider::ControlOperation::ValidateWorkflowOutput,
                )
            {
                return Err(DaemonError::WorkflowNodeControlUnsupported {
                    session_id: session_id.to_string(),
                    workflow_id: workflow.id().to_string(),
                    node_id: node.id().to_string(),
                    agent_id: node.agent_id().to_string(),
                    operation: "validate_workflow_output",
                });
            }
        }
        Ok(())
    }

    pub(super) fn workflow_flush_agent_context_if_needed(
        &self,
        session_id: &str,
        workflow: &crate::session::WorkflowDefinition,
    ) -> Result<(), DaemonError> {
        if !workflow.flush_agent_context_before_run() {
            return Ok(());
        }
        let workflow_agent_ids = workflow
            .nodes()
            .iter()
            .map(|node| node.agent_id().to_string())
            .collect::<std::collections::BTreeSet<_>>();
        if workflow_agent_ids.is_empty() {
            return Ok(());
        }
        let session = self.session_store.get_session(session_id)?;
        for agent_id in &workflow_agent_ids {
            if self
                .prompt_state_owner
                .active_prompt_for_agent(&session, agent_id)
                .is_some()
            {
                let _ = self.cancel_active_prompt_only(session_id, agent_id);
            }
        }
        for agent_id in workflow_agent_ids {
            let Some(run) = self.provider_store.get_run_for_agent(session_id, &agent_id) else {
                continue;
            };
            if run.state() == crate::provider::ProviderRunState::Ended {
                continue;
            }
            let ended = self
                .provider_store
                .terminate_run_provider_only(session_id, run.id())?
                .into_run();
            if self
                .session_store
                .get_session(session_id)?
                .active_provider_run_id()
                == Some(ended.id())
            {
                self.session_store
                    .set_active_provider_run(session_id, None)?;
            }
            self.provider_run_projection.update(ended.clone());
            self.remove_provider_process_tracking_for_run(ended.id(), None);
        }
        Ok(())
    }

    pub(super) fn workflow_schedule_entry_node(
        &self,
        session_id: &str,
        workflow_run: &crate::session::WorkflowRun,
    ) -> Result<WorkflowPromptDispatches, DaemonError> {
        let endpoint_prompt = workflow_run
            .invocation_prompt()
            .map(str::trim)
            .unwrap_or("");
        let node_run = workflow_run.node_runs().first().ok_or_else(|| {
            DaemonError::InvalidWorkflowGraphReference {
                session_id: session_id.to_string(),
                workflow_id: workflow_run.workflow_id().to_string(),
                reference: workflow_run.id().to_string(),
                message: "workflow run has no entry node run",
            }
        })?;
        let prompt_text = self.workflow_turn_prompt_text(
            session_id,
            workflow_run.id(),
            node_run.id(),
            node_run.node_id(),
            endpoint_prompt,
            None,
            None,
        )?;
        let _ = self.session_store.write().prepare_workflow_turn(
            session_id,
            workflow_run.id(),
            node_run.id(),
            format!("workflow-ack:{}", node_run.id()),
            prompt_text.clone(),
            None,
            None,
        )?;
        let claim_id = self.workflow_dispatch_claim_id(session_id, node_run.agent_id())?;
        match self.acquire_workflow_node_workspace_claim(
            session_id,
            &claim_id,
            node_run.agent_id(),
            workflow_run.id(),
            node_run.id(),
        ) {
            Ok(()) => {}
            Err(error @ DaemonError::WorkspaceClaimConflict { .. }) => {
                let _ = self
                    .session_store
                    .write()
                    .block_workflow_node_on_workspace_claim(
                        session_id,
                        workflow_run.id(),
                        node_run.id(),
                    );
                self.record_notice(
                    session_id,
                    None,
                    self.attachment_store
                        .list_session_attachment_ids(session_id),
                    format!(
                        "Workflow run `{}` blocked node `{}` on a workspace claim: {error}",
                        workflow_run.id(),
                        node_run.node_id()
                    ),
                );
                let _ = self.session_snapshot(session_id)?;
                return Ok(WorkflowPromptDispatches::default());
            }
            Err(error) => return Err(error),
        }
        let _ = self
            .session_store
            .write()
            .ready_workflow_node_after_workspace_claim(
                session_id,
                workflow_run.id(),
                node_run.id(),
            );
        let prompt = crate::session::PromptQueueItem::new(
            self.session_store.reserve_prompt_id(),
            crate::scheduler::runtime::workflow_prompt_source_attachment_id(workflow_run.id()),
            node_run.agent_id(),
            prompt_text,
            crate::session::PromptStatus::Queued,
        )
        .with_workflow_context(workflow_run.id(), node_run.id());
        self.workflow_submit_prepared_prompt(
            crate::app::KernelPreparedPromptSubmission {
                session_id: session_id.to_string(),
                prompt,
                force_queue: false,
            },
            workflow_run.id(),
            node_run.id(),
        )
    }

    pub(super) fn workflow_invoke_queued_launch(
        &self,
        session_id: &str,
        queued_launch: crate::session::QueuedWorkflowLaunch,
    ) -> Result<
        (
            crate::app::workflow_runtime::WorkflowLaunchOutcome,
            WorkflowPromptDispatches,
        ),
        DaemonError,
    > {
        let workflow = self
            .session_store
            .read()
            .resolve_workflow_ref(session_id, queued_launch.workflow_id())?;
        let endpoint = self.session_store.read().resolve_workflow_endpoint_ref(
            session_id,
            queued_launch.workflow_id(),
            queued_launch.endpoint_id(),
        )?;
        self.workflow_validate_agents(session_id, &workflow)?;
        self.workflow_flush_agent_context_if_needed(session_id, &workflow)?;
        let workflow_run = self.session_store.write().invoke_workflow_endpoint(
            session_id,
            workflow.id(),
            endpoint.id(),
            queued_launch.invocation_prompt().map(str::to_string),
        )?;
        let dispatches = self.workflow_schedule_entry_node(session_id, &workflow_run)?;
        let workflow_run = self
            .session_store
            .read()
            .resolve_workflow_run_ref(session_id, workflow_run.id())?;
        if let Some(watchdog_id) = queued_launch.watchdog_id() {
            let _ = self.session_store.write().mark_workflow_watchdog_invoked(
                session_id,
                watchdog_id,
                workflow_run.id(),
            );
        }
        Ok((
            crate::app::workflow_runtime::WorkflowLaunchOutcome::Started {
                workflow_run,
                workflow,
                endpoint,
            },
            dispatches,
        ))
    }

    pub(super) fn workflow_invoke_endpoint_with_admission(
        &self,
        session_id: &str,
        workflow_ref: &str,
        endpoint_ref: &str,
        prompt: Option<String>,
    ) -> Result<
        (
            crate::app::workflow_runtime::WorkflowLaunchOutcome,
            WorkflowPromptDispatches,
        ),
        DaemonError,
    > {
        let workflow = self
            .session_store
            .read()
            .resolve_workflow_ref(session_id, workflow_ref)?;
        let endpoint = self.session_store.read().resolve_workflow_endpoint_ref(
            session_id,
            workflow_ref,
            endpoint_ref,
        )?;
        self.workflow_validate_agents(session_id, &workflow)?;
        let admission = self.session_store.write().admit_manual_workflow_launch(
            session_id,
            workflow.id(),
            endpoint.id(),
            prompt.clone(),
        )?;
        match admission {
            crate::session::WorkflowLaunchAdmission::StartNow => {
                self.workflow_flush_agent_context_if_needed(session_id, &workflow)?;
                let workflow_run = self.session_store.write().invoke_workflow_endpoint(
                    session_id,
                    workflow.id(),
                    endpoint.id(),
                    prompt,
                )?;
                let dispatches = self.workflow_schedule_entry_node(session_id, &workflow_run)?;
                let workflow_run = self
                    .session_store
                    .read()
                    .resolve_workflow_run_ref(session_id, workflow_run.id())?;
                Ok((
                    crate::app::workflow_runtime::WorkflowLaunchOutcome::Started {
                        workflow_run,
                        workflow,
                        endpoint,
                    },
                    dispatches,
                ))
            }
            crate::session::WorkflowLaunchAdmission::Queued(queued_launch) => Ok((
                crate::app::workflow_runtime::WorkflowLaunchOutcome::Queued {
                    queued_launch,
                    workflow,
                    endpoint,
                },
                WorkflowPromptDispatches::default(),
            )),
        }
    }

    pub(super) fn workflow_cancel_prompt(
        &self,
        session_id: &str,
        prompt: &crate::session::PromptQueueItem,
    ) -> Result<(), DaemonError> {
        let (Some(workflow_run_id), Some(workflow_node_run_id)) =
            (prompt.workflow_run_id(), prompt.workflow_node_run_id())
        else {
            return Ok(());
        };
        let workflow_run = self.session_store.write().stop_workflow_node_run(
            session_id,
            workflow_run_id,
            workflow_node_run_id,
        )?;
        let _ = self.release_workflow_node_workspace_claim(
            session_id,
            workflow_run_id,
            workflow_node_run_id,
        );
        self.workflow_record_failure(
            session_id,
            workflow_run_id,
            &crate::session::WorkflowFailureEvent::new(
                crate::session::WorkflowFailureKind::RunStopped,
                workflow_node_run_id,
                Vec::new(),
                "workflow node run was stopped before validated completion",
            ),
        );
        self.record_notice(
            session_id,
            None,
            self.attachment_store
                .list_session_attachment_ids(session_id),
            format!("Workflow run `{}` was stopped.", workflow_run.id()),
        );
        self.workflow_maybe_start_next_queued_launch(session_id);
        let _ = self.session_snapshot(session_id)?;
        Ok(())
    }

    #[allow(dead_code)]
    pub(super) fn workflow_complete_prompt(
        &self,
        session_id: &str,
        prompt: &crate::session::PromptQueueItem,
        provider_run_id: Option<&str>,
    ) -> Result<WorkflowPromptDispatches, DaemonError> {
        let (Some(workflow_run_id), Some(workflow_node_run_id)) =
            (prompt.workflow_run_id(), prompt.workflow_node_run_id())
        else {
            return Ok(WorkflowPromptDispatches::default());
        };
        let completion_snapshot = self.workflow_completion_snapshot(
            session_id,
            workflow_run_id,
            workflow_node_run_id,
            provider_run_id,
        );
        let max_turns = self.workflow_max_turns(session_id);
        let completion_result = self.session_store.write().complete_workflow_node_run(
            session_id,
            workflow_run_id,
            workflow_node_run_id,
            completion_snapshot.clone(),
            max_turns,
        );
        let update = match completion_result {
            Ok(update) => update,
            Err(crate::error::DaemonError::WorkflowOutputValidationFailed {
                edge_id,
                message,
                ..
            }) => {
                self.workflow_record_failure(
                    session_id,
                    workflow_run_id,
                    &crate::session::WorkflowFailureEvent::new(
                        crate::session::WorkflowFailureKind::OutputValidationFailed,
                        workflow_node_run_id,
                        vec![edge_id.clone()],
                        message.clone(),
                    ),
                );
                self.session_store.write().stop_workflow_node_run(
                    session_id,
                    workflow_run_id,
                    workflow_node_run_id,
                )?;
                let _ = self.release_workflow_node_workspace_claim(
                    session_id,
                    workflow_run_id,
                    workflow_node_run_id,
                );
                self.record_notice(
                    session_id,
                    None,
                    self.attachment_store.list_session_attachment_ids(session_id),
                    format!(
                        "Workflow run `{workflow_run_id}` stopped after validation failed on edge `{edge_id}`: {message}"
                    ),
                );
                self.workflow_maybe_start_next_queued_launch(session_id);
                let _ = self.session_snapshot(session_id)?;
                return Ok(WorkflowPromptDispatches::default());
            }
            Err(error) => return Err(error),
        };
        for warning in &update.validation_warnings {
            let failure = crate::session::WorkflowFailureEvent::new(
                crate::session::classify_workflow_failure_kind(
                    &completion_snapshot,
                    &warning.message,
                ),
                workflow_node_run_id,
                vec![warning.edge_id.clone()],
                warning.message.clone(),
            );
            self.workflow_record_failure(session_id, workflow_run_id, &failure);
            self.record_notice(
                session_id,
                None,
                self.attachment_store
                    .list_session_attachment_ids(session_id),
                format!(
                    "Workflow output validation warning on edge `{}`: {}",
                    warning.edge_id, warning.message
                ),
            );
        }
        if update.workflow_run.status() == crate::session::WorkflowRunStatus::Stopped
            && update.workflow_run.final_output().is_none()
            && update.workflow_run.failure_events().iter().all(|event| {
                event.kind() != crate::session::WorkflowFailureKind::NodeTurnBudgetExhausted
            })
        {
            self.workflow_record_failure(
                session_id,
                workflow_run_id,
                &crate::session::WorkflowFailureEvent::new(
                    crate::session::WorkflowFailureKind::NodeTurnBudgetExhausted,
                    workflow_node_run_id,
                    Vec::new(),
                    "workflow run stopped after a node exhausted its turn budget",
                ),
            );
        }
        if update.workflow_run.final_output_valid() == Some(false) {
            self.workflow_record_failure(
                session_id,
                workflow_run_id,
                &crate::session::WorkflowFailureEvent::new(
                    crate::session::WorkflowFailureKind::WorkflowRunOutputValidationFailed,
                    workflow_node_run_id,
                    Vec::new(),
                    update
                        .workflow_run
                        .final_output_warning()
                        .unwrap_or("workflow run output validation failed"),
                ),
            );
        }
        if update.validation_warnings.is_empty() {
            let _ = self
                .session_store
                .write()
                .mark_workflow_turn_validated_completed(
                    session_id,
                    workflow_run_id,
                    workflow_node_run_id,
                )?;
        }
        let claim_provider_run_id = provider_run_id.map(str::to_string).or_else(|| {
            self.provider_store
                .get_run_for_agent(session_id, prompt.target_agent_id())
                .map(|run| run.id().to_string())
        });
        let released_claim = claim_provider_run_id
            .as_deref()
            .map(|provider_run_id| self.clear_prompt_activity(provider_run_id))
            .unwrap_or(false);
        let released_workflow_claim = self.release_workflow_node_workspace_claim(
            session_id,
            workflow_run_id,
            workflow_node_run_id,
        );
        let mut dispatches =
            self.workflow_prepare_dispatches(session_id, workflow_run_id, &update.dispatches);
        if released_claim || released_workflow_claim {
            dispatches.extend(self.workflow_retry_blocked_claims());
        }
        let state_suffix = match update.workflow_run.status() {
            crate::session::WorkflowRunStatus::Waiting => "waiting for downstream handoffs",
            crate::session::WorkflowRunStatus::Completing => "is completing",
            crate::session::WorkflowRunStatus::Completed => "completed",
            crate::session::WorkflowRunStatus::Stopped => "stopped",
            _ => "updated",
        };
        self.record_notice(
            session_id,
            None,
            self.attachment_store
                .list_session_attachment_ids(session_id),
            format!(
                "Workflow run `{}` {state_suffix}.",
                update.workflow_run.id()
            ),
        );
        if matches!(
            update.workflow_run.status(),
            crate::session::WorkflowRunStatus::Completed
                | crate::session::WorkflowRunStatus::Failed
                | crate::session::WorkflowRunStatus::Stopped
        ) {
            self.workflow_maybe_start_next_queued_launch(session_id);
        }
        let _ = self.session_snapshot(session_id)?;
        Ok(dispatches)
    }

    #[allow(dead_code)]
    pub(super) fn workflow_completion_snapshot(
        &self,
        session_id: &str,
        workflow_run_id: &str,
        workflow_node_run_id: &str,
        provider_run_id: Option<&str>,
    ) -> Option<crate::session::WorkflowCompletionSnapshot> {
        let provider_run_id = provider_run_id
            .map(str::trim)
            .filter(|value| !value.is_empty())?;
        let session = self.session_store.get_session(session_id).ok()?;
        let history = match self.history_store.load(&session) {
            Ok(history) => history,
            Err(error) => {
                crate::logging::warn_with_fields(
                    "daemon.workflow",
                    "failed to load session history for workflow completion snapshot",
                    serde_json::json!({
                        "session_id": session_id,
                        "workflow_run_id": workflow_run_id,
                        "workflow_node_run_id": workflow_node_run_id,
                        "provider_run_id": provider_run_id,
                        "error": error.to_string(),
                    }),
                );
                return None;
            }
        };
        self.history_projection
            .update_entries(session_id, history.clone());
        crate::scheduler::runtime::build_workflow_completion_snapshot_from_history(
            &session,
            history,
            session_id,
            workflow_run_id,
            workflow_node_run_id,
            provider_run_id,
        )
    }

    pub(super) fn workflow_prompt_has_completion_output(
        &self,
        session_id: &str,
        workflow_run_id: &str,
        workflow_node_run_id: &str,
        provider_run_id: &str,
    ) -> bool {
        self.workflow_completion_snapshot(
            session_id,
            workflow_run_id,
            workflow_node_run_id,
            Some(provider_run_id),
        )
        .and_then(|snapshot| snapshot.output().cloned())
        .is_some()
    }

    #[allow(dead_code)]
    pub(super) fn workflow_max_turns(&self, session_id: &str) -> Option<usize> {
        self.session_store
            .get_session(session_id)
            .ok()
            .and_then(|session| {
                session
                    .config_state()
                    .values()
                    .get("workflow.max_turns")
                    .and_then(|value| value.trim().parse::<usize>().ok())
                    .filter(|value| *value > 0)
            })
            .or(Some(
                crate::session::DEFAULT_WORKFLOW_RUN_MAX_TURNS_SAFETY_LIMIT,
            ))
    }

    pub(super) fn workflow_record_failure(
        &self,
        session_id: &str,
        workflow_run_id: &str,
        failure: &crate::session::WorkflowFailureEvent,
    ) {
        let _ = self.session_store.write().record_workflow_failure_event(
            session_id,
            workflow_run_id,
            failure.clone(),
        );
    }

    #[allow(dead_code)]
    pub(super) fn workflow_control_mailbox_text(
        &self,
        session_id: &str,
        workflow_run_id: &str,
        _workflow_node_run_id: &str,
    ) -> Option<String> {
        let workflow_run = self
            .session_store
            .read()
            .resolve_workflow_run_ref(session_id, workflow_run_id)
            .ok()?;
        let lines = workflow_run
            .failure_events()
            .iter()
            .map(|failure| format!("- {:?}: {}", failure.kind(), failure.message()))
            .collect::<Vec<_>>();
        (!lines.is_empty()).then(|| lines.join("\n"))
    }

    #[allow(dead_code)]
    pub(super) fn workflow_outgoing_edge_contracts_text(
        &self,
        session_id: &str,
        workflow_run_id: &str,
        node_id: &str,
    ) -> String {
        let workflow_id = match self
            .session_store
            .read()
            .resolve_workflow_run_ref(session_id, workflow_run_id)
        {
            Ok(run) => run.workflow_id().to_string(),
            Err(_) => return String::new(),
        };
        let Ok(workflow) = self
            .session_store
            .read()
            .resolve_workflow_ref(session_id, &workflow_id)
        else {
            return String::new();
        };
        let lines = workflow
            .edges()
            .iter()
            .filter(|edge| edge.from_node_id() == node_id)
            .map(|edge| {
                let mut line = format!("- edge {} -> {}", edge.id(), edge.to_node_id());
                if let Some(schema_ref) = edge.output_schema_ref() {
                    line.push_str(&format!(", output_schema_ref: {schema_ref}"));
                }
                if let Some(validation_policy) = edge.validation_policy() {
                    line.push_str(&format!(", validation_policy: {validation_policy:?}"));
                }
                line
            })
            .collect::<Vec<_>>();
        if lines.is_empty() {
            String::new()
        } else {
            format!("Outgoing edge contracts:\n{}\n\n", lines.join("\n"))
        }
    }

    #[allow(dead_code)]
    pub(super) fn workflow_prepare_dispatches(
        &self,
        session_id: &str,
        workflow_run_id: &str,
        dispatches: &[crate::session::WorkflowDispatch],
    ) -> WorkflowPromptDispatches {
        let mut prepared = WorkflowPromptDispatches::default();
        for dispatch in dispatches {
            if !self.workflow_dispatch_has_all_inputs(session_id, workflow_run_id, &dispatch) {
                continue;
            }
            self.record_notice(
                session_id,
                None,
                self.attachment_store
                    .list_session_attachment_ids(session_id),
                format!(
                    "Workflow run `{workflow_run_id}` routed {} upstream message(s) to node `{}`.",
                    dispatch.messages.len(),
                    dispatch.node_run.node_id()
                ),
            );
            let handoff_payloads_json =
                serde_json::to_string(&dispatch.messages).unwrap_or_else(|_| "[]".to_string());
            let control_mailbox = self.workflow_control_mailbox_text(
                session_id,
                workflow_run_id,
                dispatch.node_run.id(),
            );
            let prompt_text = match self.workflow_turn_prompt_text(
                session_id,
                workflow_run_id,
                dispatch.node_run.id(),
                dispatch.node_run.node_id(),
                "",
                Some(&handoff_payloads_json),
                control_mailbox.as_deref(),
            ) {
                Ok(prompt_text) => prompt_text,
                Err(error) => {
                    self.record_notice(
                        session_id,
                        None,
                        self.attachment_store.list_session_attachment_ids(session_id),
                        format!(
                            "Workflow run `{workflow_run_id}` could not prepare downstream node `{}`: {}",
                            dispatch.node_run.node_id(),
                            error
                        ),
                    );
                    continue;
                }
            };
            let _ = self.session_store.write().prepare_workflow_turn(
                session_id,
                workflow_run_id,
                dispatch.node_run.id(),
                format!("workflow-ack:{}", dispatch.node_run.id()),
                prompt_text.clone(),
                control_mailbox,
                Some(handoff_payloads_json),
            );
            let claim_id = match self
                .workflow_dispatch_claim_id(session_id, dispatch.node_run.agent_id())
            {
                Ok(claim_id) => claim_id,
                Err(error) => {
                    self.record_notice(
                            session_id,
                            None,
                            self.attachment_store.list_session_attachment_ids(session_id),
                            format!(
                                "Workflow run `{workflow_run_id}` could not schedule downstream node `{}`: {}",
                                dispatch.node_run.node_id(),
                                error
                            ),
                        );
                    continue;
                }
            };
            match self.acquire_workflow_node_workspace_claim(
                session_id,
                &claim_id,
                dispatch.node_run.agent_id(),
                workflow_run_id,
                dispatch.node_run.id(),
            ) {
                Ok(()) => {
                    let _ = self
                        .session_store
                        .write()
                        .ready_workflow_node_after_workspace_claim(
                            session_id,
                            workflow_run_id,
                            dispatch.node_run.id(),
                        );
                }
                Err(error @ DaemonError::WorkspaceClaimConflict { .. }) => {
                    let _ = self
                        .session_store
                        .write()
                        .block_workflow_node_on_workspace_claim(
                            session_id,
                            workflow_run_id,
                            dispatch.node_run.id(),
                        );
                    self.record_notice(
                        session_id,
                        None,
                        self.attachment_store.list_session_attachment_ids(session_id),
                        format!(
                            "Workflow run `{workflow_run_id}` blocked node `{}` on a workspace claim: {error}",
                            dispatch.node_run.node_id()
                        ),
                    );
                    continue;
                }
                Err(error) => {
                    self.record_notice(
                        session_id,
                        None,
                        self.attachment_store.list_session_attachment_ids(session_id),
                        format!(
                            "Workflow run `{workflow_run_id}` could not schedule downstream node `{}`: {}",
                            dispatch.node_run.node_id(),
                            error
                        ),
                    );
                    continue;
                }
            }
            let prompt = crate::session::PromptQueueItem::new(
                self.session_store.reserve_prompt_id(),
                crate::scheduler::runtime::workflow_prompt_source_attachment_id(workflow_run_id),
                dispatch.node_run.agent_id(),
                prompt_text,
                crate::session::PromptStatus::Queued,
            )
            .with_workflow_context(workflow_run_id, dispatch.node_run.id());
            match self.workflow_submit_prepared_prompt(
                crate::app::KernelPreparedPromptSubmission {
                    session_id: session_id.to_string(),
                    prompt,
                    force_queue: false,
                },
                workflow_run_id,
                dispatch.node_run.id(),
            ) {
                Ok(dispatches) => prepared.extend(dispatches),
                Err(error) => {
                    self.record_notice(
                        session_id,
                        None,
                        self.attachment_store.list_session_attachment_ids(session_id),
                        format!(
                            "Workflow run `{workflow_run_id}` could not schedule downstream node `{}`: {}",
                            dispatch.node_run.node_id(),
                            error
                        ),
                    );
                }
            }
        }
        prepared
    }

    pub(super) fn workflow_turn_prompt_text(
        &self,
        session_id: &str,
        workflow_run_id: &str,
        workflow_node_run_id: &str,
        node_id: &str,
        endpoint_prompt: &str,
        handoff_payloads_json: Option<&str>,
        control_mailbox: Option<&str>,
    ) -> Result<String, DaemonError> {
        let workflow_run = self
            .session_store
            .read()
            .resolve_workflow_run_ref(session_id, workflow_run_id)?;
        let workflow = self
            .session_store
            .read()
            .resolve_workflow_ref(session_id, workflow_run.workflow_id())?;
        let node = workflow.node(node_id);
        let base_directory =
            self.workflow_runtime_base_directory(session_id, workflow_run_id, workflow_node_run_id);
        let instruction_ref = self.workflow_node_instruction_reference(
            base_directory.as_ref(),
            workflow_run_id,
            node_id,
            node.and_then(|node| node.instructions()),
        );
        let turn_index = workflow_run
            .node_runs()
            .iter()
            .filter(|node_run| node_run.node_id() == node_id)
            .count() as u32;
        Ok(
            crate::scheduler::prompt_injection::build_workflow_turn_prompt(
                crate::scheduler::prompt_injection::WorkflowPromptInjectionContext {
                    endpoint_prompt: endpoint_prompt.to_string(),
                    workflow_prompt: workflow_run
                        .invocation_prompt()
                        .map(str::to_string)
                        .unwrap_or_default(),
                    node_instructions: node
                        .and_then(|node| node.instructions())
                        .map(str::trim)
                        .filter(|value| !value.is_empty())
                        .unwrap_or("No node-specific instructions were configured.")
                        .to_string(),
                    instruction_ref,
                    handoff_payloads_json: handoff_payloads_json.map(str::to_string),
                    outgoing_edge_contracts: self.workflow_outgoing_edge_contracts_text(
                        session_id,
                        workflow_run_id,
                        node_id,
                    ),
                    control_mailbox: control_mailbox.map(str::to_string),
                    delivery_token: format!("workflow-ack:{workflow_node_run_id}"),
                    node_turn: node.map(|node| {
                        crate::scheduler::prompt_injection::WorkflowNodeTurnPromptContext {
                            turn_index,
                            max_turns: node.max_turns(),
                            can_complete_workflow_run: node.can_complete_workflow_run(),
                            can_emit_intermediate_output: node.can_emit_intermediate_run_output(),
                        }
                    }),
                    base_directory,
                },
            ),
        )
    }

    pub(super) fn workflow_runtime_base_directory(
        &self,
        session_id: &str,
        workflow_run_id: &str,
        workflow_node_run_id: &str,
    ) -> Option<PathBuf> {
        let session = self.session_store.get_session(session_id).ok()?;
        let workflow_run = session.workflow_run(workflow_run_id)?;
        let node_run = workflow_run
            .node_runs()
            .iter()
            .find(|candidate| candidate.id() == workflow_node_run_id)?;
        self.provider_store
            .get_latest_run_for_agent(session_id, node_run.agent_id())
            .and_then(|run| run.working_directory().cloned())
            .or_else(|| {
                let worktree = PathBuf::from(session.worktree_id());
                if worktree.is_absolute() {
                    Some(worktree)
                } else {
                    std::env::current_dir().ok().map(|cwd| cwd.join(worktree))
                }
            })
    }

    pub(super) fn workflow_node_instruction_reference(
        &self,
        base_directory: Option<&PathBuf>,
        workflow_run_id: &str,
        node_id: &str,
        node_instructions: Option<&str>,
    ) -> Option<String> {
        let root = base_directory?
            .join(".arroba")
            .join("workflow-runtime")
            .join("kernel")
            .join(workflow_run_id)
            .join("workflow-instructions");
        let path = root.join(format!("node-{node_id}.md"));
        if !path.exists() || node_instructions.is_some() {
            if let Err(error) = std::fs::create_dir_all(&root) {
                tracing::debug!(
                    ?error,
                    "Failed to create workflow instruction directory at {:?}",
                    root
                );
                return None;
            }
            let content = node_instructions
                .map(str::to_string)
                .unwrap_or_else(|| {
                    format!(
                        "# Workflow Node Instructions\n\nThis file is daemon-managed. Update node instructions through workflow configuration tooling.\n\nNode: {node_id}\n"
                    )
                });
            if let Err(error) = std::fs::write(&path, content) {
                tracing::debug!(
                    ?error,
                    "Failed to write workflow instruction file at {:?}",
                    path
                );
                return None;
            }
        }
        Some(path.to_string_lossy().to_string())
    }

    pub(super) fn workflow_retry_blocked_claims(&self) -> WorkflowPromptDispatches {
        let mut blocked = Vec::new();
        for session in self.session_store.read().list_sessions() {
            for workflow_run in session.workflow_runs() {
                for node_run in workflow_run.node_runs() {
                    if node_run.status()
                        != crate::session::WorkflowNodeRunStatus::BlockedOnWorkspaceClaim
                    {
                        continue;
                    }
                    let Some(prompt) = node_run
                        .turn_envelope()
                        .and_then(|envelope| envelope.rendered_prompt())
                        .map(str::to_string)
                    else {
                        continue;
                    };
                    blocked.push((
                        session.id().to_string(),
                        workflow_run.id().to_string(),
                        node_run.id().to_string(),
                        node_run.agent_id().to_string(),
                        node_run.node_id().to_string(),
                        prompt,
                    ));
                }
            }
        }
        let mut dispatches = WorkflowPromptDispatches::default();
        for (session_id, workflow_run_id, workflow_node_run_id, agent_id, node_id, prompt_text) in
            blocked
        {
            let claim_id = match self.workflow_dispatch_claim_id(&session_id, &agent_id) {
                Ok(claim_id) => claim_id,
                Err(error) => {
                    self.record_notice(
                        &session_id,
                        None,
                        self.attachment_store.list_session_attachment_ids(&session_id),
                        format!(
                            "Workflow run `{workflow_run_id}` could not retry blocked node `{node_id}`: {error}"
                        ),
                    );
                    continue;
                }
            };
            match self.acquire_workflow_node_workspace_claim(
                &session_id,
                &claim_id,
                &agent_id,
                &workflow_run_id,
                &workflow_node_run_id,
            ) {
                Ok(()) => {
                    let _ = self
                        .session_store
                        .write()
                        .ready_workflow_node_after_workspace_claim(
                            &session_id,
                            &workflow_run_id,
                            &workflow_node_run_id,
                        );
                }
                Err(DaemonError::WorkspaceClaimConflict { .. }) => continue,
                Err(error) => {
                    self.record_notice(
                        &session_id,
                        None,
                        self.attachment_store.list_session_attachment_ids(&session_id),
                        format!(
                            "Workflow run `{workflow_run_id}` could not retry blocked node `{node_id}`: {error}"
                        ),
                    );
                    continue;
                }
            }
            let prompt = crate::session::PromptQueueItem::new(
                self.session_store.reserve_prompt_id(),
                crate::scheduler::runtime::workflow_prompt_source_attachment_id(&workflow_run_id),
                agent_id,
                prompt_text,
                crate::session::PromptStatus::Queued,
            )
            .with_workflow_context(&workflow_run_id, &workflow_node_run_id);
            match self.workflow_submit_prepared_prompt(
                crate::app::KernelPreparedPromptSubmission {
                    session_id: session_id.clone(),
                    prompt,
                    force_queue: false,
                },
                &workflow_run_id,
                &workflow_node_run_id,
            ) {
                Ok(prepared) => dispatches.extend(prepared),
                Err(error) => {
                    self.record_notice(
                        &session_id,
                        None,
                        self.attachment_store.list_session_attachment_ids(&session_id),
                        format!(
                            "Workflow run `{workflow_run_id}` could not retry blocked node `{node_id}`: {error}"
                        ),
                    );
                }
            }
        }
        dispatches
    }

    pub(super) fn workflow_dispatch_has_all_inputs(
        &self,
        session_id: &str,
        workflow_run_id: &str,
        dispatch: &crate::session::WorkflowDispatch,
    ) -> bool {
        let workflow_id = match self
            .session_store
            .read()
            .resolve_workflow_run_ref(session_id, workflow_run_id)
        {
            Ok(run) => run.workflow_id().to_string(),
            Err(_) => return true,
        };
        let workflow = match self
            .session_store
            .read()
            .resolve_workflow_ref(session_id, &workflow_id)
        {
            Ok(workflow) => workflow,
            Err(_) => return true,
        };
        let expected = workflow
            .edges()
            .iter()
            .filter(|edge| edge.to_node_id() == dispatch.node_run.node_id())
            .map(|edge| edge.from_node_id().to_string())
            .collect::<std::collections::BTreeSet<_>>();
        if expected.len() <= 1 {
            return true;
        }
        let run = match self
            .session_store
            .read()
            .resolve_workflow_run_ref(session_id, workflow_run_id)
        {
            Ok(run) => run,
            Err(_) => return true,
        };
        let run_node_by_id = run
            .node_runs()
            .iter()
            .map(|node_run| (node_run.id().to_string(), node_run.node_id().to_string()))
            .collect::<std::collections::BTreeMap<_, _>>();
        let delivered = dispatch
            .messages
            .iter()
            .filter_map(|message| message.source_node_run_id())
            .filter_map(|node_run_id| run_node_by_id.get(node_run_id).cloned())
            .collect::<std::collections::BTreeSet<_>>();
        expected.is_subset(&delivered)
    }

    pub(super) fn workflow_maybe_start_next_queued_launch(&self, session_id: &str) {
        let queued_launch = match self
            .session_store
            .write()
            .dequeue_next_workflow_launch(session_id)
        {
            Ok(Some(queued_launch)) => queued_launch,
            Ok(None) => return,
            Err(error) => {
                self.record_notice(
                    session_id,
                    None,
                    self.attachment_store
                        .list_session_attachment_ids(session_id),
                    format!("Failed to start queued workflow launch: {error}"),
                );
                return;
            }
        };
        if let Some(watchdog_id) = queued_launch.watchdog_id() {
            let _ = self
                .session_store
                .write()
                .mark_workflow_watchdog_pending_started(session_id, watchdog_id);
        }
        match self.workflow_invoke_queued_launch(session_id, queued_launch.clone()) {
            Ok((
                crate::app::workflow_runtime::WorkflowLaunchOutcome::Started {
                    workflow_run,
                    workflow,
                    endpoint,
                },
                _dispatches,
            )) => {
                self.record_notice(
                    session_id,
                    None,
                    self.attachment_store
                        .list_session_attachment_ids(session_id),
                    format!(
                        "Started queued workflow run `{}` for workflow `{}` endpoint `{}`.",
                        workflow_run.id(),
                        workflow.id(),
                        endpoint.id()
                    ),
                );
            }
            Ok((crate::app::workflow_runtime::WorkflowLaunchOutcome::Queued { .. }, _)) => {}
            Err(error) => {
                if let Some(watchdog_id) = queued_launch.watchdog_id() {
                    let _ = self.session_store.write().mark_workflow_watchdog_failed(
                        session_id,
                        watchdog_id,
                        error.to_string(),
                    );
                }
                self.record_notice(
                    session_id,
                    None,
                    self.attachment_store
                        .list_session_attachment_ids(session_id),
                    format!(
                        "Queued workflow launch `{}` failed: {error}",
                        queued_launch.id()
                    ),
                );
            }
        }
    }

    pub(super) fn workflow_resume_run(
        &self,
        session_id: &str,
        workflow_run_ref: &str,
    ) -> Result<(crate::session::WorkflowRun, WorkflowPromptDispatches), DaemonError> {
        let workflow_run = self
            .session_store
            .write()
            .resume_workflow_run(session_id, workflow_run_ref)?;
        let resumable = workflow_run
            .node_runs()
            .iter()
            .filter_map(|node_run| {
                let prompt = node_run.turn_envelope()?.rendered_prompt()?.to_string();
                Some((
                    node_run.id().to_string(),
                    node_run.agent_id().to_string(),
                    prompt,
                ))
            })
            .collect::<Vec<_>>();
        let mut dispatches = WorkflowPromptDispatches::default();
        for (workflow_node_run_id, agent_id, prompt_text) in resumable {
            let prompt = crate::session::PromptQueueItem::new(
                self.session_store.reserve_prompt_id(),
                crate::scheduler::runtime::workflow_prompt_source_attachment_id(workflow_run.id()),
                agent_id,
                prompt_text,
                crate::session::PromptStatus::Queued,
            )
            .with_workflow_context(workflow_run.id(), &workflow_node_run_id);
            match self.workflow_submit_prepared_prompt(
                crate::app::KernelPreparedPromptSubmission {
                    session_id: session_id.to_string(),
                    prompt,
                    force_queue: false,
                },
                workflow_run.id(),
                &workflow_node_run_id,
            ) {
                Ok(prepared) => dispatches.extend(prepared),
                Err(error) => {
                    self.record_notice(
                        session_id,
                        None,
                        self.attachment_store
                            .list_session_attachment_ids(session_id),
                        format!(
                            "Workflow run `{}` could not resume node prompt: {}",
                            workflow_run.id(),
                            error
                        ),
                    );
                }
            }
        }
        let workflow_run = self
            .session_store
            .read()
            .resolve_workflow_run_ref(session_id, workflow_run.id())?;
        Ok((workflow_run, dispatches))
    }

    pub(super) fn dispatch_workflow_runtime_tool_call(
        &self,
        tool_name: String,
        arguments: serde_json::Value,
        context: crate::transport::runtime_tools::WorkflowRuntimeToolContext,
    ) -> Result<
        (
            crate::transport::runtime_tools::RuntimeToolResult,
            WorkflowPromptDispatches,
        ),
        DaemonError,
    > {
        let canonical_tool_name = tool_name
            .strip_prefix("arroba_")
            .unwrap_or(tool_name.as_str())
            .to_string();
        let arguments_json = serde_json::to_string(&arguments)
            .unwrap_or_else(|_| String::from("<unserializable runtime tool arguments>"));
        let mut dispatches = WorkflowPromptDispatches::default();
        let result = match canonical_tool_name.as_str() {
            crate::transport::runtime_tools::ACK_WORKFLOW_TURN_TOOL => {
                let args = serde_json::from_value::<
                    crate::transport::runtime_tools::AckWorkflowTurnArgs,
                >(arguments.clone())
                .map_err(|error| DaemonError::LocalTransport {
                    operation: "runtime_tool_ack_workflow_turn",
                    message: format!("invalid tool arguments: {error}"),
                })?;
                let workflow_run_id = self
                    .session_store
                    .read()
                    .resolve_workflow_run_ref(&context.session_id, &context.workflow_run_ref)?
                    .id()
                    .to_string();
                let workflow_run = self.session_store.write().ack_workflow_turn(
                    &context.session_id,
                    &workflow_run_id,
                    &context.workflow_node_run_id,
                    &args.delivery_token,
                )?;
                Ok(crate::transport::runtime_tools::RuntimeToolResult {
                    ok: true,
                    payload: serde_json::json!({
                        "workflow_run_id": workflow_run.id(),
                        "workflow_node_run_id": context.workflow_node_run_id,
                        "state": "acknowledged",
                        "next_action": "Continue this same workflow turn. This acknowledgement is not the final answer. If this turn requires final workflow run output, call validate_and_submit_workflow_run_output before stopping; otherwise emit the required final fenced json block before stopping.",
                    }),
                })
            }
            crate::transport::runtime_tools::VALIDATE_WORKFLOW_OUTPUT_TOOL => {
                let args = serde_json::from_value::<
                    crate::transport::runtime_tools::ValidateWorkflowOutputArgs,
                >(arguments.clone())
                .map_err(|error| DaemonError::LocalTransport {
                    operation: "runtime_tool_validate_workflow_output",
                    message: format!("invalid tool arguments: {error}"),
                })?;
                if !context.allowed_output_schema_refs.is_empty()
                    && !context
                        .allowed_output_schema_refs
                        .iter()
                        .any(|schema_ref| schema_ref == &args.output_schema_ref)
                {
                    return Err(DaemonError::LocalTransport {
                        operation: "runtime_tool_validate_workflow_output",
                        message: format!(
                            "schema ref `{}` is not allowed for workflow node run `{}`",
                            args.output_schema_ref, context.workflow_node_run_id
                        ),
                    });
                }
                let warning = crate::transport::runtime_tools::validate_workflow_output_schema(
                    &args.output_schema_ref,
                    &args.output_json,
                )
                .err();
                Ok(crate::transport::runtime_tools::RuntimeToolResult {
                    ok: true,
                    payload: serde_json::json!({
                        "valid": warning.is_none(),
                        "warning": warning,
                        "next_action": if warning.is_none() {
                            "Validation passed. Now finish this same workflow turn by emitting exactly one final fenced json block and then stop."
                        } else {
                            "Validation failed or warned. Revise the output and call validate_workflow_output again before finalizing."
                        },
                    }),
                })
            }
            crate::transport::runtime_tools::VALIDATE_AND_SUBMIT_WORKFLOW_RUN_OUTPUT_TOOL
            | crate::transport::runtime_tools::VALIDATE_AND_SUBMIT_INTERMEDIATE_WORKFLOW_RUN_OUTPUT_TOOL =>
            {
                let is_final = canonical_tool_name
                    == crate::transport::runtime_tools::VALIDATE_AND_SUBMIT_WORKFLOW_RUN_OUTPUT_TOOL;
                if is_final && !context.can_complete_workflow_run {
                    return Err(DaemonError::LocalTransport {
                        operation: "runtime_tool_validate_and_submit_workflow_run_output",
                        message:
                            "current workflow node run is not allowed to complete the workflow run"
                                .to_string(),
                    });
                }
                if !is_final && !context.can_emit_intermediate_workflow_run_output {
                    return Err(DaemonError::LocalTransport {
                        operation: "runtime_tool_validate_and_submit_intermediate_workflow_run_output",
                        message:
                            "current workflow node run is not allowed to emit intermediate workflow run output"
                                .to_string(),
                    });
                }
                let args = serde_json::from_value::<
                    crate::transport::runtime_tools::ValidateAndSubmitWorkflowRunOutputArgs,
                >(arguments.clone())
                .map_err(|error| DaemonError::LocalTransport {
                    operation: if is_final {
                        "runtime_tool_validate_and_submit_workflow_run_output"
                    } else {
                        "runtime_tool_validate_and_submit_intermediate_workflow_run_output"
                    },
                    message: format!("invalid tool arguments: {error}"),
                })?;
                let workflow_run_id = self
                    .session_store
                    .read()
                    .resolve_workflow_run_ref(&context.session_id, &context.workflow_run_ref)?
                    .id()
                    .to_string();
                let schema_ref = if is_final {
                    context.workflow_run_output_schema_ref.as_deref()
                } else {
                    context.workflow_intermediate_output_schema_ref.as_deref()
                };
                let warning = schema_ref.and_then(|schema_ref| {
                    crate::transport::runtime_tools::validate_workflow_output_schema(
                        schema_ref,
                        &args.workflow_output_json,
                    )
                    .err()
                });
                let output = crate::session::WorkflowOutputPayload::new(
                    args.workflow_output_json,
                    Vec::<crate::session::WorkflowArtifactRef>::new(),
                );
                let workflow_run = if is_final {
                    self.session_store.write().submit_workflow_run_final_output(
                        &context.session_id,
                        &workflow_run_id,
                        &context.workflow_node_run_id,
                        output,
                        warning.is_none(),
                        warning.clone(),
                    )?
                } else {
                    self.session_store.write().submit_workflow_run_intermediate_output(
                        &context.session_id,
                        &workflow_run_id,
                        &context.workflow_node_run_id,
                        output,
                        warning.is_none(),
                        warning.clone(),
                    )?
                };
                if !is_final && warning.is_none() {
                    let update = self
                        .session_store
                        .write()
                        .release_workflow_intermediate_output_downstream(
                            &context.session_id,
                            &workflow_run_id,
                            &context.workflow_node_run_id,
                        )?;
                    for warning in &update.validation_warnings {
                        self.workflow_record_failure(
                            &context.session_id,
                            &workflow_run_id,
                            &crate::session::WorkflowFailureEvent::new(
                                crate::session::WorkflowFailureKind::OutputValidationFailed,
                                &context.workflow_node_run_id,
                                vec![warning.edge_id.clone()],
                                warning.message.clone(),
                            ),
                        );
                        self.record_notice(
                            &context.session_id,
                            None,
                            self.attachment_store
                                .list_session_attachment_ids(&context.session_id),
                            format!(
                                "Workflow output validation warning on edge `{}`: {}",
                                warning.edge_id, warning.message
                            ),
                        );
                    }
                    dispatches.extend(self.workflow_prepare_dispatches(
                        &context.session_id,
                        &workflow_run_id,
                        &update.dispatches,
                    ));
                    let _ = self.session_snapshot(&context.session_id);
                }
                Ok(crate::transport::runtime_tools::RuntimeToolResult {
                    ok: true,
                    payload: serde_json::json!({
                        "submitted": true,
                        "valid": warning.is_none(),
                        "warning": warning,
                        "workflow_run_id": workflow_run.id(),
                        "workflow_node_run_id": context.workflow_node_run_id,
                        "next_action": if is_final {
                            "Final workflow run output was submitted. If it is valid with no warning, finish this same workflow turn now."
                        } else {
                            "Intermediate workflow run output was submitted. Continue this same workflow turn and emit the required final fenced json block before stopping."
                        },
                    }),
                })
            }
            crate::transport::runtime_tools::WORKFLOW_CONSOLE_READ_TOOL => {
                let workflow_run = self
                    .session_store
                    .read()
                    .resolve_workflow_run_ref(&context.session_id, &context.workflow_run_ref)?;
                let console = self
                    .session_store
                    .read()
                    .read_workflow_console(&context.session_id, workflow_run.workflow_id())?;
                Ok(crate::transport::runtime_tools::RuntimeToolResult {
                    ok: true,
                    payload: serde_json::json!({
                        "workflow_id": console.workflow_id(),
                        "entries": console.entries().iter().map(|entry| serde_json::json!({
                            "timestamp_ms": entry.timestamp_ms(),
                            "source_node_run_id": entry.source_node_run_id(),
                            "source_agent_id": entry.source_agent_id(),
                            "text": entry.text(),
                        })).collect::<Vec<_>>(),
                    }),
                })
            }
            crate::transport::runtime_tools::WORKFLOW_CONSOLE_WRITE_TOOL => {
                let args = serde_json::from_value::<
                    crate::transport::runtime_tools::WorkflowConsoleWriteArgs,
                >(arguments.clone())
                .map_err(|error| DaemonError::LocalTransport {
                    operation: "runtime_tool_workflow_console_write",
                    message: format!("invalid tool arguments: {error}"),
                })?;
                let workflow_run = self
                    .session_store
                    .read()
                    .resolve_workflow_run_ref(&context.session_id, &context.workflow_run_ref)?;
                let source_agent_id = self.workflow_node_agent_id(
                    &context.session_id,
                    &context.workflow_run_ref,
                    &context.workflow_node_run_id,
                );
                let entry = self.session_store.write().append_workflow_console_entry(
                    &context.session_id,
                    workflow_run.workflow_id(),
                    Some(context.workflow_node_run_id.clone()),
                    source_agent_id,
                    &args.text,
                )?;
                Ok(crate::transport::runtime_tools::RuntimeToolResult {
                    ok: true,
                    payload: serde_json::json!({
                        "timestamp_ms": entry.timestamp_ms(),
                        "source_node_run_id": entry.source_node_run_id(),
                        "source_agent_id": entry.source_agent_id(),
                        "text": entry.text(),
                    }),
                })
            }
            crate::transport::runtime_tools::WORKFLOW_CONSOLE_CLEAR_TOOL => {
                let workflow_run = self
                    .session_store
                    .read()
                    .resolve_workflow_run_ref(&context.session_id, &context.workflow_run_ref)?;
                let console = self
                    .session_store
                    .write()
                    .clear_workflow_console(&context.session_id, workflow_run.workflow_id())?;
                Ok(crate::transport::runtime_tools::RuntimeToolResult {
                    ok: true,
                    payload: serde_json::json!({
                        "cleared": true,
                        "workflow_id": console.workflow_id(),
                    }),
                })
            }
            other => Err(DaemonError::LocalTransport {
                operation: "dispatch_runtime_tool_call",
                message: format!("unsupported runtime tool `{other}`"),
            }),
        };
        let result_json = match &result {
            Ok(result) => Some(
                serde_json::to_string(&result.payload)
                    .unwrap_or_else(|_| String::from("<unserializable runtime tool result>")),
            ),
            Err(error) => Some(serde_json::json!({"error": error.to_string()}).to_string()),
        };
        let ok = result.as_ref().map(|entry| entry.ok).unwrap_or(false);
        let _ = self
            .session_store
            .write()
            .record_workflow_runtime_tool_call(
                &context.session_id,
                &context.workflow_node_run_id,
                crate::session::WorkflowRuntimeToolCallEvent::new(
                    canonical_tool_name,
                    arguments_json,
                    result_json,
                    ok,
                ),
            );
        let _ = self.session_snapshot(&context.session_id);
        result.map(|result| (result, dispatches))
    }

    pub(super) fn workflow_node_agent_id(
        &self,
        session_id: &str,
        workflow_run_ref: &str,
        workflow_node_run_id: &str,
    ) -> Option<String> {
        self.session_store
            .read()
            .resolve_workflow_run_ref(session_id, workflow_run_ref)
            .ok()
            .and_then(|workflow_run| {
                workflow_run
                    .node_runs()
                    .iter()
                    .find(|node_run| node_run.id() == workflow_node_run_id)
                    .map(|node_run| node_run.agent_id().to_string())
            })
    }

    pub(super) fn workflow_tool_context(
        &self,
        session_id: String,
        workflow_run_ref: String,
        workflow_node_run_id: String,
        delivery_token: Option<String>,
    ) -> Result<crate::transport::runtime_tools::WorkflowRuntimeToolContext, DaemonError> {
        let workflow_run = self
            .session_store
            .read()
            .resolve_workflow_run_ref(&session_id, &workflow_run_ref)?;
        let workflow = self
            .session_store
            .read()
            .resolve_workflow_ref(&session_id, workflow_run.workflow_id())?;
        let node_id = workflow_run
            .node_runs()
            .iter()
            .find(|node_run| node_run.id() == workflow_node_run_id)
            .map(|node_run| node_run.node_id().to_string())
            .ok_or_else(|| DaemonError::InvalidWorkflowGraphReference {
                session_id: session_id.clone(),
                workflow_id: workflow.id().to_string(),
                reference: workflow_node_run_id.clone(),
                message: "workflow node run was not found while resolving runtime tool scope",
            })?;
        let allowed_output_schema_refs = workflow
            .edges()
            .iter()
            .filter(|edge| edge.from_node_id() == node_id)
            .filter_map(|edge| edge.output_schema_ref().map(str::to_string))
            .collect();
        let node = workflow.node(&node_id);
        let can_complete_workflow_run = node.is_some_and(|node| node.can_complete_workflow_run());
        let can_emit_intermediate_workflow_run_output =
            node.is_some_and(|node| node.can_emit_intermediate_run_output());
        let workflow_intermediate_output_schema_ref = node
            .and_then(|node| node.intermediate_output_schema_ref())
            .map(str::to_string)
            .or_else(|| {
                workflow
                    .intermediate_output_schema_ref()
                    .map(str::to_string)
            });
        Ok(
            crate::transport::runtime_tools::WorkflowRuntimeToolContext {
                session_id,
                workflow_run_ref,
                workflow_node_run_id,
                delivery_token,
                allowed_output_schema_refs,
                workflow_run_output_schema_ref: workflow
                    .run_output_schema_ref()
                    .map(str::to_string),
                workflow_intermediate_output_schema_ref,
                can_complete_workflow_run,
                can_emit_intermediate_workflow_run_output,
            },
        )
    }

    pub(super) fn resolve_owned_authenticated_workflow_turn(
        &self,
        session_id: &str,
        candidate_agent_ids: &[String],
        delivery_token: Option<&str>,
    ) -> Result<(String, String), DaemonError> {
        let session = self.session_store.get_session(session_id)?;
        let agent_matches = |agent_id: &str| {
            candidate_agent_ids.is_empty()
                || candidate_agent_ids
                    .iter()
                    .any(|candidate| candidate == agent_id)
        };
        for agent_id in candidate_agent_ids {
            if let Some(prompt) = self
                .prompt_state_owner
                .active_prompt_for_agent(&session, agent_id)
            {
                let (Some(workflow_run_ref), Some(workflow_node_run_id)) =
                    (prompt.workflow_run_id(), prompt.workflow_node_run_id())
                else {
                    continue;
                };
                let matches_token = delivery_token.is_none_or(|requested| {
                    session
                        .workflow_runs()
                        .iter()
                        .find(|workflow_run| workflow_run.id() == workflow_run_ref)
                        .and_then(|workflow_run| {
                            workflow_run
                                .node_runs()
                                .iter()
                                .find(|node_run| node_run.id() == workflow_node_run_id)
                        })
                        .and_then(|node_run| node_run.turn_envelope())
                        .is_some_and(|envelope| envelope.delivery_token() == requested)
                });
                if matches_token {
                    return Ok((
                        workflow_run_ref.to_string(),
                        workflow_node_run_id.to_string(),
                    ));
                }
            }
        }
        let mut running_turns = session
            .workflow_runs()
            .iter()
            .flat_map(|workflow_run| {
                workflow_run.node_runs().iter().filter_map(|node_run| {
                    let envelope = node_run.turn_envelope()?;
                    if node_run.status() != crate::session::WorkflowNodeRunStatus::Running
                        || !matches!(
                            envelope.state(),
                            crate::session::WorkflowTurnRuntimeState::Prepared
                                | crate::session::WorkflowTurnRuntimeState::Dispatched
                                | crate::session::WorkflowTurnRuntimeState::Acknowledged
                        )
                    {
                        return None;
                    }
                    if !agent_matches(node_run.agent_id()) {
                        return None;
                    }
                    if delivery_token
                        .is_some_and(|requested| envelope.delivery_token() != requested)
                    {
                        return None;
                    }
                    Some((workflow_run.id().to_string(), node_run.id().to_string()))
                })
            })
            .collect::<Vec<_>>();
        match running_turns.len() {
            1 => Ok(running_turns.remove(0)),
            0 => Err(DaemonError::LocalTransport {
                operation: "dispatch_authenticated_runtime_tool_call",
                message: "no active workflow turn for authenticated provider run".to_string(),
            }),
            _ => Err(DaemonError::LocalTransport {
                operation: "dispatch_authenticated_runtime_tool_call",
                message: "multiple workflow turns matched the authenticated provider run"
                    .to_string(),
            }),
        }
    }
}
