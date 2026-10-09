//! Per-agent command lane queueing.

use tokio::sync::oneshot;

use super::command_executor::AgentRuntimeCommandExecutor;
use super::*;
use crate::runtime::command_latency::{
    log_lane_completed, log_lane_dispatched, log_lane_enqueue_failed, log_lane_enqueued,
    CommandTrace, LaneCommandTrace,
};
use crate::runtime::projection::ActorQueueSnapshot;

#[derive(Debug)]
#[allow(
    clippy::enum_variant_names,
    reason = "Actor command names mirror the public prompt operations"
)]
pub(super) enum AgentCommand {
    SubmitPrompt {
        request: crate::local::SubmitPromptRequest,
        trace_id: String,
        operation_id: String,
        operation_fingerprint: String,
        response_mode: PromptSubmitResponseMode,
    },
    CompletePrompt {
        request: crate::local::CompletePromptRequest,
        target_agent_id: String,
        next_queued_prompt: Option<PromptQueueItem>,
    },
    CancelActivePrompt {
        request: crate::local::CancelActivePromptRequest,
        target_agent_id: String,
    },
    SteerQueuedPrompt {
        request: crate::local::SteerQueuedPromptRequest,
    },
    CancelQueuedPrompt {
        request: crate::local::CancelQueuedPromptRequest,
    },
    UpdateQueuedPrompt {
        request: crate::local::UpdateQueuedPromptRequest,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum PromptSubmitResponseMode {
    Full,
    BatchItem,
}

#[derive(Debug)]
pub(super) struct AgentCommandEnvelope {
    pub(super) command_id: String,
    pub(super) command_type: String,
    pub(super) telemetry: LaneCommandTrace,
    pub(super) external_grant_id: Option<String>,
    pub(super) command: AgentCommand,
    pub(super) result_tx: oneshot::Sender<Result<LocalDaemonResponse, DaemonError>>,
}

impl AgentRuntime {
    pub(super) async fn dispatch_to_agent(
        &self,
        agent_id: String,
        command_trace: CommandTrace,
        external_grant_id: Option<String>,
        command: AgentCommand,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let lane_key = agent_id;
        let lane = self.agent_lane(&lane_key).await;
        let (result_tx, result_rx) = oneshot::channel();
        let telemetry =
            LaneCommandTrace::new(command_trace, crate::runtime::command_latency::now_ms());
        let queue_depth_before = self.queue_limit.saturating_sub(lane.capacity());
        match lane.try_send(AgentCommandEnvelope {
            command_id: telemetry.command_id().to_string(),
            command_type: telemetry.command_type().to_string(),
            telemetry: telemetry.clone(),
            external_grant_id,
            command,
            result_tx,
        }) {
            Ok(()) => {
                let queue_depth_after = self.queue_limit.saturating_sub(lane.capacity());
                log_lane_enqueued(
                    &telemetry,
                    "agent",
                    &lane_key,
                    self.queue_limit,
                    queue_depth_before,
                    queue_depth_after,
                );
            }
            Err(error) => {
                let message = format!("agent command lane overloaded: {error}");
                log_lane_enqueue_failed(
                    &telemetry,
                    "agent",
                    &lane_key,
                    self.queue_limit,
                    queue_depth_before,
                    &message,
                );
                return Err(DaemonError::LocalTransport {
                    operation: "enqueue agent kernel command",
                    message,
                });
            }
        }
        result_rx
            .await
            .map_err(|error| DaemonError::LocalTransport {
                operation: "await agent kernel command",
                message: error.to_string(),
            })?
    }

    async fn agent_lane(&self, agent_id: &str) -> mpsc::Sender<AgentCommandEnvelope> {
        let mut lanes = self.lanes.lock().await;
        if let Some(lane) = lanes.get(agent_id) {
            return lane.clone();
        }
        let (tx, rx) = mpsc::channel(AGENT_COMMAND_QUEUE_LIMIT);
        lanes.insert(agent_id.to_string(), tx.clone());
        let prompt_commands = self
            .store
            .prompt_command_service(self.provider_runtime_lanes.clone());
        let executor = AgentRuntimeCommandExecutor::new(
            prompt_commands,
            self.session_projection.clone(),
            self.agent_runtime_projection.clone(),
            self.prompt_id_allocator.clone(),
        );
        tokio::spawn(run_agent_command_lane(
            executor,
            self.store.state.clone(),
            agent_id.to_string(),
            rx,
        ));
        tx
    }

    #[allow(dead_code)]
    pub(crate) async fn queue_snapshots(&self) -> Vec<ActorQueueSnapshot> {
        let lanes = self.lanes.lock().await;
        let mut snapshots = lanes
            .iter()
            .map(|(agent_id, sender)| {
                ActorQueueSnapshot::new(
                    agent_id.clone(),
                    self.queue_limit,
                    self.queue_limit.saturating_sub(sender.capacity()),
                )
            })
            .collect::<Vec<_>>();
        snapshots.sort_by(|left, right| left.lane_id.cmp(&right.lane_id));
        snapshots
    }

    pub(crate) async fn remove_agent_lane(&self, agent_id: &str) {
        self.lanes.lock().await.remove(agent_id);
    }

    pub(crate) async fn remove_agent_lanes<'a>(
        &self,
        agent_ids: impl IntoIterator<Item = &'a str>,
    ) {
        let mut lanes = self.lanes.lock().await;
        for agent_id in agent_ids {
            lanes.remove(agent_id);
        }
    }
}

pub(super) async fn run_agent_command_lane(
    executor: AgentRuntimeCommandExecutor,
    state: KernelRuntimeState,
    agent_id: String,
    mut rx: mpsc::Receiver<AgentCommandEnvelope>,
) {
    while let Some(envelope) = rx.recv().await {
        let dispatch_started_at_ms = crate::runtime::command_latency::now_ms();
        log_lane_dispatched(
            &envelope.telemetry,
            "agent",
            &agent_id,
            dispatch_started_at_ms,
        );
        crate::logging::info_with_fields(
            "daemon.kernel_agent_actor",
            "agent kernel command dispatched",
            serde_json::json!({
                "agent_id": agent_id,
                "command_id": envelope.command_id,
                "command_type": envelope.command_type,
            }),
        );
        let authorization = envelope
            .external_grant_id
            .as_deref()
            .map(|id| state.authorize_external_request(id, &envelope.command.local_request()))
            .transpose();
        let result = match authorization {
            Err(error) => Err(error),
            Ok(_) => {
                let executor = match envelope.external_grant_id {
                    Some(id) => {
                        executor.with_external_authority(id, envelope.command.local_request())
                    }
                    None => executor.clone(),
                };
                executor.execute(envelope.command).await
            }
        };
        log_lane_completed(
            &envelope.telemetry,
            "agent",
            &agent_id,
            dispatch_started_at_ms,
            &result,
        );
        let _ = envelope.result_tx.send(result);
    }
}

impl AgentCommand {
    fn local_request(&self) -> crate::local::LocalDaemonRequest {
        use crate::local::LocalDaemonRequest;
        match self {
            Self::SubmitPrompt { request, .. } => LocalDaemonRequest::SubmitPrompt(request.clone()),
            Self::CompletePrompt { request, .. } => {
                LocalDaemonRequest::CompletePrompt(request.clone())
            }
            Self::CancelActivePrompt { request, .. } => {
                LocalDaemonRequest::CancelActivePrompt(request.clone())
            }
            Self::SteerQueuedPrompt { request } => {
                LocalDaemonRequest::SteerQueuedPrompt(request.clone())
            }
            Self::CancelQueuedPrompt { request } => {
                LocalDaemonRequest::CancelQueuedPrompt(request.clone())
            }
            Self::UpdateQueuedPrompt { request } => {
                LocalDaemonRequest::UpdateQueuedPrompt(request.clone())
            }
        }
    }
}
