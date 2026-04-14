use std::collections::HashMap;
use std::sync::Arc;

use tokio::sync::{mpsc, oneshot, Mutex};

use crate::app::workflow_runtime::WorkflowLaunchOutcome;
use crate::app::DaemonApp;
use crate::error::DaemonError;
use crate::kernel::projection::{
    ActorQueueSnapshot, AgentRuntimeProjectionStore, SessionStateProjectionStore,
};
use crate::local::{
    AckWorkflowTurnRequest, AddWorkflowEdgeRequest, AddWorkflowNodeRequest,
    AliasWorkflowEndpointRequest, AliasWorkflowRequest, BindWorkflowEndpointRequest,
    CancelWorkflowRunRequest, ClearQueuedWorkflowLaunchesRequest, CreateWorkflowEndpointRequest,
    CreateWorkflowRequest, CreateWorkflowWatchdogRequest, GetWorkflowRunRequest,
    InvokeWorkflowEndpointRequest, ListQueuedWorkflowLaunchesRequest, ListWorkflowRunsRequest,
    ListWorkflowWatchdogsRequest, ListWorkflowsRequest, LocalDaemonRequest, LocalDaemonResponse,
    RemoveQueuedWorkflowLaunchRequest, RemoveWorkflowEdgeRequest, RemoveWorkflowNodeRequest,
    RemoveWorkflowWatchdogRequest, ResolveWorkflowRequest, ResumeWorkflowRunRequest,
    SetWorkflowFlushContextRequest, SetWorkflowIntermediateOutputSchemaRequest,
    SetWorkflowLaunchPolicyRequest, SetWorkflowNodeCanCompleteRunRequest,
    SetWorkflowNodeCanEmitIntermediateOutputRequest,
    SetWorkflowNodeIntermediateOutputSchemaRequest, SetWorkflowNodeMaxTurnsRequest,
    SetWorkflowRunOutputSchemaRequest, SetWorkflowWatchdogEnabledRequest,
    UpdateWorkflowNodeInstructionsRequest, ValidateWorkflowOutputRequest,
};

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

