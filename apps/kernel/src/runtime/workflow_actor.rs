use std::collections::HashMap;
use std::sync::Arc;

use tokio::sync::{mpsc, oneshot, Mutex};

use crate::error::DaemonError;
use crate::local::{LocalDaemonRequest, LocalDaemonResponse};
use crate::runtime::command::{KernelCallerKind, KernelCommand};
use crate::runtime::command_latency::{
    log_lane_completed, log_lane_dispatched, log_lane_enqueue_failed, log_lane_enqueued,
    CommandTrace, LaneCommandTrace,
};
use crate::runtime::projection::{
    ActorQueueSnapshot, AgentRuntimeProjectionStore, SessionStateProjectionStore,
};
use crate::runtime::state::KernelRuntimeState;
use crate::session::DEFAULT_LOCAL_USER_ID;

pub(crate) const WORKFLOW_COMMAND_QUEUE_LIMIT: usize = 128;

#[derive(Debug)]
struct WorkflowCommandEnvelope {
    command_id: String,
    command_type: String,
    telemetry: LaneCommandTrace,
    caller_user_id: String,
    caller_metaagent_id: Option<String>,
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
        state: KernelRuntimeState,
        session_projection: SessionStateProjectionStore,
        agent_runtime_projection: AgentRuntimeProjectionStore,
    ) -> Self {
        Self::with_store(
            WorkflowRuntimeStore::new(state),
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
        command: KernelCommand,
        request: LocalDaemonRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let session_id = self.resolve_workflow_lane_key(&request)?;
        let lane = self.workflow_lane(&session_id).await;
        let (result_tx, result_rx) = oneshot::channel();
        let caller_user_id = command_workflow_actor_user_id(&command);
        let caller_metaagent_id = command.caller.metaagent_id.clone();
        let telemetry = LaneCommandTrace::new(
            CommandTrace::from_command(&command),
            crate::runtime::command_latency::now_ms(),
        );
        let queue_depth_before = self.queue_limit.saturating_sub(lane.capacity());
        match lane.try_send(WorkflowCommandEnvelope {
            command_id: telemetry.command_id().to_string(),
            command_type: telemetry.command_type().to_string(),
            telemetry: telemetry.clone(),
            caller_user_id,
            caller_metaagent_id,
            request,
            result_tx,
        }) {
            Ok(()) => {
                let queue_depth_after = self.queue_limit.saturating_sub(lane.capacity());
                log_lane_enqueued(
                    &telemetry,
                    "workflow",
                    &session_id,
                    self.queue_limit,
                    queue_depth_before,
                    queue_depth_after,
                );
            }
            Err(error) => {
                let message = format!("workflow command lane overloaded: {error}");
                log_lane_enqueue_failed(
                    &telemetry,
                    "workflow",
                    &session_id,
                    self.queue_limit,
                    queue_depth_before,
                    &message,
                );
                return Err(DaemonError::LocalTransport {
                    operation: "enqueue workflow kernel command",
                    message,
                });
            }
        }
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
        if matches!(
            request,
            LocalDaemonRequest::MaterializeWorkflowPublication(_)
        ) {
            return Ok(session_id);
        }
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
    state: KernelRuntimeState,
}

type WorkflowStoreExecutionResult = (
    Result<LocalDaemonResponse, DaemonError>,
    Option<crate::session::RuntimeSession>,
);

impl WorkflowRuntimeStore {
    pub(crate) fn new(state: KernelRuntimeState) -> Self {
        Self { state }
    }

    async fn execute_workflow_request(
        &self,
        request: LocalDaemonRequest,
        caller_user_id: String,
        caller_metaagent_id: Option<String>,
    ) -> WorkflowStoreExecutionResult {
        self.state
            .execute_workflow_request(request, caller_user_id, caller_metaagent_id)
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
        let dispatch_started_at_ms = crate::runtime::command_latency::now_ms();
        log_lane_dispatched(
            &envelope.telemetry,
            "workflow",
            &session_id,
            dispatch_started_at_ms,
        );
        crate::logging::info_with_fields(
            "daemon.kernel_workflow_actor",
            "workflow kernel command dispatched",
            serde_json::json!({
                "session_id": session_id,
                "command_id": envelope.command_id,
                "command_type": envelope.command_type,
            }),
        );
        let result = executor
            .execute(
                envelope.request,
                envelope.caller_user_id,
                envelope.caller_metaagent_id,
            )
            .await;
        log_lane_completed(
            &envelope.telemetry,
            "workflow",
            &session_id,
            dispatch_started_at_ms,
            &result,
        );
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
        caller_user_id: String,
        caller_metaagent_id: Option<String>,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let (result, projected_session) = self
            .store
            .execute_workflow_request(request, caller_user_id, caller_metaagent_id)
            .await;
        if let Some(session) = projected_session {
            self.agent_runtime_projection.update_session(&session);
            self.session_projection.update(session);
        }
        result
    }
}

fn command_workflow_actor_user_id(command: &KernelCommand) -> String {
    match command.caller.caller_kind {
        KernelCallerKind::LocalClient | KernelCallerKind::Metaagent => command
            .caller
            .user_id
            .clone()
            .unwrap_or_else(|| DEFAULT_LOCAL_USER_ID.to_string()),
        KernelCallerKind::RemoteClient
        | KernelCallerKind::RemoteKernel
        | KernelCallerKind::HostedService => command
            .caller
            .user_id
            .clone()
            .unwrap_or_else(|| DEFAULT_LOCAL_USER_ID.to_string()),
    }
}

pub(crate) fn is_workflow_command(request: &LocalDaemonRequest) -> bool {
    workflow_session_id(request).is_some()
}

fn workflow_session_id(request: &LocalDaemonRequest) -> Option<String> {
    Some(match request {
        LocalDaemonRequest::CreateWorkflow(request) => request.session_id.clone(),
        LocalDaemonRequest::ValidateWorkflowCode(request) => request.session_id.clone(),
        LocalDaemonRequest::ApplyWorkflowCode(request) => request.session_id.clone(),
        LocalDaemonRequest::CreateWorkflowCodeArtifact(request) => request.session_id.clone(),
        LocalDaemonRequest::UpdateWorkflowCodeArtifact(request) => request.session_id.clone(),
        LocalDaemonRequest::GetWorkflowCodeArtifact(request) => request.session_id.clone(),
        LocalDaemonRequest::ListWorkflowCodeArtifacts(request) => request.session_id.clone(),
        LocalDaemonRequest::DeleteWorkflowCodeArtifact(request) => request.session_id.clone(),
        LocalDaemonRequest::ApplyWorkflowDesignOp(request) => request.session_id.clone(),
        LocalDaemonRequest::AliasWorkflow(request) => request.session_id.clone(),
        LocalDaemonRequest::ListWorkflows(request) => request.session_id.clone(),
        LocalDaemonRequest::ResolveWorkflow(request) => request.session_id.clone(),
        LocalDaemonRequest::CreateWorkflowPublication(request) => request.session_id.clone(),
        LocalDaemonRequest::ListWorkflowPublications(request) => request.session_id.clone(),
        LocalDaemonRequest::GetWorkflowPublication(request) => request.session_id.clone(),
        LocalDaemonRequest::ExportWorkflowPublicationPackage(request) => request.session_id.clone(),
        LocalDaemonRequest::DisableWorkflowPublication(request) => request.session_id.clone(),
        LocalDaemonRequest::MaterializeWorkflowPublication(request) => {
            format!("publication-materialize:{}", request.publication_id)
        }
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
        LocalDaemonRequest::SetWorkflowNodeWaitForAllInputs(request) => request.session_id.clone(),
        LocalDaemonRequest::SetWorkflowNodeIntermediateOutputSchema(request) => {
            request.session_id.clone()
        }
        LocalDaemonRequest::SetWorkflowNodeMaxTurns(request) => request.session_id.clone(),
        LocalDaemonRequest::AddWorkflowEdge(request) => request.session_id.clone(),
        LocalDaemonRequest::RemoveWorkflowEdge(request) => request.session_id.clone(),
        LocalDaemonRequest::UpdateWorkflowCanvasLayout(request) => request.session_id.clone(),
        LocalDaemonRequest::SetWorkflowRunOutputSchema(request) => request.session_id.clone(),
        LocalDaemonRequest::SetWorkflowIntermediateOutputSchema(request) => {
            request.session_id.clone()
        }
        LocalDaemonRequest::SetWorkflowFlushContext(request) => request.session_id.clone(),
        LocalDaemonRequest::InvokeWorkflowEndpoint(request) => request.session_id.clone(),
        LocalDaemonRequest::ListWorkflowRuns(request) => request.session_id.clone(),
        LocalDaemonRequest::GetWorkflowRun(request) => request.session_id.clone(),
        LocalDaemonRequest::AckWorkflowTurn(request) => request.session_id.clone(),
        LocalDaemonRequest::ValidateWorkflowHandoff(request) => request.session_id.clone(),
        LocalDaemonRequest::CancelWorkflowRun(request) => request.session_id.clone(),
        LocalDaemonRequest::ResumeWorkflowRun(request) => request.session_id.clone(),
        LocalDaemonRequest::ListWorkflowPromptQueues(request) => request.session_id.clone(),
        LocalDaemonRequest::CreateWorkflowPromptQueue(request) => request.session_id.clone(),
        LocalDaemonRequest::UpdateWorkflowPromptQueue(request) => request.session_id.clone(),
        LocalDaemonRequest::RemoveWorkflowPromptQueue(request) => request.session_id.clone(),
        LocalDaemonRequest::ListQueuedWorkflowPrompts(request) => request.session_id.clone(),
        LocalDaemonRequest::UpdateQueuedWorkflowPrompt(request) => request.session_id.clone(),
        LocalDaemonRequest::RemoveQueuedWorkflowPrompt(request) => request.session_id.clone(),
        LocalDaemonRequest::ClearWorkflowPromptQueue(request) => request.session_id.clone(),
        LocalDaemonRequest::CreateWorkflowWatchdog(request) => request.session_id.clone(),
        LocalDaemonRequest::RemoveWorkflowWatchdog(request) => request.session_id.clone(),
        LocalDaemonRequest::SetWorkflowWatchdogEnabled(request) => request.session_id.clone(),
        LocalDaemonRequest::ListWorkflowWatchdogs(request) => request.session_id.clone(),
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use tokio::sync::Mutex;
    use tokio::time::{timeout, Duration};

    use crate::local::{
        ApplyWorkflowDesignOpRequest, CreateWorkflowRequest, LocalDaemonRequest,
        LocalDaemonResponse, WorkflowDesignOp, WorkflowDesignWorkflow,
    };
    use crate::runtime::command::KernelCommand;
    use crate::runtime::projection::{AgentRuntimeProjectionStore, SessionStateProjectionStore};
    use crate::runtime::state::KernelRuntimeState;
    use crate::runtime::workflow_actor::WorkflowRuntime;
    use crate::session::CreateSessionRequest;
    use crate::{DaemonApp, DaemonConfig, DaemonError};

    async fn owned_runtime_state(app: &Arc<Mutex<DaemonApp>>) -> KernelRuntimeState {
        let app_locked = app.lock().await;
        KernelRuntimeState::new_with_owned_state(
            Arc::clone(app),
            app_locked.config_projection_store(),
            app_locked.session_state_store(),
            app_locked.agents().clone(),
            app_locked.attachments().clone(),
            app_locked.providers().clone(),
            app_locked.provider_process_tracking_store(),
            app_locked.slices(),
            app_locked.session_state_projection_store(),
            app_locked.provider_run_projection_store(),
            app_locked.history_store(),
            app_locked.operational_history_store(),
            app_locked.durable_state_store(),
            app_locked.session_history_projection_store(),
            app_locked.prompt_state_owner(),
            app_locked.active_turn_store(),
            app_locked.prompt_activity_store(),
            app_locked.prompt_workspace_claim_store(),
            app_locked.structured_output_record_store(),
            app_locked.terminal_stream_store(),
            app_locked.workflow_design_event_store(),
            app_locked.metaagent_event_store(),
            app_locked.workspace_coordinator(),
        )
    }

    #[tokio::test]
    async fn workflow_lane_resolution_rejects_warmed_missing_session_without_app_lock() {
        let app = Arc::new(Mutex::new(
            DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot"),
        ));
        let session_projection = SessionStateProjectionStore::default();
        session_projection.update_list(Vec::new());
        let runtime = WorkflowRuntime::new(
            owned_runtime_state(&app).await,
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

    #[tokio::test]
    async fn workflow_design_op_appends_event_without_app_lock() {
        let mut daemon =
            DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
        let (session, _agent) = crate::app::KernelSessionService::new(&mut daemon)
            .create_session(CreateSessionRequest::new("workspace", "worktree"))
            .expect("session should be created");
        let event_store = daemon.workflow_design_event_store();
        let app = Arc::new(Mutex::new(daemon));
        let session_projection = SessionStateProjectionStore::default();
        session_projection.update(session.clone());
        let runtime = WorkflowRuntime::new(
            owned_runtime_state(&app).await,
            session_projection,
            AgentRuntimeProjectionStore::default(),
        );
        let request = LocalDaemonRequest::ApplyWorkflowDesignOp(ApplyWorkflowDesignOpRequest {
            session_id: session.id().to_string(),
            origin_client_id: "web-canvas".to_string(),
            op_id: "op-create-workflow".to_string(),
            op: WorkflowDesignOp::WorkflowCreate {
                workflow: WorkflowDesignWorkflow {
                    id: "workflow-design-lock-free".to_string(),
                    alias: Some("lock-free".to_string()),
                    flush_agent_context_before_run: None,
                    run_output_schema_ref: None,
                    intermediate_output_schema_ref: None,
                },
            },
        });
        let command =
            KernelCommand::from_local_request("cmd-workflow-design", None, None, &request);

        let _locked_app = app.lock().await;
        let response = timeout(Duration::from_millis(100), async {
            runtime.dispatch_workflow_command(command, request).await
        })
        .await
        .expect("workflow design op should not wait for the app lock")
        .expect("workflow design op should succeed");

        let LocalDaemonResponse::WorkflowDesignOpAccepted { event, .. } = response else {
            panic!("unexpected response: {response:?}");
        };
        assert_eq!(event.session_id, session.id());
        assert_eq!(event.origin_client_id, "web-canvas");
        assert_eq!(event.op_id, "op-create-workflow");

        let forwarded = event_store.events_since(session.id(), 0, "other-client");
        assert_eq!(forwarded, vec![event]);
    }
}
