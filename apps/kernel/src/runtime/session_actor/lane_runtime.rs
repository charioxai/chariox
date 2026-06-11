use std::collections::HashMap;
use std::sync::Arc;

use tokio::sync::{mpsc, oneshot, Mutex};

use crate::error::DaemonError;
use crate::local::{LocalDaemonRequest, LocalDaemonResponse};
use crate::runtime::command::{KernelCallerKind, KernelCommand};
use crate::runtime::command_latency::{
    log_lane_completed, log_lane_dispatched, log_lane_enqueue_failed, log_lane_enqueued,
    LaneCommandTrace,
};
use crate::runtime::projection::{
    ActorQueueSnapshot, AgentRuntimeProjectionStore, SessionStateProjectionStore,
};
use crate::runtime::state::KernelRuntimeState;
use crate::session::DEFAULT_LOCAL_USER_ID;
use crate::terminal::TerminalStreamStore;

use super::command_executor::SessionRuntimeCommandExecutor;
use super::focus_projection::FocusedAgentProjection;
use super::lane_resolution;
use super::store::SessionRuntimeStore;

#[derive(Debug)]
struct SessionCommandEnvelope {
    command_id: String,
    command_type: String,
    telemetry: LaneCommandTrace,
    caller_user_id: String,
    caller_is_metaagent: bool,
    request: LocalDaemonRequest,
    result_tx: oneshot::Sender<Result<LocalDaemonResponse, DaemonError>>,
}

#[derive(Clone)]
pub(crate) struct SessionRuntime {
    store: SessionRuntimeStore,
    queue_limit: usize,
    focus_projection: FocusedAgentProjection,
    session_projection: SessionStateProjectionStore,
    agent_runtime_projection: AgentRuntimeProjectionStore,
    terminal_stream: TerminalStreamStore,
    lanes: Arc<Mutex<HashMap<String, mpsc::Sender<SessionCommandEnvelope>>>>,
}

impl SessionRuntime {
    pub(crate) fn with_queue_limit_and_focus_projection(
        state: KernelRuntimeState,
        queue_limit: usize,
        focus_projection: FocusedAgentProjection,
        session_projection: SessionStateProjectionStore,
        agent_runtime_projection: AgentRuntimeProjectionStore,
        terminal_stream: TerminalStreamStore,
    ) -> Self {
        Self::with_store_and_focus_projection(
            SessionRuntimeStore::new(state),
            queue_limit,
            focus_projection,
            session_projection,
            agent_runtime_projection,
            terminal_stream,
        )
    }