type WorkflowStoreExecutionResult = (
    Result<LocalDaemonResponse, DaemonError>,
    Option<crate::session::RuntimeSession>,
);

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

    async fn create_workflow(
        &self,
        request: CreateWorkflowRequest,
    ) -> WorkflowStoreExecutionResult {
        let session_id = request.session_id.clone();
        self.execute_operation(&session_id, move |app| {
            app.kernel_workflows().create_workflow(request)
        })
        .await
    }

    async fn alias_workflow(&self, request: AliasWorkflowRequest) -> WorkflowStoreExecutionResult {
        let session_id = request.session_id.clone();
        self.execute_operation(&session_id, move |app| {
            app.kernel_workflows().alias_workflow(request)
        })
        .await
    }

    async fn list_workflows(&self, request: ListWorkflowsRequest) -> WorkflowStoreExecutionResult {
        let session_id = request.session_id.clone();
        self.execute_operation(&session_id, move |app| {
            app.kernel_workflows().list_workflows(request)
        })
        .await
    }

    async fn resolve_workflow(
        &self,
        request: ResolveWorkflowRequest,
    ) -> WorkflowStoreExecutionResult {
        let session_id = request.session_id.clone();
        self.execute_operation(&session_id, move |app| {
            app.kernel_workflows().resolve_workflow(request)
        })
        .await
    }

    async fn create_workflow_endpoint(
        &self,
        request: CreateWorkflowEndpointRequest,
    ) -> WorkflowStoreExecutionResult {
        let session_id = request.session_id.clone();
        self.execute_operation(&session_id, move |app| {
            app.kernel_workflows().create_workflow_endpoint(request)
        })
        .await
    }

    async fn alias_workflow_endpoint(
        &self,
        request: AliasWorkflowEndpointRequest,
    ) -> WorkflowStoreExecutionResult {
        let session_id = request.session_id.clone();
        self.execute_operation(&session_id, move |app| {
            app.kernel_workflows().alias_workflow_endpoint(request)
        })
        .await
    }

    async fn bind_workflow_endpoint(
        &self,
        request: BindWorkflowEndpointRequest,
    ) -> WorkflowStoreExecutionResult {
        let session_id = request.session_id.clone();
        self.execute_operation(&session_id, move |app| {
            app.kernel_workflows().bind_workflow_endpoint(request)
        })
        .await
    }

    async fn add_workflow_node(
        &self,
        request: AddWorkflowNodeRequest,
    ) -> WorkflowStoreExecutionResult {
        let session_id = request.session_id.clone();
        self.execute_operation(&session_id, move |app| {
            app.kernel_workflows().add_workflow_node(request)
        })
        .await
    }

    async fn remove_workflow_node(
        &self,
        request: RemoveWorkflowNodeRequest,
    ) -> WorkflowStoreExecutionResult {
        let session_id = request.session_id.clone();
        self.execute_operation(&session_id, move |app| {
            app.kernel_workflows().remove_workflow_node(request)
        })
        .await
    }

    async fn update_workflow_node_instructions(
        &self,
        request: UpdateWorkflowNodeInstructionsRequest,
    ) -> WorkflowStoreExecutionResult {
        let session_id = request.session_id.clone();
        self.execute_operation(&session_id, move |app| {
            app.kernel_workflows()
                .update_workflow_node_instructions(request)
        })
        .await
    }

    async fn set_workflow_node_can_complete_run(
        &self,
        request: SetWorkflowNodeCanCompleteRunRequest,
    ) -> WorkflowStoreExecutionResult {
        let session_id = request.session_id.clone();
        self.execute_operation(&session_id, move |app| {
            app.kernel_workflows()
                .set_workflow_node_can_complete_run(request)
        })
        .await
    }

    async fn set_workflow_node_can_emit_intermediate_output(
        &self,
        request: SetWorkflowNodeCanEmitIntermediateOutputRequest,
    ) -> WorkflowStoreExecutionResult {
        let session_id = request.session_id.clone();
        self.execute_operation(&session_id, move |app| {
            app.kernel_workflows()
                .set_workflow_node_can_emit_intermediate_output(request)
        })
        .await
    }

    async fn set_workflow_node_intermediate_output_schema(
        &self,
        request: SetWorkflowNodeIntermediateOutputSchemaRequest,
    ) -> WorkflowStoreExecutionResult {
        let session_id = request.session_id.clone();
        self.execute_operation(&session_id, move |app| {
            app.kernel_workflows()
                .set_workflow_node_intermediate_output_schema(request)
        })
        .await
    }

    async fn set_workflow_node_max_turns(
        &self,
        request: SetWorkflowNodeMaxTurnsRequest,
    ) -> WorkflowStoreExecutionResult {
        let session_id = request.session_id.clone();
        self.execute_operation(&session_id, move |app| {
            app.kernel_workflows().set_workflow_node_max_turns(request)
        })
        .await
    }

    async fn add_workflow_edge(
        &self,
        request: AddWorkflowEdgeRequest,
    ) -> WorkflowStoreExecutionResult {
        let session_id = request.session_id.clone();
        self.execute_operation(&session_id, move |app| {
            app.kernel_workflows().add_workflow_edge(request)
        })
        .await
    }

    async fn remove_workflow_edge(
        &self,
        request: RemoveWorkflowEdgeRequest,
    ) -> WorkflowStoreExecutionResult {
        let session_id = request.session_id.clone();
        self.execute_operation(&session_id, move |app| {
            app.kernel_workflows().remove_workflow_edge(request)
        })
        .await
    }

    async fn invoke_workflow_endpoint(
        &self,
        request: InvokeWorkflowEndpointRequest,
    ) -> WorkflowStoreExecutionResult {
        let session_id = request.session_id.clone();
        self.execute_operation(&session_id, move |app| {
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
        })
        .await
    }

    async fn list_workflow_runs(
        &self,
        request: ListWorkflowRunsRequest,
    ) -> WorkflowStoreExecutionResult {
        let session_id = request.session_id.clone();
        self.execute_operation(&session_id, move |app| {
            Ok(LocalDaemonResponse::WorkflowRunsListed {
                workflow_runs: app
                    .sessions()
                    .list_workflow_runs(&request.session_id, request.workflow_ref.as_deref())?,
            })
        })
        .await
    }

    async fn get_workflow_run(
        &self,
        request: GetWorkflowRunRequest,
    ) -> WorkflowStoreExecutionResult {
        let session_id = request.session_id.clone();
        self.execute_operation(&session_id, move |app| {
            Ok(LocalDaemonResponse::WorkflowRun {
                workflow_run: app
                    .sessions()
                    .resolve_workflow_run_ref(&request.session_id, &request.workflow_run_ref)?,
            })
        })
        .await
    }

    async fn cancel_workflow_run(
        &self,
        request: CancelWorkflowRunRequest,
    ) -> WorkflowStoreExecutionResult {
        let session_id = request.session_id.clone();
        self.execute_operation(&session_id, move |app| {
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
        })
        .await
    }

    async fn resume_workflow_run(
        &self,
        request: ResumeWorkflowRunRequest,
    ) -> WorkflowStoreExecutionResult {
        let session_id = request.session_id.clone();
        self.execute_operation(&session_id, move |app| {
            let workflow_run = app
                .resume_workflow_run_from_runtime(&request.session_id, &request.workflow_run_ref)?;
            let session = app.local_api_session_snapshot(&request.session_id)?;
            Ok(LocalDaemonResponse::WorkflowRunResumed {
                workflow_run,
                session,
            })
        })
        .await
    }

    async fn create_workflow_watchdog(
        &self,
        request: CreateWorkflowWatchdogRequest,
    ) -> WorkflowStoreExecutionResult {
        let session_id = request.session_id.clone();
        self.execute_operation(&session_id, move |app| {
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
        })
        .await
    }

    async fn list_workflow_watchdogs(
        &self,
        request: ListWorkflowWatchdogsRequest,
    ) -> WorkflowStoreExecutionResult {
        let session_id = request.session_id.clone();
        self.execute_operation(&session_id, move |app| {
            Ok(LocalDaemonResponse::WorkflowWatchdogsListed {
                watchdogs: app.sessions().list_workflow_watchdogs(
                    &request.session_id,
                    request.workflow_ref.as_deref(),
                )?,
            })
        })
        .await
    }

    async fn set_workflow_watchdog_enabled(
        &self,
        request: SetWorkflowWatchdogEnabledRequest,
    ) -> WorkflowStoreExecutionResult {
        let session_id = request.session_id.clone();
        self.execute_operation(&session_id, move |app| {
            let watchdog = app.sessions_mut().set_workflow_watchdog_enabled(
                &request.session_id,
                &request.watchdog_ref,
                request.enabled,
            )?;
            let session = app.local_api_session_snapshot(&request.session_id)?;
            Ok(LocalDaemonResponse::WorkflowWatchdogUpdated { watchdog, session })
        })
        .await
    }

    async fn remove_workflow_watchdog(
        &self,
        request: RemoveWorkflowWatchdogRequest,
    ) -> WorkflowStoreExecutionResult {
        let session_id = request.session_id.clone();
        self.execute_operation(&session_id, move |app| {
            let watchdog = app
                .sessions_mut()
                .remove_workflow_watchdog(&request.session_id, &request.watchdog_ref)?;
            let session = app.local_api_session_snapshot(&request.session_id)?;
            Ok(LocalDaemonResponse::WorkflowWatchdogRemoved { watchdog, session })
        })
        .await
    }

    async fn set_workflow_flush_context(
        &self,
        request: SetWorkflowFlushContextRequest,
    ) -> WorkflowStoreExecutionResult {
        let session_id = request.session_id.clone();
        self.execute_operation(&session_id, move |app| {
            app.kernel_workflows().set_workflow_flush_context(request)
        })
        .await
    }

    async fn set_workflow_run_output_schema(
        &self,
        request: SetWorkflowRunOutputSchemaRequest,
    ) -> WorkflowStoreExecutionResult {
        let session_id = request.session_id.clone();
        self.execute_operation(&session_id, move |app| {
            app.kernel_workflows()
                .set_workflow_run_output_schema(request)
        })
        .await
    }

    async fn set_workflow_intermediate_output_schema(
        &self,
        request: SetWorkflowIntermediateOutputSchemaRequest,
    ) -> WorkflowStoreExecutionResult {
        let session_id = request.session_id.clone();
        self.execute_operation(&session_id, move |app| {
            app.kernel_workflows()
                .set_workflow_intermediate_output_schema(request)
        })
        .await
    }

    async fn set_workflow_launch_policy(
        &self,
        request: SetWorkflowLaunchPolicyRequest,
    ) -> WorkflowStoreExecutionResult {
        let session_id = request.session_id.clone();
        self.execute_operation(&session_id, move |app| {
            app.kernel_workflows().set_workflow_launch_policy(request)
        })
        .await
    }

    async fn list_queued_workflow_launches(
        &self,
        request: ListQueuedWorkflowLaunchesRequest,
    ) -> WorkflowStoreExecutionResult {
        let session_id = request.session_id.clone();
        self.execute_operation(&session_id, move |app| {
            Ok(LocalDaemonResponse::QueuedWorkflowLaunchesListed {
                queued_launches: app
                    .sessions()
                    .list_queued_workflow_launches(&request.session_id)?,
            })
        })
        .await
    }

    async fn remove_queued_workflow_launch(
        &self,
        request: RemoveQueuedWorkflowLaunchRequest,
    ) -> WorkflowStoreExecutionResult {
        let session_id = request.session_id.clone();
        self.execute_operation(&session_id, move |app| {
            let queued_launch = app
                .sessions_mut()
                .remove_queued_workflow_launch(&request.session_id, &request.queue_item_ref)?;
            let session = app.local_api_session_snapshot(&request.session_id)?;
            Ok(LocalDaemonResponse::QueuedWorkflowLaunchRemoved {
                queued_launch,
                session,
            })
        })
        .await
    }

    async fn clear_queued_workflow_launches(
        &self,
        request: ClearQueuedWorkflowLaunchesRequest,
    ) -> WorkflowStoreExecutionResult {
        let session_id = request.session_id.clone();
        self.execute_operation(&session_id, move |app| {
            let queued_launches = app
                .sessions_mut()
                .clear_queued_workflow_launches(&request.session_id)?;
            let session = app.local_api_session_snapshot(&request.session_id)?;
            Ok(LocalDaemonResponse::QueuedWorkflowLaunchesCleared {
                queued_launches,
                session,
            })
        })
        .await
    }

    async fn validate_workflow_output(
        &self,
        request: ValidateWorkflowOutputRequest,
    ) -> WorkflowStoreExecutionResult {
        let session_id = request.session_id.clone();
        self.execute_operation(&session_id, move |app| {
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
        })
        .await
    }

    async fn ack_workflow_turn(
        &self,
        request: AckWorkflowTurnRequest,
    ) -> WorkflowStoreExecutionResult {
        let session_id = request.session_id.clone();
        self.execute_operation(&session_id, move |app| {
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
        })
        .await
    }
}

