use crate::app::{workflow_runtime::WorkflowLaunchOutcome, DaemonApp};
use crate::error::DaemonError;
use crate::local::{
    AckWorkflowTurnRequest, AddWorkflowEdgeRequest, AddWorkflowNodeRequest,
    AliasWorkflowEndpointRequest, AliasWorkflowRequest, BindWorkflowEndpointRequest,
    CancelWorkflowRunRequest, ClearQueuedWorkflowLaunchesRequest, CreateWorkflowEndpointRequest,
    CreateWorkflowRequest, CreateWorkflowWatchdogRequest, GetWorkflowRunRequest,
    InvokeWorkflowEndpointRequest, ListQueuedWorkflowLaunchesRequest, ListWorkflowRunsRequest,
    ListWorkflowWatchdogsRequest, ListWorkflowsRequest, LocalDaemonResponse,
    RemoveQueuedWorkflowLaunchRequest, RemoveWorkflowEdgeRequest, RemoveWorkflowNodeRequest,
    RemoveWorkflowWatchdogRequest, ResolveWorkflowRequest, ResumeWorkflowRunRequest,
    SetWorkflowFlushContextRequest, SetWorkflowIntermediateOutputSchemaRequest,
    SetWorkflowLaunchPolicyRequest, SetWorkflowNodeCanCompleteRunRequest,
    SetWorkflowNodeCanEmitIntermediateOutputRequest,
    SetWorkflowNodeIntermediateOutputSchemaRequest, SetWorkflowNodeMaxTurnsRequest,
    SetWorkflowRunOutputSchemaRequest, SetWorkflowWatchdogEnabledRequest,
    UpdateWorkflowNodeInstructionsRequest, ValidateWorkflowOutputRequest,
};
use crate::session::{RuntimeSession, SessionService};
use std::sync::MutexGuard;

pub(crate) struct KernelWorkflowService<'a> {
    context: KernelWorkflowContext<'a>,
}

struct KernelWorkflowContext<'a> {
    app: &'a mut DaemonApp,
}

impl<'a> KernelWorkflowContext<'a> {
    fn new(app: &'a mut DaemonApp) -> Self {
        Self { app }
    }

    fn sessions(&self) -> SessionService {
        self.app.sessions()
    }

    fn sessions_mut(&mut self) -> MutexGuard<'_, SessionService> {
        self.app.sessions_mut()
    }

    fn session_snapshot(&self, session_id: &str) -> Result<RuntimeSession, DaemonError> {
        crate::app::KernelSessionReadService::new(self.app).session_snapshot(session_id)
    }

    fn session_agents(&self, session_id: &str) -> Vec<crate::agent::AgentInstance> {
        self.app.agents().get_session_agents(session_id)
    }

    fn session_has_agent(&self, session_id: &str, agent_id: &str) -> bool {
        self.session_agents(session_id)
            .into_iter()
            .any(|agent| agent.id() == agent_id)
    }

    fn invoke_workflow_endpoint_with_admission(
        &mut self,
        request: InvokeWorkflowEndpointRequest,
    ) -> Result<WorkflowLaunchOutcome, DaemonError> {
        self.app.invoke_workflow_endpoint_with_admission(
            &request.session_id,
            &request.workflow_ref,
            &request.endpoint_ref,
            request.prompt,
        )
    }

    fn cancel_active_prompt_for_runtime(&mut self, session_id: &str) -> Result<(), DaemonError> {
        let _ = crate::transport::TransportService::cancel_active_prompt_for_runtime(
            self.app, session_id,
        )?;
        Ok(())
    }

    fn active_prompt_workflow_run_id(
        &mut self,
        session_id: &str,
    ) -> Result<Option<String>, DaemonError> {
        let Some(agent_id) = self.app.prompt_owner_active_prompt_agent_id(session_id)? else {
            return Ok(None);
        };
        Ok(self
            .app
            .prompt_owner_active_prompt_for_agent(session_id, &agent_id)?
            .and_then(|prompt| prompt.workflow_run_id().map(str::to_string)))
    }

    fn remove_queued_prompts_by_workflow_run(
        &mut self,
        session_id: &str,
        workflow_run_id: &str,
    ) -> Result<(), DaemonError> {
        let _ = self
            .app
            .prompt_owner_remove_queued_prompts_by_workflow_run(session_id, workflow_run_id)?;
        Ok(())
    }

    fn drain_session_workflow_launch_queue(&mut self, session_id: &str) -> Result<(), DaemonError> {
        let _ = self.app.drain_session_workflow_launch_queue(session_id)?;
        Ok(())
    }

    fn resume_workflow_run(
        &mut self,
        session_id: &str,
        workflow_run_ref: &str,
    ) -> Result<crate::session::WorkflowRun, DaemonError> {
        self.app
            .resume_workflow_run_from_runtime(session_id, workflow_run_ref)
    }

    fn dispatch_runtime_tool_call(
        &mut self,
        call: crate::transport::runtime_tools::RuntimeToolCall,
    ) -> Result<crate::transport::runtime_tools::RuntimeToolResult, DaemonError> {
        crate::transport::runtime_tools::dispatch_runtime_tool_call(self.app, call)
    }
}

