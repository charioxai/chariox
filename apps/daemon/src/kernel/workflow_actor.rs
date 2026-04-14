use std::collections::HashMap;
use std::sync::Arc;

use tokio::sync::{mpsc, oneshot, Mutex};

use crate::app::workflow_runtime::WorkflowLaunchOutcome;
use crate::app::DaemonApp;
use crate::error::DaemonError;
use crate::kernel::projection::{
    ActorQueueSnapshot, AgentRuntimeProjectionStore, SessionStateProjectionStore,
};
use crate::local::{LocalDaemonRequest, LocalDaemonResponse};

pub(crate) const WORKFLOW_COMMAND_QUEUE_LIMIT: usize = 128;

#[derive(Debug)]
struct WorkflowCommandEnvelope {
    command_id: String,
    command_type: String,
    request: LocalDaemonRequest,
    result_tx: oneshot::Sender<Result<LocalDaemonResponse, DaemonError>>,
}

#[derive(Clone)]
pub(crate) struct WorkflowRuntime {
    store: WorkflowRuntimeStore,
    queue_limit: usize,
    session_projection: SessionStateProjectionStore,
    agent_runtime_projection: AgentRuntimeProjectionStore,
    lanes: Arc<Mutex<HashMap<String, mpsc::Sender<WorkflowCommandEnvelope>>>>,
}

impl WorkflowRuntime {
    pub(crate) fn new(
        app: Arc<Mutex<DaemonApp>>,
        session_projection: SessionStateProjectionStore,
        agent_runtime_projection: AgentRuntimeProjectionStore,
    ) -> Self {
        Self::with_store(
            WorkflowRuntimeStore::new(app),
            session_projection,
            agent_runtime_projection,
        )
    }

