use crate::app::DaemonApp;
use crate::error::DaemonError;
use crate::local::{LocalDaemonRequest, LocalDaemonResponse};
use crate::session::{
    unix_epoch_ms, PromptQueueItem, QueuedWorkflowLaunch, QueuedWorkflowLaunchSource,
    WorkflowConsole, WorkflowConsoleEntry, WorkflowDefinition, WorkflowEndpointDefinition,
    WorkflowLaunchAdmission, WorkflowRun, WorkflowWatchdogTickPlan,
};
use std::collections::BTreeSet;

struct WorkflowProgression;

impl WorkflowProgression {
    fn is_workflow_prompt_attachment(attachment_id: &str) -> bool {
        crate::scheduler::runtime::is_workflow_prompt_attachment(attachment_id)
    }

    fn ensure_provider_run(
        app: &mut DaemonApp,
        session_id: &str,
        agent_id: &str,
    ) -> Result<String, DaemonError> {
        crate::scheduler::runtime::ensure_workflow_provider_run_for_agent(app, session_id, agent_id)
    }

    fn validate_agents(
        app: &DaemonApp,
        session_id: &str,
        workflow: &WorkflowDefinition,
    ) -> Result<(), DaemonError> {
        crate::scheduler::runtime::validate_workflow_agents(app, session_id, workflow)
    }

    fn schedule_entry_node(
        app: &mut DaemonApp,
        session_id: &str,
        workflow_run: &WorkflowRun,
    ) -> Result<(), DaemonError> {
        crate::scheduler::runtime::schedule_workflow_run_entry_node(app, session_id, workflow_run)
    }

    fn on_prompt_started(
        app: &mut DaemonApp,
        session_id: &str,
        prompt: &PromptQueueItem,
    ) -> Result<(), DaemonError> {
        crate::scheduler::runtime::on_workflow_prompt_started(app, session_id, prompt)
    }

    fn on_prompt_completed(
        app: &mut DaemonApp,
        session_id: &str,
        prompt: &PromptQueueItem,
        provider_run_id: Option<&str>,
    ) -> Result<(), DaemonError> {
        crate::scheduler::runtime::on_workflow_prompt_completed(
            app,
            session_id,
            prompt,
            provider_run_id,
        )
    }

    fn on_prompt_cancelled(
        app: &mut DaemonApp,
        session_id: &str,
        prompt: &PromptQueueItem,
    ) -> Result<(), DaemonError> {
        crate::scheduler::runtime::on_workflow_prompt_cancelled(app, session_id, prompt)
    }

    fn retry_blocked_claims(app: &mut DaemonApp) {
        crate::scheduler::runtime::retry_blocked_workflow_claims(app);
    }

    fn resume_run(
        app: &mut DaemonApp,
        session_id: &str,
        workflow_run_ref: &str,
    ) -> Result<WorkflowRun, DaemonError> {
        crate::scheduler::runtime::resume_workflow_run(app, session_id, workflow_run_ref)
    }

    fn read_console(
        app: &DaemonApp,
        session_id: &str,
        workflow_id: &str,
    ) -> Result<WorkflowConsole, DaemonError> {
        crate::scheduler::runtime::read_workflow_console(app, session_id, workflow_id)
    }

    fn write_console(
        app: &mut DaemonApp,
        session_id: &str,
        workflow_id: &str,
        workflow_node_run_id: &str,
        text: &str,
    ) -> Result<WorkflowConsoleEntry, DaemonError> {
        crate::scheduler::runtime::write_workflow_console(
            app,
            session_id,
            workflow_id,
            workflow_node_run_id,
            text,
        )
    }