impl<'a> KernelWorkflowService<'a> {
    pub(crate) fn new(app: &'a mut DaemonApp) -> Self {
        Self {
            context: KernelWorkflowContext::new(app),
        }
    }

    pub(crate) fn create_workflow(
        &mut self,
        request: CreateWorkflowRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let workflow = self
            .context
            .sessions_mut()
            .create_workflow(&request.session_id, request.alias)?;
        let session = self.context.session_snapshot(&request.session_id)?;
        Ok(LocalDaemonResponse::WorkflowCreated { workflow, session })
    }

    pub(crate) fn alias_workflow(
        &mut self,
        request: AliasWorkflowRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let workflow = self.context.sessions_mut().assign_workflow_alias(
            &request.session_id,
            &request.workflow_ref,
            request.alias,
        )?;
        let session = self.context.session_snapshot(&request.session_id)?;
        Ok(LocalDaemonResponse::WorkflowAliased { workflow, session })
    }

    pub(crate) fn list_workflows(
        &mut self,
        request: ListWorkflowsRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        Ok(LocalDaemonResponse::WorkflowsListed {
            workflows: self
                .context
                .sessions()
                .list_workflows(&request.session_id)?,
        })
    }

    pub(crate) fn resolve_workflow(
        &mut self,
        request: ResolveWorkflowRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        Ok(LocalDaemonResponse::WorkflowResolved {
            workflow: self
                .context
                .sessions()
                .resolve_workflow_ref(&request.session_id, &request.workflow_ref)?,
        })
    }

    pub(crate) fn create_workflow_endpoint(
        &mut self,
        request: CreateWorkflowEndpointRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let endpoint = self.context.sessions_mut().create_workflow_endpoint(
            &request.session_id,
            &request.workflow_ref,
            &request.entry_node_id,
            request.alias,
        )?;
        let workflow = self
            .context
            .sessions()
            .resolve_workflow_ref(&request.session_id, &request.workflow_ref)?;
        let session = self.context.session_snapshot(&request.session_id)?;
        Ok(LocalDaemonResponse::WorkflowEndpointCreated {
            endpoint,
            workflow,
            session,
        })
    }

    pub(crate) fn alias_workflow_endpoint(
        &mut self,
        request: AliasWorkflowEndpointRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let endpoint = self.context.sessions_mut().assign_workflow_endpoint_alias(
            &request.session_id,
            &request.workflow_ref,
            &request.endpoint_ref,
            request.alias,
        )?;
        let workflow = self
            .context
            .sessions()
            .resolve_workflow_ref(&request.session_id, &request.workflow_ref)?;
        let session = self.context.session_snapshot(&request.session_id)?;
        Ok(LocalDaemonResponse::WorkflowEndpointAliased {
            endpoint,
            workflow,
            session,
        })
    }

    pub(crate) fn bind_workflow_endpoint(
        &mut self,
        request: BindWorkflowEndpointRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let endpoint = self.context.sessions_mut().bind_workflow_endpoint(
            &request.session_id,
            &request.workflow_ref,
            &request.endpoint_ref,
            &request.entry_node_id,
        )?;
        let workflow = self
            .context
            .sessions()
            .resolve_workflow_ref(&request.session_id, &request.workflow_ref)?;
        let session = self.context.session_snapshot(&request.session_id)?;
        Ok(LocalDaemonResponse::WorkflowEndpointBound {
            endpoint,
            workflow,
            session,
        })
    }

