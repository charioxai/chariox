use std::collections::HashMap;
use std::sync::Arc;

use tokio::sync::{mpsc, oneshot, Mutex};

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
    app: Arc<Mutex<DaemonApp>>,
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
        Self {
            app,
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
        let session_id =
            workflow_session_id(&request).ok_or_else(|| DaemonError::LocalTransport {
                operation: "route workflow kernel command",
                message: "request is not handled by the workflow runtime".to_string(),
            })?;
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

    async fn workflow_lane(&self, session_id: &str) -> mpsc::Sender<WorkflowCommandEnvelope> {
        let mut lanes = self.lanes.lock().await;
        if let Some(lane) = lanes.get(session_id) {
            return lane.clone();
        }
        let (tx, rx) = mpsc::channel(self.queue_limit);
        lanes.insert(session_id.to_string(), tx.clone());
        tokio::spawn(run_workflow_command_lane(
            Arc::clone(&self.app),
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

async fn run_workflow_command_lane(
    app: Arc<Mutex<DaemonApp>>,
    session_projection: SessionStateProjectionStore,
    agent_runtime_projection: AgentRuntimeProjectionStore,
    session_id: String,
    mut rx: mpsc::Receiver<WorkflowCommandEnvelope>,
) {
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
        let (result, projected_session) = {
            let mut app = app.lock().await;
            let result = app.handle_local_request(envelope.request);
            let projected_session = if result.is_ok() {
                app.local_api_session_snapshot(&session_id).ok()
            } else {
                None
            };
            (result, projected_session)
        };
        if let Some(session) = projected_session {
            agent_runtime_projection.update_session(&session);
            session_projection.update(session);
        }
        let _ = envelope.result_tx.send(result);
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