    pub(crate) fn with_store_and_focus_projection(
        store: SessionRuntimeStore,
        queue_limit: usize,
        focus_projection: FocusedAgentProjection,
        session_projection: SessionStateProjectionStore,
        agent_runtime_projection: AgentRuntimeProjectionStore,
        terminal_stream: TerminalStreamStore,
    ) -> Self {
        Self {
            store,
            queue_limit,
            focus_projection,
            session_projection,
            agent_runtime_projection,
            terminal_stream,
            lanes: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub(crate) async fn dispatch_session_command(
        &self,
        command: KernelCommand,
        request: LocalDaemonRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let session_id = self.resolve_session_lane_key(&request).await?;
        let lane = self.session_lane(&session_id).await;
        let (result_tx, result_rx) = oneshot::channel();
        let caller_user_id = command_session_actor_user_id(&command);
        let telemetry = LaneCommandTrace::new(
            crate::runtime::command_latency::CommandTrace::from_command(&command),
            crate::runtime::command_latency::now_ms(),
        );
        let queue_depth_before = self.queue_limit.saturating_sub(lane.capacity());
        let command_id = command.command_id;
        let command_type = command.command_type;
        let caller_is_metaagent = command.caller.caller_id.starts_with("metaagent:");
        match lane.try_send(SessionCommandEnvelope {
            telemetry: telemetry.clone(),
            command_id,
            command_type,
            caller_user_id,
            caller_is_metaagent,
            request,
            result_tx,
        }) {
            Ok(()) => {
                let queue_depth_after = self.queue_limit.saturating_sub(lane.capacity());
                log_lane_enqueued(
                    &telemetry,
                    "session",
                    &session_id,
                    self.queue_limit,
                    queue_depth_before,
                    queue_depth_after,
                );
            }
            Err(error) => {
                let message = format!("session command lane overloaded: {error}");
                log_lane_enqueue_failed(
                    &telemetry,
                    "session",
                    &session_id,
                    self.queue_limit,
                    queue_depth_before,
                    &message,
                );
                return Err(DaemonError::LocalTransport {
                    operation: "enqueue session kernel command",
                    message,
                });
            }
        }
        let response = result_rx
            .await
            .map_err(|error| DaemonError::LocalTransport {
                operation: "await session kernel command",
                message: error.to_string(),
            })??;
        if matches!(
            response,
            LocalDaemonResponse::SessionEnded { .. } | LocalDaemonResponse::SessionDeleted { .. }
        ) {
            self.remove_session_lane(&session_id).await;
        }
        Ok(response)
    }

    pub(super) async fn resolve_session_lane_key(
        &self,
        request: &LocalDaemonRequest,
    ) -> Result<String, DaemonError> {
        lane_resolution::resolve_session_lane_key(&self.store, &self.session_projection, request)
            .await
    }

    async fn session_lane(&self, session_id: &str) -> mpsc::Sender<SessionCommandEnvelope> {
        let mut lanes = self.lanes.lock().await;
        if let Some(lane) = lanes.get(session_id) {
            return lane.clone();
        }
        let (tx, rx) = mpsc::channel(self.queue_limit);
        lanes.insert(session_id.to_string(), tx.clone());
        tokio::spawn(run_session_command_lane(
            self.store.clone(),
            self.focus_projection.clone(),
            self.session_projection.clone(),
            self.agent_runtime_projection.clone(),
            self.terminal_stream.clone(),
            session_id.to_string(),
            rx,
        ));
        tx
    }

    async fn remove_session_lane(&self, session_id: &str) {
        let mut lanes = self.lanes.lock().await;
        lanes.remove(session_id);
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
        let lanes = self.lanes.lock().await;
        lanes.contains_key(session_id)
    }

    #[cfg(test)]
    pub(crate) async fn lane_capacity(&self, session_id: &str) -> Option<usize> {
        let lanes = self.lanes.lock().await;
        lanes.get(session_id).map(mpsc::Sender::capacity)
    }

    #[cfg(test)]
    pub(crate) async fn enqueue_for_test(
        &self,
        session_id: &str,
        command_id: impl Into<String>,
        command_type: impl Into<String>,
        request: LocalDaemonRequest,
    ) -> Result<oneshot::Receiver<Result<LocalDaemonResponse, DaemonError>>, DaemonError> {
        let lane = self.session_lane(session_id).await;
        let (result_tx, result_rx) = oneshot::channel();
        let command_id = command_id.into();
        let command_type = command_type.into();
        let mut command =
            KernelCommand::from_local_request(command_id.clone(), None, None, &request);
        command.command_type = command_type.clone();
        lane.try_send(SessionCommandEnvelope {
            command_id,
            command_type,
            telemetry: LaneCommandTrace::new(
                crate::runtime::command_latency::CommandTrace::from_command(&command),
                crate::runtime::command_latency::now_ms(),
            ),
            caller_user_id: DEFAULT_LOCAL_USER_ID.to_string(),
            caller_is_metaagent: false,
            request,
            result_tx,
        })
        .map_err(|error| DaemonError::LocalTransport {
            operation: "enqueue test session kernel command",
            message: format!("session command lane overloaded: {error}"),
        })?;
        Ok(result_rx)
    }
}

async fn run_session_command_lane(
    store: SessionRuntimeStore,
    focus_projection: FocusedAgentProjection,
    session_projection: SessionStateProjectionStore,
    agent_runtime_projection: AgentRuntimeProjectionStore,
    terminal_stream: TerminalStreamStore,
    session_id: String,
    mut rx: mpsc::Receiver<SessionCommandEnvelope>,
) {
    let executor = SessionRuntimeCommandExecutor::new(
        store,
        focus_projection,
        session_projection,
        agent_runtime_projection,
        terminal_stream,
        session_id.clone(),
    );
    while let Some(envelope) = rx.recv().await {
        let dispatch_started_at_ms = crate::runtime::command_latency::now_ms();
        log_lane_dispatched(
            &envelope.telemetry,
            "session",
            &session_id,
            dispatch_started_at_ms,
        );
        crate::logging::info_with_fields(
            "daemon.kernel_session_actor",
            "session kernel command dispatched",
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
                envelope.caller_is_metaagent,
            )
            .await;
        log_lane_completed(
            &envelope.telemetry,
            "session",
            &session_id,
            dispatch_started_at_ms,
            &result,
        );
        match &result {
            Ok(response) => crate::logging::info_with_fields(
                "daemon.kernel_session_actor",
                "session kernel command completed",
                serde_json::json!({
                    "session_id": session_id,
                    "command_id": envelope.command_id,
                    "command_type": envelope.command_type,
                    "response_kind": local_response_kind(response),
                }),
            ),
            Err(error) => crate::logging::warn_with_fields(
                "daemon.kernel_session_actor",
                "session kernel command failed",
                serde_json::json!({
                    "session_id": session_id,
                    "command_id": envelope.command_id,
                    "command_type": envelope.command_type,
                    "error": error.to_string(),
                }),
            ),
        }
        let _ = envelope.result_tx.send(result);
    }
}

fn command_session_actor_user_id(command: &KernelCommand) -> String {
    match command.caller.caller_kind {
        KernelCallerKind::LocalClient => command
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

fn local_response_kind(response: &LocalDaemonResponse) -> &'static str {
    match response {
        LocalDaemonResponse::SessionCreated { .. } => "SessionCreated",
        LocalDaemonResponse::SessionAttached { .. } => "SessionAttached",
        LocalDaemonResponse::SessionDetached { .. } => "SessionDetached",
        LocalDaemonResponse::SessionConfigUpdated { .. } => "SessionConfigUpdated",
        LocalDaemonResponse::SessionAliased { .. } => "SessionAliased",
        LocalDaemonResponse::SessionEnded { .. } => "SessionEnded",
        LocalDaemonResponse::SessionDeleted { .. } => "SessionDeleted",
        LocalDaemonResponse::AgentSpawned { .. } => "AgentSpawned",
        LocalDaemonResponse::AgentDestroyed { .. } => "AgentDestroyed",
        LocalDaemonResponse::AgentFocused { .. } => "AgentFocused",
        LocalDaemonResponse::AgentOutputSeenAcknowledged { .. } => "AgentOutputSeenAcknowledged",
        LocalDaemonResponse::AgentFocusCycled { .. } => "AgentFocusCycled",
        LocalDaemonResponse::AgentAliased { .. } => "AgentAliased",
        LocalDaemonResponse::AgentConfigUpdated { .. } => "AgentConfigUpdated",
        LocalDaemonResponse::AgentProfileUpdated { .. } => "AgentProfileUpdated",
        LocalDaemonResponse::TerminalResized { .. } => "TerminalResized",
        LocalDaemonResponse::RuntimeNotices { .. } => "RuntimeNotices",
        _ => "Other",
    }
}