    pub(crate) fn add_workflow_node(
        &mut self,
        request: AddWorkflowNodeRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let agent_exists = self
            .context
            .session_has_agent(&request.session_id, &request.agent_id);
        if !agent_exists {
            return Err(DaemonError::AgentNotFound {
                agent_id: request.agent_id,
            });
        }
        let node = self.context.sessions_mut().add_workflow_node(
            &request.session_id,
            &request.workflow_ref,
            &request.agent_id,
        )?;
        let workflow = self
            .context
            .sessions()
            .resolve_workflow_ref(&request.session_id, &request.workflow_ref)?;
        let session = self.context.session_snapshot(&request.session_id)?;
        Ok(LocalDaemonResponse::WorkflowNodeAdded {
            node,
            workflow,
            session,
        })
    }

    pub(crate) fn remove_workflow_node(
        &mut self,
        request: RemoveWorkflowNodeRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let node = self.context.sessions_mut().remove_workflow_node(
            &request.session_id,
            &request.workflow_ref,
            &request.node_id,
        )?;
        let workflow = self
            .context
            .sessions()
            .resolve_workflow_ref(&request.session_id, &request.workflow_ref)?;
        let session = self.context.session_snapshot(&request.session_id)?;
        Ok(LocalDaemonResponse::WorkflowNodeRemoved {
            node,
            workflow,
            session,
        })
    }

    pub(crate) fn update_workflow_node_instructions(
        &mut self,
        request: UpdateWorkflowNodeInstructionsRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let node = self
            .context
            .sessions_mut()
            .update_workflow_node_instructions(
                &request.session_id,
                &request.workflow_ref,
                &request.node_id,
                request.instructions.clone(),
            )?;
        let workflow = self
            .context
            .sessions()
            .resolve_workflow_ref(&request.session_id, &request.workflow_ref)?;
        let session = self.context.session_snapshot(&request.session_id)?;
        Ok(LocalDaemonResponse::WorkflowNodeInstructionsUpdated {
            node,
            workflow,
            session,
        })
    }

    pub(crate) fn set_workflow_node_can_complete_run(
        &mut self,
        request: SetWorkflowNodeCanCompleteRunRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let node = self
            .context
            .sessions_mut()
            .set_workflow_node_can_complete_run(
                &request.session_id,
                &request.workflow_ref,
                &request.node_id,
                request.can_complete_workflow_run,
            )?;
        let workflow = self
            .context
            .sessions()
            .resolve_workflow_ref(&request.session_id, &request.workflow_ref)?;
        let session = self.context.session_snapshot(&request.session_id)?;
        Ok(LocalDaemonResponse::WorkflowNodeCanCompleteRunUpdated {
            node,
            workflow,
            session,
        })
    }

    pub(crate) fn set_workflow_node_can_emit_intermediate_output(
        &mut self,
        request: SetWorkflowNodeCanEmitIntermediateOutputRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let node = self
            .context
            .sessions_mut()
            .set_workflow_node_can_emit_intermediate_output(
                &request.session_id,
                &request.workflow_ref,
                &request.node_id,
                request.can_emit_intermediate_workflow_run_output,
            )?;
        let workflow = self
            .context
            .sessions()
            .resolve_workflow_ref(&request.session_id, &request.workflow_ref)?;
        let session = self.context.session_snapshot(&request.session_id)?;
        Ok(
            LocalDaemonResponse::WorkflowNodeCanEmitIntermediateOutputUpdated {
                node,
                workflow,
                session,
            },
        )
    }

    pub(crate) fn set_workflow_node_intermediate_output_schema(
        &mut self,
        request: SetWorkflowNodeIntermediateOutputSchemaRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let node = self
            .context
            .sessions_mut()
            .set_workflow_node_intermediate_output_schema_ref(
                &request.session_id,
                &request.workflow_ref,
                &request.node_id,
                request.intermediate_output_schema_ref.clone(),
            )?;
        let workflow = self
            .context
            .sessions()
            .resolve_workflow_ref(&request.session_id, &request.workflow_ref)?;
        let session = self.context.session_snapshot(&request.session_id)?;
        Ok(
            LocalDaemonResponse::WorkflowNodeIntermediateOutputSchemaUpdated {
                node,
                workflow,
                session,
            },
        )
    }