async fn run_workflow_command_lane(
    store: WorkflowRuntimeStore,
    session_projection: SessionStateProjectionStore,
    agent_runtime_projection: AgentRuntimeProjectionStore,
    session_id: String,
    mut rx: mpsc::Receiver<WorkflowCommandEnvelope>,
) {
    let executor =
        WorkflowRuntimeCommandExecutor::new(store, session_projection, agent_runtime_projection);
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
}

impl WorkflowRuntimeCommandExecutor {
    fn new(
        store: WorkflowRuntimeStore,
        session_projection: SessionStateProjectionStore,
        agent_runtime_projection: AgentRuntimeProjectionStore,
    ) -> Self {
        Self {
            store,
            session_projection,
            agent_runtime_projection,
        }
    }

    async fn execute(
        &self,
        request: LocalDaemonRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let (result, projected_session) = match request {
            LocalDaemonRequest::CreateWorkflow(request) => {
                self.store.create_workflow(request).await
            }
            LocalDaemonRequest::AliasWorkflow(request) => self.store.alias_workflow(request).await,
            LocalDaemonRequest::ListWorkflows(request) => self.store.list_workflows(request).await,
            LocalDaemonRequest::ResolveWorkflow(request) => {
                self.store.resolve_workflow(request).await
            }
            LocalDaemonRequest::CreateWorkflowEndpoint(request) => {
                self.store.create_workflow_endpoint(request).await
            }
            LocalDaemonRequest::AliasWorkflowEndpoint(request) => {
                self.store.alias_workflow_endpoint(request).await
            }
            LocalDaemonRequest::BindWorkflowEndpoint(request) => {
                self.store.bind_workflow_endpoint(request).await
            }
            LocalDaemonRequest::AddWorkflowNode(request) => {
                self.store.add_workflow_node(request).await
            }
            LocalDaemonRequest::RemoveWorkflowNode(request) => {
                self.store.remove_workflow_node(request).await
            }
            LocalDaemonRequest::UpdateWorkflowNodeInstructions(request) => {
                self.store.update_workflow_node_instructions(request).await
            }
            LocalDaemonRequest::SetWorkflowNodeCanCompleteRun(request) => {
                self.store.set_workflow_node_can_complete_run(request).await
            }
            LocalDaemonRequest::SetWorkflowNodeCanEmitIntermediateOutput(request) => {
                self.store
                    .set_workflow_node_can_emit_intermediate_output(request)
                    .await
            }
            LocalDaemonRequest::SetWorkflowNodeIntermediateOutputSchema(request) => {
                self.store
                    .set_workflow_node_intermediate_output_schema(request)
                    .await
            }
            LocalDaemonRequest::SetWorkflowNodeMaxTurns(request) => {
                self.store.set_workflow_node_max_turns(request).await
            }
            LocalDaemonRequest::AddWorkflowEdge(request) => {
                self.store.add_workflow_edge(request).await
            }
            LocalDaemonRequest::RemoveWorkflowEdge(request) => {
                self.store.remove_workflow_edge(request).await
            }
            LocalDaemonRequest::InvokeWorkflowEndpoint(request) => {
                self.store.invoke_workflow_endpoint(request).await
            }
            LocalDaemonRequest::ListWorkflowRuns(request) => {
                self.store.list_workflow_runs(request).await
            }
            LocalDaemonRequest::GetWorkflowRun(request) => {
                self.store.get_workflow_run(request).await
            }
            LocalDaemonRequest::CancelWorkflowRun(request) => {
                self.store.cancel_workflow_run(request).await
            }
            LocalDaemonRequest::ResumeWorkflowRun(request) => {
                self.store.resume_workflow_run(request).await
            }
            LocalDaemonRequest::CreateWorkflowWatchdog(request) => {
                self.store.create_workflow_watchdog(request).await
            }
            LocalDaemonRequest::ListWorkflowWatchdogs(request) => {
                self.store.list_workflow_watchdogs(request).await
            }
            LocalDaemonRequest::SetWorkflowWatchdogEnabled(request) => {
                self.store.set_workflow_watchdog_enabled(request).await
            }
            LocalDaemonRequest::RemoveWorkflowWatchdog(request) => {
                self.store.remove_workflow_watchdog(request).await
            }
            LocalDaemonRequest::SetWorkflowFlushContext(request) => {
                self.store.set_workflow_flush_context(request).await
            }
            LocalDaemonRequest::SetWorkflowRunOutputSchema(request) => {
                self.store.set_workflow_run_output_schema(request).await
            }
            LocalDaemonRequest::SetWorkflowIntermediateOutputSchema(request) => {
                self.store
                    .set_workflow_intermediate_output_schema(request)
                    .await
            }
            LocalDaemonRequest::SetWorkflowLaunchPolicy(request) => {
                self.store.set_workflow_launch_policy(request).await
            }
            LocalDaemonRequest::ListQueuedWorkflowLaunches(request) => {
                self.store.list_queued_workflow_launches(request).await
            }
            LocalDaemonRequest::RemoveQueuedWorkflowLaunch(request) => {
                self.store.remove_queued_workflow_launch(request).await
            }
            LocalDaemonRequest::ClearQueuedWorkflowLaunches(request) => {
                self.store.clear_queued_workflow_launches(request).await
            }
            LocalDaemonRequest::ValidateWorkflowOutput(request) => {
                self.store.validate_workflow_output(request).await
            }
            LocalDaemonRequest::AckWorkflowTurn(request) => {
                self.store.ack_workflow_turn(request).await
            }
            _ => (
                Err(DaemonError::LocalTransport {
                    operation: "execute workflow request",
                    message: "request is not handled by the workflow runtime".to_string(),
                }),
                None,
            ),
        };
        if let Some(session) = projected_session {
            self.agent_runtime_projection.update_session(&session);
            self.session_projection.update(session);
        }
        result
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