    fn clear_console(
        app: &mut DaemonApp,
        session_id: &str,
        workflow_id: &str,
    ) -> Result<WorkflowConsole, DaemonError> {
        crate::scheduler::runtime::clear_workflow_console(app, session_id, workflow_id)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorkflowLaunchOutcome {
    Started {
        workflow_run: WorkflowRun,
        workflow: WorkflowDefinition,
        endpoint: WorkflowEndpointDefinition,
    },
    Queued {
        queued_launch: QueuedWorkflowLaunch,
        workflow: WorkflowDefinition,
        endpoint: WorkflowEndpointDefinition,
    },
}

impl DaemonApp {
    pub(crate) fn is_workflow_prompt_source(&self, attachment_id: &str) -> bool {
        WorkflowProgression::is_workflow_prompt_attachment(attachment_id)
    }

    pub(crate) fn start_workflow_prompt_from_runtime(
        &mut self,
        session_id: &str,
        prompt: &PromptQueueItem,
    ) -> Result<(), DaemonError> {
        WorkflowProgression::on_prompt_started(self, session_id, prompt)
    }

    pub(crate) fn complete_workflow_prompt_from_runtime(
        &mut self,
        session_id: &str,
        prompt: &PromptQueueItem,
        provider_run_id: Option<&str>,
    ) -> Result<(), DaemonError> {
        WorkflowProgression::on_prompt_completed(self, session_id, prompt, provider_run_id)
    }

    pub(crate) fn cancel_workflow_prompt_from_runtime(
        &mut self,
        session_id: &str,
        prompt: &PromptQueueItem,
    ) -> Result<(), DaemonError> {
        WorkflowProgression::on_prompt_cancelled(self, session_id, prompt)
    }

    pub(crate) fn ensure_workflow_provider_run_from_runtime(
        &mut self,
        session_id: &str,
        agent_id: &str,
    ) -> Result<String, DaemonError> {
        WorkflowProgression::ensure_provider_run(self, session_id, agent_id)
    }

    pub(crate) fn retry_blocked_workflow_claims_from_runtime(&mut self) {
        WorkflowProgression::retry_blocked_claims(self);
    }

    pub(crate) fn resume_workflow_run_from_runtime(
        &mut self,
        session_id: &str,
        workflow_run_ref: &str,
    ) -> Result<WorkflowRun, DaemonError> {
        WorkflowProgression::resume_run(self, session_id, workflow_run_ref)
    }

    pub(crate) fn read_workflow_console_from_runtime(
        &self,
        session_id: &str,
        workflow_id: &str,
    ) -> Result<WorkflowConsole, DaemonError> {
        WorkflowProgression::read_console(self, session_id, workflow_id)
    }

    pub(crate) fn write_workflow_console_from_runtime(
        &mut self,
        session_id: &str,
        workflow_id: &str,
        workflow_node_run_id: &str,
        text: &str,
    ) -> Result<WorkflowConsoleEntry, DaemonError> {
        WorkflowProgression::write_console(
            self,
            session_id,
            workflow_id,
            workflow_node_run_id,
            text,
        )
    }

    pub(crate) fn clear_workflow_console_from_runtime(
        &mut self,
        session_id: &str,
        workflow_id: &str,
    ) -> Result<WorkflowConsole, DaemonError> {
        WorkflowProgression::clear_console(self, session_id, workflow_id)
    }

    pub(crate) fn handle_workflow_request(
        &mut self,
        request: LocalDaemonRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        match request {
            LocalDaemonRequest::CreateWorkflow(request) => {
                let workflow = self
                    .sessions_mut()
                    .create_workflow(&request.session_id, request.alias)?;
                let session = self.local_api_session_snapshot(&request.session_id)?;
                Ok(LocalDaemonResponse::WorkflowCreated { workflow, session })
            }
            LocalDaemonRequest::AliasWorkflow(request) => {
                let workflow = self.sessions_mut().assign_workflow_alias(
                    &request.session_id,
                    &request.workflow_ref,
                    request.alias,
                )?;
                let session = self.local_api_session_snapshot(&request.session_id)?;
                Ok(LocalDaemonResponse::WorkflowAliased { workflow, session })
            }
            LocalDaemonRequest::ListWorkflows(request) => {
                Ok(LocalDaemonResponse::WorkflowsListed {
                    workflows: self.sessions().list_workflows(&request.session_id)?,
                })
            }
            LocalDaemonRequest::ResolveWorkflow(request) => {
                Ok(LocalDaemonResponse::WorkflowResolved {
                    workflow: self
                        .sessions()
                        .resolve_workflow_ref(&request.session_id, &request.workflow_ref)?,
                })
            }
            LocalDaemonRequest::CreateWorkflowEndpoint(request) => {
                let endpoint = self.sessions_mut().create_workflow_endpoint(
                    &request.session_id,
                    &request.workflow_ref,
                    &request.entry_node_id,
                    request.alias,
                )?;
                let workflow = self
                    .sessions()
                    .resolve_workflow_ref(&request.session_id, &request.workflow_ref)?;
                let session = self.local_api_session_snapshot(&request.session_id)?;
                Ok(LocalDaemonResponse::WorkflowEndpointCreated {
                    endpoint,
                    workflow,
                    session,
                })
            }
            LocalDaemonRequest::AliasWorkflowEndpoint(request) => {
                let endpoint = self.sessions_mut().assign_workflow_endpoint_alias(
                    &request.session_id,
                    &request.workflow_ref,
                    &request.endpoint_ref,
                    request.alias,
                )?;
                let workflow = self
                    .sessions()
                    .resolve_workflow_ref(&request.session_id, &request.workflow_ref)?;
                let session = self.local_api_session_snapshot(&request.session_id)?;
                Ok(LocalDaemonResponse::WorkflowEndpointAliased {
                    endpoint,
                    workflow,
                    session,
                })
            }
            LocalDaemonRequest::BindWorkflowEndpoint(request) => {
                let endpoint = self.sessions_mut().bind_workflow_endpoint(
                    &request.session_id,
                    &request.workflow_ref,
                    &request.endpoint_ref,
                    &request.entry_node_id,
                )?;
                let workflow = self
                    .sessions()
                    .resolve_workflow_ref(&request.session_id, &request.workflow_ref)?;
                let session = self.local_api_session_snapshot(&request.session_id)?;
                Ok(LocalDaemonResponse::WorkflowEndpointBound {
                    endpoint,
                    workflow,
                    session,
                })
            }
            LocalDaemonRequest::AddWorkflowNode(request) => {
                let agent_exists = self
                    .agents()
                    .get_session_agents(&request.session_id)
                    .into_iter()
                    .any(|agent| agent.id() == request.agent_id);
                if !agent_exists {
                    return Err(DaemonError::AgentNotFound {
                        agent_id: request.agent_id,
                    });
                }
                let node = self.sessions_mut().add_workflow_node(
                    &request.session_id,
                    &request.workflow_ref,
                    &request.agent_id,
                )?;
                let workflow = self
                    .sessions()
                    .resolve_workflow_ref(&request.session_id, &request.workflow_ref)?;
                let session = self.local_api_session_snapshot(&request.session_id)?;
                Ok(LocalDaemonResponse::WorkflowNodeAdded {
                    node,
                    workflow,
                    session,
                })
            }
            LocalDaemonRequest::RemoveWorkflowNode(request) => {
                let node = self.sessions_mut().remove_workflow_node(
                    &request.session_id,
                    &request.workflow_ref,
                    &request.node_id,
                )?;
                let workflow = self
                    .sessions()
                    .resolve_workflow_ref(&request.session_id, &request.workflow_ref)?;
                let session = self.local_api_session_snapshot(&request.session_id)?;
                Ok(LocalDaemonResponse::WorkflowNodeRemoved {
                    node,
                    workflow,
                    session,
                })
            }
            LocalDaemonRequest::UpdateWorkflowNodeInstructions(request) => {
                let node = self.sessions_mut().update_workflow_node_instructions(
                    &request.session_id,
                    &request.workflow_ref,
                    &request.node_id,
                    request.instructions.clone(),
                )?;
                let workflow = self
                    .sessions()
                    .resolve_workflow_ref(&request.session_id, &request.workflow_ref)?;
                let session = self.local_api_session_snapshot(&request.session_id)?;
                Ok(LocalDaemonResponse::WorkflowNodeInstructionsUpdated {
                    node,
                    workflow,
                    session,
                })
            }
            LocalDaemonRequest::SetWorkflowNodeCanCompleteRun(request) => {
                let node = self.sessions_mut().set_workflow_node_can_complete_run(
                    &request.session_id,
                    &request.workflow_ref,
                    &request.node_id,
                    request.can_complete_workflow_run,
                )?;
                let workflow = self
                    .sessions()
                    .resolve_workflow_ref(&request.session_id, &request.workflow_ref)?;
                let session = self.local_api_session_snapshot(&request.session_id)?;
                Ok(LocalDaemonResponse::WorkflowNodeCanCompleteRunUpdated {
                    node,
                    workflow,
                    session,
                })
            }
            LocalDaemonRequest::SetWorkflowNodeCanEmitIntermediateOutput(request) => {
                let node = self
                    .sessions_mut()
                    .set_workflow_node_can_emit_intermediate_output(
                        &request.session_id,
                        &request.workflow_ref,
                        &request.node_id,
                        request.can_emit_intermediate_workflow_run_output,
                    )?;
                let workflow = self
                    .sessions()
                    .resolve_workflow_ref(&request.session_id, &request.workflow_ref)?;
                let session = self.local_api_session_snapshot(&request.session_id)?;
                Ok(
                    LocalDaemonResponse::WorkflowNodeCanEmitIntermediateOutputUpdated {
                        node,
                        workflow,
                        session,
                    },
                )
            }
            LocalDaemonRequest::SetWorkflowNodeIntermediateOutputSchema(request) => {
                let node = self
                    .sessions_mut()
                    .set_workflow_node_intermediate_output_schema_ref(
                        &request.session_id,
                        &request.workflow_ref,
                        &request.node_id,
                        request.intermediate_output_schema_ref.clone(),
                    )?;
                let workflow = self
                    .sessions()
                    .resolve_workflow_ref(&request.session_id, &request.workflow_ref)?;
                let session = self.local_api_session_snapshot(&request.session_id)?;
                Ok(
                    LocalDaemonResponse::WorkflowNodeIntermediateOutputSchemaUpdated {
                        node,
                        workflow,
                        session,
                    },
                )
            }
            LocalDaemonRequest::SetWorkflowNodeMaxTurns(request) => {
                let node = self.sessions_mut().set_workflow_node_max_turns(
                    &request.session_id,
                    &request.workflow_ref,
                    &request.node_id,
                    request.max_turns,
                )?;
                let workflow = self
                    .sessions()
                    .resolve_workflow_ref(&request.session_id, &request.workflow_ref)?;
                let session = self.local_api_session_snapshot(&request.session_id)?;
                Ok(LocalDaemonResponse::WorkflowNodeMaxTurnsUpdated {
                    node,
                    workflow,
                    session,
                })
            }
            LocalDaemonRequest::AddWorkflowEdge(request) => {
                let edge = self.sessions_mut().add_workflow_edge(
                    &request.session_id,
                    &request.workflow_ref,
                    &request.from_node_id,
                    &request.to_node_id,
                    request.output_schema_ref.clone(),
                    request.validation_policy,
                )?;
                let workflow = self
                    .sessions()
                    .resolve_workflow_ref(&request.session_id, &request.workflow_ref)?;
                let session = self.local_api_session_snapshot(&request.session_id)?;
                Ok(LocalDaemonResponse::WorkflowEdgeAdded {
                    edge,
                    workflow,
                    session,
                })
            }
            LocalDaemonRequest::RemoveWorkflowEdge(request) => {
                let edge = self.sessions_mut().remove_workflow_edge(
                    &request.session_id,
                    &request.workflow_ref,
                    &request.edge_id,
                )?;
                let workflow = self
                    .sessions()
                    .resolve_workflow_ref(&request.session_id, &request.workflow_ref)?;
                let session = self.local_api_session_snapshot(&request.session_id)?;
                Ok(LocalDaemonResponse::WorkflowEdgeRemoved {
                    edge,
                    workflow,
                    session,
                })
            }
            LocalDaemonRequest::InvokeWorkflowEndpoint(request) => {
                let outcome = self.invoke_workflow_endpoint_with_admission(
                    &request.session_id,
                    &request.workflow_ref,
                    &request.endpoint_ref,
                    request.prompt,
                )?;
                let session = self.local_api_session_snapshot(&request.session_id)?;
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
            LocalDaemonRequest::ListWorkflowRuns(request) => {
                Ok(LocalDaemonResponse::WorkflowRunsListed {
                    workflow_runs: self
                        .sessions()
                        .list_workflow_runs(&request.session_id, request.workflow_ref.as_deref())?,
                })
            }
            LocalDaemonRequest::GetWorkflowRun(request) => Ok(LocalDaemonResponse::WorkflowRun {
                workflow_run: self
                    .sessions()
                    .resolve_workflow_run_ref(&request.session_id, &request.workflow_run_ref)?,
            }),
            LocalDaemonRequest::CancelWorkflowRun(request) => {
                let workflow_run_id = self
                    .sessions()
                    .resolve_workflow_run_ref(&request.session_id, &request.workflow_run_ref)?
                    .id()
                    .to_string();
                let active_prompt_workflow_run_id = if let Some(agent_id) =
                    self.prompt_owner_active_prompt_agent_id(&request.session_id)?
                {
                    self.prompt_owner_active_prompt_for_agent(&request.session_id, &agent_id)?
                        .and_then(|prompt| prompt.workflow_run_id().map(str::to_string))
                } else {
                    None
                };
                let should_cancel_active_prompt =
                    active_prompt_workflow_run_id.as_deref() == Some(workflow_run_id.as_str());
                if should_cancel_active_prompt {
                    let _ = crate::transport::TransportService::cancel_active_prompt_for_runtime(
                        self,
                        &request.session_id,
                    )?;
                }
                let workflow_run = self
                    .sessions_mut()
                    .cancel_workflow_run(&request.session_id, &request.workflow_run_ref)?;
                let _ = self.prompt_owner_remove_queued_prompts_by_workflow_run(
                    &request.session_id,
                    &workflow_run_id,
                )?;
                let _ = self.drain_session_workflow_launch_queue(&request.session_id)?;
                let session = self.local_api_session_snapshot(&request.session_id)?;
                Ok(LocalDaemonResponse::WorkflowRunCancelled {
                    workflow_run,
                    session,
                })
            }
            LocalDaemonRequest::ResumeWorkflowRun(request) => {
                let workflow_run = self.resume_workflow_run_from_runtime(
                    &request.session_id,
                    &request.workflow_run_ref,
                )?;
                let session = self.local_api_session_snapshot(&request.session_id)?;
                Ok(LocalDaemonResponse::WorkflowRunResumed {
                    workflow_run,
                    session,
                })
            }
            LocalDaemonRequest::CreateWorkflowWatchdog(request) => {
                let watchdog = self.sessions_mut().create_workflow_watchdog(
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
                    .sessions()
                    .resolve_workflow_ref(&request.session_id, &request.workflow_ref)?;
                let endpoint = self.sessions().resolve_workflow_endpoint_ref(
                    &request.session_id,
                    &request.workflow_ref,
                    &request.endpoint_ref,
                )?;
                let session = self.local_api_session_snapshot(&request.session_id)?;
                Ok(LocalDaemonResponse::WorkflowWatchdogCreated {
                    watchdog,
                    workflow,
                    endpoint,
                    session,
                })
            }
            LocalDaemonRequest::ListWorkflowWatchdogs(request) => {
                Ok(LocalDaemonResponse::WorkflowWatchdogsListed {
                    watchdogs: self.sessions().list_workflow_watchdogs(
                        &request.session_id,
                        request.workflow_ref.as_deref(),
                    )?,
                })
            }
            LocalDaemonRequest::SetWorkflowWatchdogEnabled(request) => {
                let watchdog = self.sessions_mut().set_workflow_watchdog_enabled(
                    &request.session_id,
                    &request.watchdog_ref,
                    request.enabled,
                )?;
                let session = self.local_api_session_snapshot(&request.session_id)?;
                Ok(LocalDaemonResponse::WorkflowWatchdogUpdated { watchdog, session })
            }
            LocalDaemonRequest::RemoveWorkflowWatchdog(request) => {
                let watchdog = self
                    .sessions_mut()
                    .remove_workflow_watchdog(&request.session_id, &request.watchdog_ref)?;
                let session = self.local_api_session_snapshot(&request.session_id)?;
                Ok(LocalDaemonResponse::WorkflowWatchdogRemoved { watchdog, session })
            }
            LocalDaemonRequest::SetWorkflowFlushContext(request) => {
                let workflow = self
                    .sessions_mut()
                    .set_workflow_flush_agent_context_before_run(
                        &request.session_id,
                        &request.workflow_ref,
                        request.flush_agent_context_before_run,
                    )?;
                let session = self.local_api_session_snapshot(&request.session_id)?;
                Ok(LocalDaemonResponse::WorkflowFlushContextUpdated { workflow, session })
            }
            LocalDaemonRequest::SetWorkflowRunOutputSchema(request) => {
                let workflow = self.sessions_mut().set_workflow_run_output_schema_ref(
                    &request.session_id,
                    &request.workflow_ref,
                    request.run_output_schema_ref.clone(),
                )?;
                let session = self.local_api_session_snapshot(&request.session_id)?;
                Ok(LocalDaemonResponse::WorkflowRunOutputSchemaUpdated { workflow, session })
            }
            LocalDaemonRequest::SetWorkflowIntermediateOutputSchema(request) => {
                let workflow = self
                    .sessions_mut()
                    .set_workflow_intermediate_output_schema_ref(
                        &request.session_id,
                        &request.workflow_ref,
                        request.intermediate_output_schema_ref.clone(),
                    )?;
                let session = self.local_api_session_snapshot(&request.session_id)?;
                Ok(
                    LocalDaemonResponse::WorkflowIntermediateOutputSchemaUpdated {
                        workflow,
                        session,
                    },
                )
            }
            LocalDaemonRequest::SetWorkflowLaunchPolicy(request) => {
                let session = self
                    .sessions_mut()
                    .set_workflow_launch_policy(&request.session_id, request.policy)?;
                let mut session = session;
                session.set_agents(self.agents().get_session_agents(&request.session_id));
                Ok(LocalDaemonResponse::WorkflowLaunchPolicyUpdated { session })
            }
            LocalDaemonRequest::ListQueuedWorkflowLaunches(request) => {
                Ok(LocalDaemonResponse::QueuedWorkflowLaunchesListed {
                    queued_launches: self
                        .sessions()
                        .list_queued_workflow_launches(&request.session_id)?,
                })
            }
            LocalDaemonRequest::RemoveQueuedWorkflowLaunch(request) => {
                let queued_launch = self
                    .sessions_mut()
                    .remove_queued_workflow_launch(&request.session_id, &request.queue_item_ref)?;
                let session = self.local_api_session_snapshot(&request.session_id)?;
                Ok(LocalDaemonResponse::QueuedWorkflowLaunchRemoved {
                    queued_launch,
                    session,
                })
            }
            LocalDaemonRequest::ClearQueuedWorkflowLaunches(request) => {
                let queued_launches = self
                    .sessions_mut()
                    .clear_queued_workflow_launches(&request.session_id)?;
                let session = self.local_api_session_snapshot(&request.session_id)?;
                Ok(LocalDaemonResponse::QueuedWorkflowLaunchesCleared {
                    queued_launches,
                    session,
                })
            }
            LocalDaemonRequest::ValidateWorkflowOutput(request) => {
                let result = crate::transport::runtime_tools::dispatch_runtime_tool_call(
                    self,
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
                            allowed_output_schema_refs: vec![request.output_schema_ref.clone()],
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
            LocalDaemonRequest::AckWorkflowTurn(request) => {
                crate::transport::runtime_tools::dispatch_runtime_tool_call(
                    self,
                    crate::transport::runtime_tools::RuntimeToolCall {
                        tool_name: crate::transport::runtime_tools::ACK_WORKFLOW_TURN_TOOL
                            .to_string(),
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
                    .sessions()
                    .resolve_workflow_run_ref(&request.session_id, &request.workflow_run_ref)?
                    .clone();
                let session = self.local_api_session_snapshot(&request.session_id)?;
                Ok(LocalDaemonResponse::WorkflowTurnAcknowledged {
                    workflow_run,
                    session,
                })
            }
            _ => Err(DaemonError::LocalTransport {
                operation: "execute workflow request",
                message: "request is not handled by the workflow runtime".to_string(),
            }),
        }
    }

    pub fn invoke_workflow_endpoint_with_admission(
        &mut self,
        session_id: &str,
        workflow_ref: &str,
        endpoint_ref: &str,
        prompt: Option<String>,
    ) -> Result<WorkflowLaunchOutcome, DaemonError> {
        let workflow = self
            .sessions()
            .resolve_workflow_ref(session_id, workflow_ref)?;
        let endpoint = self.sessions().resolve_workflow_endpoint_ref(
            session_id,
            workflow_ref,
            endpoint_ref,
        )?;
        WorkflowProgression::validate_agents(self, session_id, &workflow)?;
        match self.sessions_mut().admit_manual_workflow_launch(
            session_id,
            workflow.id(),
            endpoint.id(),
            prompt.clone(),
        )? {
            WorkflowLaunchAdmission::StartNow => {
                self.flush_workflow_agent_context_if_needed(session_id, &workflow)?;
                let workflow_run = self.sessions_mut().invoke_workflow_endpoint(
                    session_id,
                    workflow.id(),
                    endpoint.id(),
                    prompt,
                )?;
                WorkflowProgression::schedule_entry_node(self, session_id, &workflow_run)?;
                let workflow_run = self
                    .sessions()
                    .resolve_workflow_run_ref(session_id, workflow_run.id())?;
                Ok(WorkflowLaunchOutcome::Started {
                    workflow_run,
                    workflow,
                    endpoint,
                })
            }
            WorkflowLaunchAdmission::Queued(queued_launch) => Ok(WorkflowLaunchOutcome::Queued {
                queued_launch,
                workflow,
                endpoint,
            }),
        }
    }

    pub fn invoke_workflow_endpoint_and_schedule(
        &mut self,
        session_id: &str,
        workflow_ref: &str,
        endpoint_ref: &str,
        prompt: Option<String>,
    ) -> Result<(WorkflowRun, WorkflowDefinition, WorkflowEndpointDefinition), DaemonError> {
        match self.invoke_workflow_endpoint_with_admission(
            session_id,
            workflow_ref,
            endpoint_ref,
            prompt,
        )? {
            WorkflowLaunchOutcome::Started {
                workflow_run,
                workflow,
                endpoint,
            } => Ok((workflow_run, workflow, endpoint)),
            WorkflowLaunchOutcome::Queued {
                workflow, endpoint, ..
            } => Err(DaemonError::WorkflowLaunchRejected {
                session_id: session_id.to_string(),
                workflow_id: workflow.id().to_string(),
                endpoint_id: endpoint.id().to_string(),
                message: "workflow launch was queued instead of started".to_string(),
            }),
        }
    }

    pub fn drain_session_workflow_launch_queue(
        &mut self,
        session_id: &str,
    ) -> Result<Option<WorkflowLaunchOutcome>, DaemonError> {
        let Some(queued_launch) = self
            .sessions_mut()
            .dequeue_next_workflow_launch(session_id)?
        else {
            return Ok(None);
        };
        if let Some(watchdog_id) = queued_launch.watchdog_id() {
            let _ = self
                .sessions_mut()
                .mark_workflow_watchdog_pending_started(session_id, watchdog_id);
        }
        let outcome = self.invoke_queued_workflow_launch(session_id, queued_launch.clone());
        match outcome {
            Ok(outcome) => Ok(Some(outcome)),
            Err(error) => {
                if let Some(watchdog_id) = queued_launch.watchdog_id() {
                    let _ = self.sessions_mut().mark_workflow_watchdog_failed(
                        session_id,
                        watchdog_id,
                        error.to_string(),
                    );
                }
                self.record_notice(
                    session_id,
                    None,
                    self.attachments().list_session_attachment_ids(session_id),
                    format!(
                        "Queued workflow launch `{}` failed: {}",
                        queued_launch.id(),
                        error
                    ),
                );
                Ok(None)
            }
        }
    }

    pub fn pump_workflow_watchdogs(&mut self) {
        let plans = match self
            .sessions_mut()
            .collect_due_workflow_watchdog_invocations(unix_epoch_ms())
        {
            Ok(plans) => plans,
            Err(error) => {
                crate::logging::warn_with_fields(
                    "daemon.app",
                    "workflow watchdog collection failed",
                    serde_json::json!({ "error": error.to_string() }),
                );
                return;
            }
        };
        for plan in plans {
            match self.invoke_watchdog_workflow_launch(plan) {
                Ok(()) => {}
                Err(error) => {
                    crate::logging::warn_with_fields(
                        "daemon.app",
                        "workflow watchdog invoke failed",
                        serde_json::json!({ "error": error.to_string() }),
                    );
                }
            }
        }
    }

    fn invoke_watchdog_workflow_launch(
        &mut self,
        plan: WorkflowWatchdogTickPlan,
    ) -> Result<(), DaemonError> {
        match self.invoke_queued_workflow_launch(
            &plan.session_id,
            QueuedWorkflowLaunch::new(
                format!("watchdog-launch-{}", plan.watchdog_id),
                plan.workflow_id.clone(),
                plan.endpoint_id.clone(),
                Some(plan.invocation_prompt.clone()),
                QueuedWorkflowLaunchSource::Watchdog,
                Some(plan.watchdog_id.clone()),
            ),
        ) {
            Ok(WorkflowLaunchOutcome::Started { .. }) => Ok(()),
            Ok(WorkflowLaunchOutcome::Queued { .. }) => Ok(()),
            Err(error) => {
                let _ = self.sessions_mut().mark_workflow_watchdog_failed(
                    &plan.session_id,
                    &plan.watchdog_id,
                    error.to_string(),
                );
                Err(error)
            }
        }
    }

    fn invoke_queued_workflow_launch(
        &mut self,
        session_id: &str,
        queued_launch: QueuedWorkflowLaunch,
    ) -> Result<WorkflowLaunchOutcome, DaemonError> {
        let workflow = self
            .sessions()
            .resolve_workflow_ref(session_id, queued_launch.workflow_id())?;
        let endpoint = self.sessions().resolve_workflow_endpoint_ref(
            session_id,
            queued_launch.workflow_id(),
            queued_launch.endpoint_id(),
        )?;
        WorkflowProgression::validate_agents(self, session_id, &workflow)?;
        self.flush_workflow_agent_context_if_needed(session_id, &workflow)?;
        let workflow_run = self.sessions_mut().invoke_workflow_endpoint(
            session_id,
            workflow.id(),
            endpoint.id(),
            queued_launch.invocation_prompt().map(str::to_string),
        )?;
        WorkflowProgression::schedule_entry_node(self, session_id, &workflow_run)?;
        let workflow_run = self
            .sessions()
            .resolve_workflow_run_ref(session_id, workflow_run.id())?;
        if let Some(watchdog_id) = queued_launch.watchdog_id() {
            let _ = self.sessions_mut().mark_workflow_watchdog_invoked(
                session_id,
                watchdog_id,
                workflow_run.id(),
            );
        }
        Ok(WorkflowLaunchOutcome::Started {
            workflow_run,
            workflow,
            endpoint,
        })
    }

    pub(crate) fn flush_workflow_agent_context_if_needed(
        &mut self,
        session_id: &str,
        workflow: &WorkflowDefinition,
    ) -> Result<(), DaemonError> {
        if !workflow.flush_agent_context_before_run() {
            return Ok(());
        }
        let workflow_agent_ids = workflow
            .nodes()
            .iter()
            .map(|node| node.agent_id().to_string())
            .collect::<BTreeSet<_>>();
        if workflow_agent_ids.is_empty() {
            return Ok(());
        }
        let should_cancel_active_prompt = self
            .sessions()
            .get_session(session_id)?
            .active_prompt()
            .map(|prompt| prompt.target_agent_id())
            .is_some_and(|agent_id| workflow_agent_ids.contains(agent_id));
        if should_cancel_active_prompt {
            let _ = crate::transport::TransportService::cancel_active_prompt_for_runtime(
                self, session_id,
            )?;
        }
        for agent_id in workflow_agent_ids {
            if let Some(run) = self.providers().get_run_for_agent(session_id, &agent_id) {
                if run.state() == crate::provider::ProviderRunState::Ended {
                    continue;
                }
                let run = self
                    .providers
                    .terminate_run(&mut self.sessions, session_id, run.id())?;
                let _ = self.remove_tracked_provider_process_for_run(run.id());
            }
        }
        Ok(())
    }
}