    pub(crate) fn set_workflow_node_max_turns(
        &mut self,
        request: SetWorkflowNodeMaxTurnsRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let node = self.context.sessions_mut().set_workflow_node_max_turns(
            &request.session_id,
            &request.workflow_ref,
            &request.node_id,
            request.max_turns,
        )?;
        let workflow = self
            .context
            .sessions()
            .resolve_workflow_ref(&request.session_id, &request.workflow_ref)?;
        let session = self.context.session_snapshot(&request.session_id)?;
        Ok(LocalDaemonResponse::WorkflowNodeMaxTurnsUpdated {
            node,
            workflow,
            session,
        })
    }

    pub(crate) fn add_workflow_edge(
        &mut self,
        request: AddWorkflowEdgeRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let edge = self.context.sessions_mut().add_workflow_edge(
            &request.session_id,
            &request.workflow_ref,
            &request.from_node_id,
            &request.to_node_id,
            request.output_schema_ref.clone(),
            request.validation_policy,
        )?;
        let workflow = self
            .context
            .sessions()
            .resolve_workflow_ref(&request.session_id, &request.workflow_ref)?;
        let session = self.context.session_snapshot(&request.session_id)?;
        Ok(LocalDaemonResponse::WorkflowEdgeAdded {
            edge,
            workflow,
            session,
        })
    }

    pub(crate) fn remove_workflow_edge(
        &mut self,
        request: RemoveWorkflowEdgeRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let edge = self.context.sessions_mut().remove_workflow_edge(
            &request.session_id,
            &request.workflow_ref,
            &request.edge_id,
        )?;
        let workflow = self
            .context
            .sessions()
            .resolve_workflow_ref(&request.session_id, &request.workflow_ref)?;
        let session = self.context.session_snapshot(&request.session_id)?;
        Ok(LocalDaemonResponse::WorkflowEdgeRemoved {
            edge,
            workflow,
            session,
        })
    }

    pub(crate) fn set_workflow_flush_context(
        &mut self,
        request: SetWorkflowFlushContextRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let workflow = self
            .context
            .sessions_mut()
            .set_workflow_flush_agent_context_before_run(
                &request.session_id,
                &request.workflow_ref,
                request.flush_agent_context_before_run,
            )?;
        let session = self.context.session_snapshot(&request.session_id)?;
        Ok(LocalDaemonResponse::WorkflowFlushContextUpdated { workflow, session })
    }

    pub(crate) fn set_workflow_run_output_schema(
        &mut self,
        request: SetWorkflowRunOutputSchemaRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let workflow = self
            .context
            .sessions_mut()
            .set_workflow_run_output_schema_ref(
                &request.session_id,
                &request.workflow_ref,
                request.run_output_schema_ref.clone(),
            )?;
        let session = self.context.session_snapshot(&request.session_id)?;
        Ok(LocalDaemonResponse::WorkflowRunOutputSchemaUpdated { workflow, session })
    }

    pub(crate) fn set_workflow_intermediate_output_schema(
        &mut self,
        request: SetWorkflowIntermediateOutputSchemaRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let workflow = self
            .context
            .sessions_mut()
            .set_workflow_intermediate_output_schema_ref(
                &request.session_id,
                &request.workflow_ref,
                request.intermediate_output_schema_ref.clone(),
            )?;
        let session = self.context.session_snapshot(&request.session_id)?;
        Ok(LocalDaemonResponse::WorkflowIntermediateOutputSchemaUpdated { workflow, session })
    }

    pub(crate) fn set_workflow_launch_policy(
        &mut self,
        request: SetWorkflowLaunchPolicyRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let session = self
            .context
            .sessions_mut()
            .set_workflow_launch_policy(&request.session_id, request.policy)?;
        let mut session = session;
        session.set_agents(self.context.session_agents(&request.session_id));
        Ok(LocalDaemonResponse::WorkflowLaunchPolicyUpdated { session })
    }

    pub(crate) fn invoke_workflow_endpoint(
        &mut self,
        request: InvokeWorkflowEndpointRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let session_id = request.session_id.clone();
        let outcome = self
            .context
            .invoke_workflow_endpoint_with_admission(request)?;
        let session = self.context.session_snapshot(&session_id)?;
        match outcome {
            WorkflowLaunchOutcome::Started {
                workflow_run,
                workflow,
                endpoint,
            } => Ok(LocalDaemonResponse::WorkflowRunInvoked {
                workflow_run,
                workflow,
                endpoint,
                session,
            }),
            WorkflowLaunchOutcome::Queued {
                queued_launch,
                workflow,
                endpoint,
            } => Ok(LocalDaemonResponse::WorkflowRunQueued {
                queued_launch,
                workflow,
                endpoint,
                session,
            }),
        }
    }