    pub(crate) fn with_store(
        store: WorkflowRuntimeStore,
        session_projection: SessionStateProjectionStore,
        agent_runtime_projection: AgentRuntimeProjectionStore,
    ) -> Self {
        Self {
            store,
            queue_limit: WORKFLOW_COMMAND_QUEUE_LIMIT,
            session_projection,
            agent_runtime_projection,
            lanes: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub(crate) async fn dispatch_workflow_command(
        &self,
        command: crate::kernel::command::KernelCommand,
        request: LocalDaemonRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let session_id = self.resolve_workflow_lane_key(&request)?;
        let lane = self.workflow_lane(&session_id).await;
        let (result_tx, result_rx) = oneshot::channel();
        lane.try_send(WorkflowCommandEnvelope {
            command_id: command.command_id,
            command_type: command.command_type,
            request,
            result_tx,
        })
        .map_err(|error| DaemonError::LocalTransport {
            operation: "enqueue workflow kernel command",
            message: format!("workflow command lane overloaded: {error}"),
        })?;
        result_rx
            .await
            .map_err(|error| DaemonError::LocalTransport {
                operation: "await workflow kernel command",
                message: error.to_string(),
            })?
    }

    fn resolve_workflow_lane_key(
        &self,
        request: &LocalDaemonRequest,
    ) -> Result<String, DaemonError> {
        let session_id =
            workflow_session_id(request).ok_or_else(|| DaemonError::LocalTransport {
                operation: "route workflow kernel command",
                message: "request is not handled by the workflow runtime".to_string(),
            })?;
        if self.session_projection.get(&session_id).is_some()
            || !self.session_projection.has_warmed_list()
        {
            return Ok(session_id);
        }
        Err(DaemonError::SessionNotFound { session_id })
    }

    async fn workflow_lane(&self, session_id: &str) -> mpsc::Sender<WorkflowCommandEnvelope> {
        let mut lanes = self.lanes.lock().await;
        if let Some(lane) = lanes.get(session_id) {
            return lane.clone();
        }
        let (tx, rx) = mpsc::channel(self.queue_limit);
        lanes.insert(session_id.to_string(), tx.clone());
        tokio::spawn(run_workflow_command_lane(
            self.store.clone(),
            self.session_projection.clone(),
            self.agent_runtime_projection.clone(),
            session_id.to_string(),
            rx,
        ));
        tx
    }

    pub(crate) async fn remove_session_lane(&self, session_id: &str) {
        self.lanes.lock().await.remove(session_id);
    }

    #[allow(dead_code)]
    pub(crate) async fn queue_snapshots(&self) -> Vec<ActorQueueSnapshot> {
        let lanes = self.lanes.lock().await;
        let mut snapshots = lanes
            .iter()
            .map(|(session_id, sender)| {
                ActorQueueSnapshot::new(
                    session_id.clone(),
                    self.queue_limit,
                    self.queue_limit.saturating_sub(sender.capacity()),
                )
            })
            .collect::<Vec<_>>();
        snapshots.sort_by(|left, right| left.lane_id.cmp(&right.lane_id));
        snapshots
    }

    #[cfg(test)]
    pub(crate) async fn has_lane(&self, session_id: &str) -> bool {
        self.lanes.lock().await.contains_key(session_id)
    }
}

#[derive(Clone)]
pub(crate) struct WorkflowRuntimeStore {
    app: Arc<Mutex<DaemonApp>>,
}

impl WorkflowRuntimeStore {
    pub(crate) fn new(app: Arc<Mutex<DaemonApp>>) -> Self {
        Self { app }
    }

    async fn execute_operation(
        &self,
        session_id: &str,
        operation: impl FnOnce(&mut DaemonApp) -> Result<LocalDaemonResponse, DaemonError>,
    ) -> (
        Result<LocalDaemonResponse, DaemonError>,
        Option<crate::session::RuntimeSession>,
    ) {
        let mut app = self.app.lock().await;
        let result = operation(&mut app);
        let projected_session = if let Ok(response) = result.as_ref() {
            workflow_response_session(response)
                .or_else(|| app.local_api_session_snapshot(session_id).ok())
        } else {
            None
        };
        (result, projected_session)
    }
}

async fn run_workflow_command_lane(
    store: WorkflowRuntimeStore,
    session_projection: SessionStateProjectionStore,
    agent_runtime_projection: AgentRuntimeProjectionStore,
    session_id: String,
    mut rx: mpsc::Receiver<WorkflowCommandEnvelope>,
) {
    let executor = WorkflowRuntimeCommandExecutor::new(
        store,
        session_projection,
        agent_runtime_projection,
        session_id.clone(),
    );
    while let Some(envelope) = rx.recv().await {
        crate::logging::info_with_fields(
            "daemon.kernel_workflow_actor",
            "workflow kernel command dispatched",
            serde_json::json!({
                "session_id": session_id,
                "command_id": envelope.command_id,
                "command_type": envelope.command_type,
            }),
        );
        let result = executor.execute(envelope.request).await;
        let _ = envelope.result_tx.send(result);
    }
}

#[derive(Clone)]
struct WorkflowRuntimeCommandExecutor {
    store: WorkflowRuntimeStore,
    session_projection: SessionStateProjectionStore,
    agent_runtime_projection: AgentRuntimeProjectionStore,
    session_id: String,
}

impl WorkflowRuntimeCommandExecutor {
    fn new(
        store: WorkflowRuntimeStore,
        session_projection: SessionStateProjectionStore,
        agent_runtime_projection: AgentRuntimeProjectionStore,
        session_id: String,
    ) -> Self {
        Self {
            store,
            session_projection,
            agent_runtime_projection,
            session_id,
        }
    }

    async fn execute(
        &self,
        request: LocalDaemonRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let (result, projected_session) = self
            .store
            .execute_operation(&self.session_id, move |app| {
                execute_workflow_command_mutation(app, request)
            })
            .await;
        if let Some(session) = projected_session {
            self.agent_runtime_projection.update_session(&session);
            self.session_projection.update(session);
        }
        result
    }
}

fn execute_workflow_command_mutation(
    app: &mut DaemonApp,
    request: LocalDaemonRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    match request {
        LocalDaemonRequest::CreateWorkflow(request) => {
            let workflow = app
                .sessions_mut()
                .create_workflow(&request.session_id, request.alias)?;
            let session = app.local_api_session_snapshot(&request.session_id)?;
            Ok(LocalDaemonResponse::WorkflowCreated { workflow, session })
        }
        LocalDaemonRequest::AliasWorkflow(request) => {
            let workflow = app.sessions_mut().assign_workflow_alias(
                &request.session_id,
                &request.workflow_ref,
                request.alias,
            )?;
            let session = app.local_api_session_snapshot(&request.session_id)?;
            Ok(LocalDaemonResponse::WorkflowAliased { workflow, session })
        }
        LocalDaemonRequest::ListWorkflows(request) => Ok(LocalDaemonResponse::WorkflowsListed {
            workflows: app.sessions().list_workflows(&request.session_id)?,
        }),
        LocalDaemonRequest::ResolveWorkflow(request) => Ok(LocalDaemonResponse::WorkflowResolved {
            workflow: app
                .sessions()
                .resolve_workflow_ref(&request.session_id, &request.workflow_ref)?,
        }),
        LocalDaemonRequest::CreateWorkflowEndpoint(request) => {
            let endpoint = app.sessions_mut().create_workflow_endpoint(
                &request.session_id,
                &request.workflow_ref,
                &request.entry_node_id,
                request.alias,
            )?;
            let workflow = app
                .sessions()
                .resolve_workflow_ref(&request.session_id, &request.workflow_ref)?;
            let session = app.local_api_session_snapshot(&request.session_id)?;
            Ok(LocalDaemonResponse::WorkflowEndpointCreated {
                endpoint,
                workflow,
                session,
            })
        }
        LocalDaemonRequest::AliasWorkflowEndpoint(request) => {
            let endpoint = app.sessions_mut().assign_workflow_endpoint_alias(
                &request.session_id,
                &request.workflow_ref,
                &request.endpoint_ref,
                request.alias,
            )?;
            let workflow = app
                .sessions()
                .resolve_workflow_ref(&request.session_id, &request.workflow_ref)?;
            let session = app.local_api_session_snapshot(&request.session_id)?;
            Ok(LocalDaemonResponse::WorkflowEndpointAliased {
                endpoint,
                workflow,
                session,
            })
        }
        LocalDaemonRequest::BindWorkflowEndpoint(request) => {
            let endpoint = app.sessions_mut().bind_workflow_endpoint(
                &request.session_id,
                &request.workflow_ref,
                &request.endpoint_ref,
                &request.entry_node_id,
            )?;
            let workflow = app
                .sessions()
                .resolve_workflow_ref(&request.session_id, &request.workflow_ref)?;
            let session = app.local_api_session_snapshot(&request.session_id)?;
            Ok(LocalDaemonResponse::WorkflowEndpointBound {
                endpoint,
                workflow,
                session,
            })
        }
        LocalDaemonRequest::AddWorkflowNode(request) => {
            let agent_exists = app
                .agents()
                .get_session_agents(&request.session_id)
                .into_iter()
                .any(|agent| agent.id() == request.agent_id);
            if !agent_exists {
                return Err(DaemonError::AgentNotFound {
                    agent_id: request.agent_id,
                });
            }
            let node = app.sessions_mut().add_workflow_node(
                &request.session_id,
                &request.workflow_ref,
                &request.agent_id,
            )?;
            let workflow = app
                .sessions()
                .resolve_workflow_ref(&request.session_id, &request.workflow_ref)?;
            let session = app.local_api_session_snapshot(&request.session_id)?;
            Ok(LocalDaemonResponse::WorkflowNodeAdded {
                node,
                workflow,
                session,
            })
        }
        LocalDaemonRequest::RemoveWorkflowNode(request) => {
            let node = app.sessions_mut().remove_workflow_node(
                &request.session_id,
                &request.workflow_ref,
                &request.node_id,
            )?;
            let workflow = app
                .sessions()
                .resolve_workflow_ref(&request.session_id, &request.workflow_ref)?;
            let session = app.local_api_session_snapshot(&request.session_id)?;
            Ok(LocalDaemonResponse::WorkflowNodeRemoved {
                node,
                workflow,
                session,
            })
        }
        LocalDaemonRequest::UpdateWorkflowNodeInstructions(request) => {
            let node = app.sessions_mut().update_workflow_node_instructions(
                &request.session_id,
                &request.workflow_ref,
                &request.node_id,
                request.instructions.clone(),
            )?;
            let workflow = app
                .sessions()
                .resolve_workflow_ref(&request.session_id, &request.workflow_ref)?;
            let session = app.local_api_session_snapshot(&request.session_id)?;
            Ok(LocalDaemonResponse::WorkflowNodeInstructionsUpdated {
                node,
                workflow,
                session,
            })
        }
        LocalDaemonRequest::SetWorkflowNodeCanCompleteRun(request) => {
            let node = app.sessions_mut().set_workflow_node_can_complete_run(
                &request.session_id,
                &request.workflow_ref,
                &request.node_id,
                request.can_complete_workflow_run,
            )?;
            let workflow = app
                .sessions()
                .resolve_workflow_ref(&request.session_id, &request.workflow_ref)?;
            let session = app.local_api_session_snapshot(&request.session_id)?;
            Ok(LocalDaemonResponse::WorkflowNodeCanCompleteRunUpdated {
                node,
                workflow,
                session,
            })
        }
        LocalDaemonRequest::SetWorkflowNodeCanEmitIntermediateOutput(request) => {
            let node = app
                .sessions_mut()
                .set_workflow_node_can_emit_intermediate_output(
                    &request.session_id,
                    &request.workflow_ref,
                    &request.node_id,
                    request.can_emit_intermediate_workflow_run_output,
                )?;
            let workflow = app
                .sessions()
                .resolve_workflow_ref(&request.session_id, &request.workflow_ref)?;
            let session = app.local_api_session_snapshot(&request.session_id)?;
            Ok(
                LocalDaemonResponse::WorkflowNodeCanEmitIntermediateOutputUpdated {
                    node,
                    workflow,
                    session,
                },
            )
        }
        LocalDaemonRequest::SetWorkflowNodeIntermediateOutputSchema(request) => {
            let node = app
                .sessions_mut()
                .set_workflow_node_intermediate_output_schema_ref(
                    &request.session_id,
                    &request.workflow_ref,
                    &request.node_id,
                    request.intermediate_output_schema_ref.clone(),
                )?;
            let workflow = app
                .sessions()
                .resolve_workflow_ref(&request.session_id, &request.workflow_ref)?;
            let session = app.local_api_session_snapshot(&request.session_id)?;
            Ok(
                LocalDaemonResponse::WorkflowNodeIntermediateOutputSchemaUpdated {
                    node,
                    workflow,
                    session,
                },
            )
        }
        LocalDaemonRequest::SetWorkflowNodeMaxTurns(request) => {
            let node = app.sessions_mut().set_workflow_node_max_turns(
                &request.session_id,
                &request.workflow_ref,
                &request.node_id,
                request.max_turns,
            )?;
            let workflow = app
                .sessions()
                .resolve_workflow_ref(&request.session_id, &request.workflow_ref)?;
            let session = app.local_api_session_snapshot(&request.session_id)?;
            Ok(LocalDaemonResponse::WorkflowNodeMaxTurnsUpdated {
                node,
                workflow,
                session,
            })
        }
        LocalDaemonRequest::AddWorkflowEdge(request) => {
            let edge = app.sessions_mut().add_workflow_edge(
                &request.session_id,
                &request.workflow_ref,
                &request.from_node_id,
                &request.to_node_id,
                request.output_schema_ref.clone(),
                request.validation_policy,
            )?;
            let workflow = app
                .sessions()
                .resolve_workflow_ref(&request.session_id, &request.workflow_ref)?;
            let session = app.local_api_session_snapshot(&request.session_id)?;
            Ok(LocalDaemonResponse::WorkflowEdgeAdded {
                edge,
                workflow,
                session,
            })
        }
        LocalDaemonRequest::RemoveWorkflowEdge(request) => {
            let edge = app.sessions_mut().remove_workflow_edge(
                &request.session_id,
                &request.workflow_ref,
                &request.edge_id,
            )?;
            let workflow = app
                .sessions()
                .resolve_workflow_ref(&request.session_id, &request.workflow_ref)?;
            let session = app.local_api_session_snapshot(&request.session_id)?;
            Ok(LocalDaemonResponse::WorkflowEdgeRemoved {
                edge,
                workflow,
                session,
            })
        }
        LocalDaemonRequest::InvokeWorkflowEndpoint(request) => {
            let outcome = app.invoke_workflow_endpoint_with_admission(
                &request.session_id,
                &request.workflow_ref,
                &request.endpoint_ref,
                request.prompt,
            )?;
            let session = app.local_api_session_snapshot(&request.session_id)?;
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
                workflow_runs: app
                    .sessions()
                    .list_workflow_runs(&request.session_id, request.workflow_ref.as_deref())?,
            })
        }
        LocalDaemonRequest::GetWorkflowRun(request) => Ok(LocalDaemonResponse::WorkflowRun {
            workflow_run: app
                .sessions()
                .resolve_workflow_run_ref(&request.session_id, &request.workflow_run_ref)?,
        }),
        LocalDaemonRequest::CancelWorkflowRun(request) => {
            let workflow_run_id = app
                .sessions()
                .resolve_workflow_run_ref(&request.session_id, &request.workflow_run_ref)?
                .id()
                .to_string();
            let active_prompt_workflow_run_id = if let Some(agent_id) =
                app.prompt_owner_active_prompt_agent_id(&request.session_id)?
            {
                app.prompt_owner_active_prompt_for_agent(&request.session_id, &agent_id)?
                    .and_then(|prompt| prompt.workflow_run_id().map(str::to_string))
            } else {
                None
            };
            let should_cancel_active_prompt =
                active_prompt_workflow_run_id.as_deref() == Some(workflow_run_id.as_str());
            if should_cancel_active_prompt {
                let _ = crate::transport::TransportService::cancel_active_prompt_for_runtime(
                    app,
                    &request.session_id,
                )?;
            }
            let workflow_run = app
                .sessions_mut()
                .cancel_workflow_run(&request.session_id, &request.workflow_run_ref)?;
            let _ = app.prompt_owner_remove_queued_prompts_by_workflow_run(
                &request.session_id,
                &workflow_run_id,
            )?;
            let _ = app.drain_session_workflow_launch_queue(&request.session_id)?;
            let session = app.local_api_session_snapshot(&request.session_id)?;
            Ok(LocalDaemonResponse::WorkflowRunCancelled {
                workflow_run,
                session,
            })
        }
        LocalDaemonRequest::ResumeWorkflowRun(request) => {
            let workflow_run = app
                .resume_workflow_run_from_runtime(&request.session_id, &request.workflow_run_ref)?;
            let session = app.local_api_session_snapshot(&request.session_id)?;
            Ok(LocalDaemonResponse::WorkflowRunResumed {
                workflow_run,
                session,
            })
        }
        LocalDaemonRequest::CreateWorkflowWatchdog(request) => {
            let watchdog = app.sessions_mut().create_workflow_watchdog(
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
            let workflow = app
                .sessions()
                .resolve_workflow_ref(&request.session_id, &request.workflow_ref)?;
            let endpoint = app.sessions().resolve_workflow_endpoint_ref(
                &request.session_id,
                &request.workflow_ref,
                &request.endpoint_ref,
            )?;
            let session = app.local_api_session_snapshot(&request.session_id)?;
            Ok(LocalDaemonResponse::WorkflowWatchdogCreated {
                watchdog,
                workflow,
                endpoint,
                session,
            })
        }
        LocalDaemonRequest::ListWorkflowWatchdogs(request) => {
            Ok(LocalDaemonResponse::WorkflowWatchdogsListed {
                watchdogs: app.sessions().list_workflow_watchdogs(
                    &request.session_id,
                    request.workflow_ref.as_deref(),
                )?,
            })
        }
        LocalDaemonRequest::SetWorkflowWatchdogEnabled(request) => {
            let watchdog = app.sessions_mut().set_workflow_watchdog_enabled(
                &request.session_id,
                &request.watchdog_ref,
                request.enabled,
            )?;
            let session = app.local_api_session_snapshot(&request.session_id)?;
            Ok(LocalDaemonResponse::WorkflowWatchdogUpdated { watchdog, session })
        }
        LocalDaemonRequest::RemoveWorkflowWatchdog(request) => {
            let watchdog = app
                .sessions_mut()
                .remove_workflow_watchdog(&request.session_id, &request.watchdog_ref)?;
            let session = app.local_api_session_snapshot(&request.session_id)?;
            Ok(LocalDaemonResponse::WorkflowWatchdogRemoved { watchdog, session })
        }
        LocalDaemonRequest::SetWorkflowFlushContext(request) => {
            let workflow = app
                .sessions_mut()
                .set_workflow_flush_agent_context_before_run(
                    &request.session_id,
                    &request.workflow_ref,
                    request.flush_agent_context_before_run,
                )?;
            let session = app.local_api_session_snapshot(&request.session_id)?;
            Ok(LocalDaemonResponse::WorkflowFlushContextUpdated { workflow, session })
        }
        LocalDaemonRequest::SetWorkflowRunOutputSchema(request) => {
            let workflow = app.sessions_mut().set_workflow_run_output_schema_ref(
                &request.session_id,
                &request.workflow_ref,
                request.run_output_schema_ref.clone(),
            )?;
            let session = app.local_api_session_snapshot(&request.session_id)?;
            Ok(LocalDaemonResponse::WorkflowRunOutputSchemaUpdated { workflow, session })
        }
        LocalDaemonRequest::SetWorkflowIntermediateOutputSchema(request) => {
            let workflow = app
                .sessions_mut()
                .set_workflow_intermediate_output_schema_ref(
                    &request.session_id,
                    &request.workflow_ref,
                    request.intermediate_output_schema_ref.clone(),
                )?;
            let session = app.local_api_session_snapshot(&request.session_id)?;
            Ok(LocalDaemonResponse::WorkflowIntermediateOutputSchemaUpdated { workflow, session })
        }
        LocalDaemonRequest::SetWorkflowLaunchPolicy(request) => {
            let session = app
                .sessions_mut()
                .set_workflow_launch_policy(&request.session_id, request.policy)?;
            let mut session = session;
            session.set_agents(app.agents().get_session_agents(&request.session_id));
            Ok(LocalDaemonResponse::WorkflowLaunchPolicyUpdated { session })
        }
        LocalDaemonRequest::ListQueuedWorkflowLaunches(request) => {
            Ok(LocalDaemonResponse::QueuedWorkflowLaunchesListed {
                queued_launches: app
                    .sessions()
                    .list_queued_workflow_launches(&request.session_id)?,
            })
        }
        LocalDaemonRequest::RemoveQueuedWorkflowLaunch(request) => {
            let queued_launch = app
                .sessions_mut()
                .remove_queued_workflow_launch(&request.session_id, &request.queue_item_ref)?;
            let session = app.local_api_session_snapshot(&request.session_id)?;
            Ok(LocalDaemonResponse::QueuedWorkflowLaunchRemoved {
                queued_launch,
                session,
            })
        }
        LocalDaemonRequest::ClearQueuedWorkflowLaunches(request) => {
            let queued_launches = app
                .sessions_mut()
                .clear_queued_workflow_launches(&request.session_id)?;
            let session = app.local_api_session_snapshot(&request.session_id)?;
            Ok(LocalDaemonResponse::QueuedWorkflowLaunchesCleared {
                queued_launches,
                session,
            })
        }
        LocalDaemonRequest::ValidateWorkflowOutput(request) => {
            let result = crate::transport::runtime_tools::dispatch_runtime_tool_call(
                app,
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
                app,
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
            let workflow_run = app
                .sessions()
                .resolve_workflow_run_ref(&request.session_id, &request.workflow_run_ref)?
                .clone();
            let session = app.local_api_session_snapshot(&request.session_id)?;
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

pub(crate) fn is_workflow_command(request: &LocalDaemonRequest) -> bool {
    workflow_session_id(request).is_some()
}

fn workflow_session_id(request: &LocalDaemonRequest) -> Option<String> {
    Some(match request {
        LocalDaemonRequest::CreateWorkflow(request) => request.session_id.clone(),
        LocalDaemonRequest::AliasWorkflow(request) => request.session_id.clone(),
        LocalDaemonRequest::ListWorkflows(request) => request.session_id.clone(),
        LocalDaemonRequest::ResolveWorkflow(request) => request.session_id.clone(),
        LocalDaemonRequest::CreateWorkflowEndpoint(request) => request.session_id.clone(),
        LocalDaemonRequest::AliasWorkflowEndpoint(request) => request.session_id.clone(),
        LocalDaemonRequest::BindWorkflowEndpoint(request) => request.session_id.clone(),
        LocalDaemonRequest::AddWorkflowNode(request) => request.session_id.clone(),
        LocalDaemonRequest::RemoveWorkflowNode(request) => request.session_id.clone(),
        LocalDaemonRequest::UpdateWorkflowNodeInstructions(request) => request.session_id.clone(),
        LocalDaemonRequest::SetWorkflowNodeCanCompleteRun(request) => request.session_id.clone(),
        LocalDaemonRequest::SetWorkflowNodeCanEmitIntermediateOutput(request) => {
            request.session_id.clone()
        }
        LocalDaemonRequest::SetWorkflowNodeIntermediateOutputSchema(request) => {
            request.session_id.clone()
        }
        LocalDaemonRequest::SetWorkflowNodeMaxTurns(request) => request.session_id.clone(),
        LocalDaemonRequest::AddWorkflowEdge(request) => request.session_id.clone(),
        LocalDaemonRequest::RemoveWorkflowEdge(request) => request.session_id.clone(),
        LocalDaemonRequest::SetWorkflowRunOutputSchema(request) => request.session_id.clone(),
        LocalDaemonRequest::SetWorkflowIntermediateOutputSchema(request) => {
            request.session_id.clone()
        }
        LocalDaemonRequest::SetWorkflowFlushContext(request) => request.session_id.clone(),
        LocalDaemonRequest::SetWorkflowLaunchPolicy(request) => request.session_id.clone(),
        LocalDaemonRequest::InvokeWorkflowEndpoint(request) => request.session_id.clone(),
        LocalDaemonRequest::ListWorkflowRuns(request) => request.session_id.clone(),
        LocalDaemonRequest::GetWorkflowRun(request) => request.session_id.clone(),
        LocalDaemonRequest::AckWorkflowTurn(request) => request.session_id.clone(),
        LocalDaemonRequest::ValidateWorkflowOutput(request) => request.session_id.clone(),
        LocalDaemonRequest::CancelWorkflowRun(request) => request.session_id.clone(),
        LocalDaemonRequest::ResumeWorkflowRun(request) => request.session_id.clone(),
        LocalDaemonRequest::ClearQueuedWorkflowLaunches(request) => request.session_id.clone(),
        LocalDaemonRequest::RemoveQueuedWorkflowLaunch(request) => request.session_id.clone(),
        LocalDaemonRequest::ListQueuedWorkflowLaunches(request) => request.session_id.clone(),
        LocalDaemonRequest::CreateWorkflowWatchdog(request) => request.session_id.clone(),
        LocalDaemonRequest::RemoveWorkflowWatchdog(request) => request.session_id.clone(),
        LocalDaemonRequest::SetWorkflowWatchdogEnabled(request) => request.session_id.clone(),
        LocalDaemonRequest::ListWorkflowWatchdogs(request) => request.session_id.clone(),
        _ => return None,
    })
}

fn workflow_response_session(
    response: &LocalDaemonResponse,
) -> Option<crate::session::RuntimeSession> {
    match response {
        LocalDaemonResponse::WorkflowCreated { session, .. }
        | LocalDaemonResponse::WorkflowAliased { session, .. }
        | LocalDaemonResponse::WorkflowEndpointCreated { session, .. }
        | LocalDaemonResponse::WorkflowEndpointAliased { session, .. }
        | LocalDaemonResponse::WorkflowEndpointBound { session, .. }
        | LocalDaemonResponse::WorkflowNodeAdded { session, .. }
        | LocalDaemonResponse::WorkflowNodeRemoved { session, .. }
        | LocalDaemonResponse::WorkflowNodeInstructionsUpdated { session, .. }
        | LocalDaemonResponse::WorkflowNodeCanCompleteRunUpdated { session, .. }
        | LocalDaemonResponse::WorkflowNodeCanEmitIntermediateOutputUpdated { session, .. }
        | LocalDaemonResponse::WorkflowNodeIntermediateOutputSchemaUpdated { session, .. }
        | LocalDaemonResponse::WorkflowNodeMaxTurnsUpdated { session, .. }
        | LocalDaemonResponse::WorkflowEdgeAdded { session, .. }
        | LocalDaemonResponse::WorkflowEdgeRemoved { session, .. }
        | LocalDaemonResponse::WorkflowRunInvoked { session, .. }
        | LocalDaemonResponse::WorkflowRunQueued { session, .. }
        | LocalDaemonResponse::WorkflowRunCancelled { session, .. }
        | LocalDaemonResponse::WorkflowRunResumed { session, .. }
        | LocalDaemonResponse::WorkflowWatchdogCreated { session, .. }
        | LocalDaemonResponse::WorkflowWatchdogUpdated { session, .. }
        | LocalDaemonResponse::WorkflowWatchdogRemoved { session, .. }
        | LocalDaemonResponse::WorkflowFlushContextUpdated { session, .. }
        | LocalDaemonResponse::WorkflowRunOutputSchemaUpdated { session, .. }
        | LocalDaemonResponse::WorkflowIntermediateOutputSchemaUpdated { session, .. }
        | LocalDaemonResponse::WorkflowLaunchPolicyUpdated { session, .. }
        | LocalDaemonResponse::QueuedWorkflowLaunchRemoved { session, .. }
        | LocalDaemonResponse::QueuedWorkflowLaunchesCleared { session, .. }
        | LocalDaemonResponse::WorkflowTurnAcknowledged { session, .. } => Some(session.clone()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use tokio::sync::Mutex;
    use tokio::time::{timeout, Duration};

    use crate::kernel::projection::{AgentRuntimeProjectionStore, SessionStateProjectionStore};
    use crate::kernel::workflow_actor::WorkflowRuntime;
    use crate::local::{CreateWorkflowRequest, LocalDaemonRequest};
    use crate::{DaemonApp, DaemonConfig, DaemonError};

    #[tokio::test]
    async fn workflow_lane_resolution_rejects_warmed_missing_session_without_app_lock() {
        let app = Arc::new(Mutex::new(
            DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot"),
        ));
        let session_projection = SessionStateProjectionStore::default();
        session_projection.update_list(Vec::new());
        let runtime = WorkflowRuntime::new(
            Arc::clone(&app),
            session_projection,
            AgentRuntimeProjectionStore::default(),
        );
        let request = LocalDaemonRequest::CreateWorkflow(CreateWorkflowRequest {
            session_id: "missing-session".to_string(),
            alias: Some("pipeline".to_string()),
        });

        let _locked_app = app.lock().await;
        let error = timeout(Duration::from_millis(100), async {
            runtime.resolve_workflow_lane_key(&request)
        })
        .await
        .expect("warmed missing workflow session resolution should not wait for the app lock")
        .expect_err("missing workflow session should fail");

        match error {
            DaemonError::SessionNotFound { session_id } => {
                assert_eq!(session_id, "missing-session");
            }
            error => panic!("unexpected error: {error}"),
        }
        assert!(
            !runtime.has_lane("missing-session").await,
            "missing workflow session should be rejected before creating a workflow lane"
        );
    }
}