    pub(crate) fn list_workflow_runs(
        &mut self,
        request: ListWorkflowRunsRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        Ok(LocalDaemonResponse::WorkflowRunsListed {
            workflow_runs: self
                .context
                .sessions()
                .list_workflow_runs(&request.session_id, request.workflow_ref.as_deref())?,
        })
    }

    pub(crate) fn get_workflow_run(
        &mut self,
        request: GetWorkflowRunRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        Ok(LocalDaemonResponse::WorkflowRun {
            workflow_run: self
                .context
                .sessions()
                .resolve_workflow_run_ref(&request.session_id, &request.workflow_run_ref)?,
        })
    }

    pub(crate) fn cancel_workflow_run(
        &mut self,
        request: CancelWorkflowRunRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let workflow_run_id = self
            .context
            .sessions()
            .resolve_workflow_run_ref(&request.session_id, &request.workflow_run_ref)?
            .id()
            .to_string();
        let active_prompt_workflow_run_id = self
            .context
            .active_prompt_workflow_run_id(&request.session_id)?;
        let should_cancel_active_prompt =
            active_prompt_workflow_run_id.as_deref() == Some(workflow_run_id.as_str());
        if should_cancel_active_prompt {
            self.context
                .cancel_active_prompt_for_runtime(&request.session_id)?;
        }
        let workflow_run = self
            .context
            .sessions_mut()
            .cancel_workflow_run(&request.session_id, &request.workflow_run_ref)?;
        self.context
            .remove_queued_prompts_by_workflow_run(&request.session_id, &workflow_run_id)?;
        self.context
            .drain_session_workflow_launch_queue(&request.session_id)?;
        let session = self.context.session_snapshot(&request.session_id)?;
        Ok(LocalDaemonResponse::WorkflowRunCancelled {
            workflow_run,
            session,
        })
    }

    pub(crate) fn resume_workflow_run(
        &mut self,
        request: ResumeWorkflowRunRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let workflow_run = self
            .context
            .resume_workflow_run(&request.session_id, &request.workflow_run_ref)?;
        let session = self.context.session_snapshot(&request.session_id)?;
        Ok(LocalDaemonResponse::WorkflowRunResumed {
            workflow_run,
            session,
        })
    }

    pub(crate) fn create_workflow_watchdog(
        &mut self,
        request: CreateWorkflowWatchdogRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let watchdog = self.context.sessions_mut().create_workflow_watchdog(
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
            .context
            .sessions()
            .resolve_workflow_ref(&request.session_id, &request.workflow_ref)?;
        let endpoint = self.context.sessions().resolve_workflow_endpoint_ref(
            &request.session_id,
            &request.workflow_ref,
            &request.endpoint_ref,
        )?;
        let session = self.context.session_snapshot(&request.session_id)?;
        Ok(LocalDaemonResponse::WorkflowWatchdogCreated {
            watchdog,
            workflow,
            endpoint,
            session,
        })
    }

    pub(crate) fn list_workflow_watchdogs(
        &mut self,
        request: ListWorkflowWatchdogsRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        Ok(LocalDaemonResponse::WorkflowWatchdogsListed {
            watchdogs: self
                .context
                .sessions()
                .list_workflow_watchdogs(&request.session_id, request.workflow_ref.as_deref())?,
        })
    }

    pub(crate) fn set_workflow_watchdog_enabled(
        &mut self,
        request: SetWorkflowWatchdogEnabledRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let watchdog = self.context.sessions_mut().set_workflow_watchdog_enabled(
            &request.session_id,
            &request.watchdog_ref,
            request.enabled,
        )?;
        let session = self.context.session_snapshot(&request.session_id)?;
        Ok(LocalDaemonResponse::WorkflowWatchdogUpdated { watchdog, session })
    }

    pub(crate) fn remove_workflow_watchdog(
        &mut self,
        request: RemoveWorkflowWatchdogRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let watchdog = self
            .context
            .sessions_mut()
            .remove_workflow_watchdog(&request.session_id, &request.watchdog_ref)?;
        let session = self.context.session_snapshot(&request.session_id)?;
        Ok(LocalDaemonResponse::WorkflowWatchdogRemoved { watchdog, session })
    }

    pub(crate) fn list_queued_workflow_launches(
        &mut self,
        request: ListQueuedWorkflowLaunchesRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        Ok(LocalDaemonResponse::QueuedWorkflowLaunchesListed {
            queued_launches: self
                .context
                .sessions()
                .list_queued_workflow_launches(&request.session_id)?,
        })
    }

    pub(crate) fn remove_queued_workflow_launch(
        &mut self,
        request: RemoveQueuedWorkflowLaunchRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let queued_launch = self
            .context
            .sessions_mut()
            .remove_queued_workflow_launch(&request.session_id, &request.queue_item_ref)?;
        let session = self.context.session_snapshot(&request.session_id)?;
        Ok(LocalDaemonResponse::QueuedWorkflowLaunchRemoved {
            queued_launch,
            session,
        })
    }

    pub(crate) fn clear_queued_workflow_launches(
        &mut self,
        request: ClearQueuedWorkflowLaunchesRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let queued_launches = self
            .context
            .sessions_mut()
            .clear_queued_workflow_launches(&request.session_id)?;
        let session = self.context.session_snapshot(&request.session_id)?;
        Ok(LocalDaemonResponse::QueuedWorkflowLaunchesCleared {
            queued_launches,
            session,
        })
    }

    pub(crate) fn validate_workflow_output(
        &mut self,
        request: ValidateWorkflowOutputRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let output_schema_ref = request.output_schema_ref.clone();
        let result = self.context.dispatch_runtime_tool_call(
            crate::transport::runtime_tools::RuntimeToolCall {
                tool_name: crate::transport::runtime_tools::VALIDATE_WORKFLOW_OUTPUT_TOOL
                    .to_string(),
                arguments: serde_json::json!({
                    "output_schema_ref": request.output_schema_ref,
                    "output_json": request.output_json,
                }),
                context: crate::transport::runtime_tools::WorkflowRuntimeToolContext {
                    session_id: request.session_id.clone(),
                    workflow_run_ref: String::new(),
                    workflow_node_run_id: String::new(),
                    delivery_token: None,
                    allowed_output_schema_refs: vec![output_schema_ref],
                    workflow_run_output_schema_ref: None,
                    workflow_intermediate_output_schema_ref: None,
                    can_complete_workflow_run: false,
                    can_emit_intermediate_workflow_run_output: false,
                },
            },
        )?;
        Ok(LocalDaemonResponse::WorkflowOutputValidated {
            valid: result.payload["valid"].as_bool().unwrap_or(false),
            warning: result.payload["warning"]
                .as_str()
                .map(str::to_string)
                .filter(|value| !value.is_empty()),
        })
    }

    pub(crate) fn ack_workflow_turn(
        &mut self,
        request: AckWorkflowTurnRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        self.context.dispatch_runtime_tool_call(
            crate::transport::runtime_tools::RuntimeToolCall {
                tool_name: crate::transport::runtime_tools::ACK_WORKFLOW_TURN_TOOL.to_string(),
                arguments: serde_json::json!({
                    "delivery_token": request.delivery_token,
                }),
                context: crate::transport::runtime_tools::WorkflowRuntimeToolContext {
                    session_id: request.session_id.clone(),
                    workflow_run_ref: request.workflow_run_ref.clone(),
                    workflow_node_run_id: request.workflow_node_run_id.clone(),
                    delivery_token: Some(request.delivery_token.clone()),
                    allowed_output_schema_refs: Vec::new(),
                    workflow_run_output_schema_ref: None,
                    workflow_intermediate_output_schema_ref: None,
                    can_complete_workflow_run: false,
                    can_emit_intermediate_workflow_run_output: false,
                },
            },
        )?;
        let workflow_run = self
            .context
            .sessions()
            .resolve_workflow_run_ref(&request.session_id, &request.workflow_run_ref)?
            .clone();
        let session = self.context.session_snapshot(&request.session_id)?;
        Ok(LocalDaemonResponse::WorkflowTurnAcknowledged {
            workflow_run,
            session,
        })
    }
}
