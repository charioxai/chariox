use std::sync::Arc;
use tokio::sync::Mutex;
use tokio::sync::RwLock;

use crate::app::DaemonApp;
use crate::history::OperationalHistoryStore;
use crate::history::SessionHistoryStore;
use crate::provider::ProviderRunOperationLanes;
use crate::runtime::agent_actor::AgentRuntime;
use crate::runtime::capability_executor::{CapabilityExecutorHealthStore, CapabilityRuntimeStore};
use crate::runtime::credential_enrollment_control::CredentialEnrollmentControl;
use crate::runtime::projection::{
    AgentRuntimeProjectionStore, DaemonConfigProjectionStore, ProviderCatalogProjectionStore,
    ProviderProcessProjectionStore, ProviderRunProjectionStore,
    RemoteRelayInventoryProjectionStore, SessionStateProjectionStore, TransportHealthStore,
};
use crate::runtime::provider_launch_executor::ProviderLaunchPendingTracker;
use crate::runtime::session_actor::{FocusedAgentProjection, SessionRuntime};
use crate::runtime::state::KernelRuntimeState;
use crate::runtime::terminal_output_executor::TerminalOutputExecutor;
use crate::runtime::waiting_room_public_projection::WaitingRoomSessionSummaryProjectionStore;
use crate::runtime::workflow_actor::WorkflowRuntime;
use crate::runtime::workspace_coordinator::WorkspaceCoordinator;
use crate::terminal::TerminalStreamHealthStore;
use crate::transport::relay_client::RelayClientState;

mod caller_identity_bridge;
mod cloud_relay_bridge;
mod composition;
mod dispatch;
mod meta_runtime_command;
mod pre_lane_dispatch;
mod priority_dispatch;
mod refresh_dispatch;
mod relay_peer_bridge;
mod runtime_tool_bridge;
mod status_projection_bridge;
mod transport_bridge;

pub(crate) const INTERACTIVE_COMMAND_QUEUE_LIMIT: usize = 128;

#[derive(Clone)]
pub(crate) struct CommandRouter {
    app: Arc<Mutex<DaemonApp>>,
    runtime_state: KernelRuntimeState,
    agent_runtime: AgentRuntime,
    session_runtime: SessionRuntime,
    workflow_runtime: WorkflowRuntime,
    provider_runtime_lanes: ProviderRunOperationLanes,
    focus_projection: FocusedAgentProjection,
    session_projection: SessionStateProjectionStore,
    waiting_room_session_summaries: WaitingRoomSessionSummaryProjectionStore,
    agent_runtime_projection: AgentRuntimeProjectionStore,
    history_store: SessionHistoryStore,
    operational_history_store: OperationalHistoryStore,
    provider_catalog_projection: ProviderCatalogProjectionStore,
    provider_run_projection: ProviderRunProjectionStore,
    provider_process_projection: ProviderProcessProjectionStore,
    credential_enrollment_control: CredentialEnrollmentControl,
    #[allow(dead_code)]
    active_turns: crate::app::ActiveTurnStore,
    config_projection: DaemonConfigProjectionStore,
    remote_relay_inventory_projection: RemoteRelayInventoryProjectionStore,
    relay_state: Arc<RwLock<RelayClientState>>,
    capability_health: CapabilityExecutorHealthStore,
    capability_runtime: CapabilityRuntimeStore,
    transport_health: TransportHealthStore,
    terminal_health: TerminalStreamHealthStore,
    terminal_output_executor: TerminalOutputExecutor,
    workspace_coordinator: WorkspaceCoordinator,
    provider_launch_pending: ProviderLaunchPendingTracker,
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::sync::Arc;

    use tokio::sync::Mutex;
    use tokio::time::{timeout, Duration};

    use crate::agent::CreateAgentRequest;
    use crate::attachment::ClientCapabilityLevel;
    use crate::local::{
        AckMetaagentEventsRequest, AcknowledgeAgentOutputSeenRequest, AddWorkflowEdgeRequest,
        AddWorkflowNodeRequest, AliasSessionRequest, AttachToSessionRequest,
        AttachWorkspaceLinkRequest, CancelActivePromptRequest, CancelQueuedPromptRequest,
        CompletePromptRequest, CreateWorkflowEndpointRequest, CreateWorkflowRequest,
        CreateWorkspaceLinkRequest, CycleAgentFocusRequest, DeleteKernelRequest,
        DeleteSessionRequest, DestroyAgentRequest, DetachFromSessionRequest, EndSessionRequest,
        FocusAgentRequest, GetDaemonHealthRequest, GetMetaagentTurnBlobRequest,
        GetMetaagentTurnOverviewRequest, GetProviderAuthStatusRequest, GetProviderCatalogRequest,
        GetProviderCommandCatalogsRequest, GetProviderRunRequest, GetSessionStateRequest,
        GetWorkspaceLiveSyncStatusRequest, InvokeWorkflowEndpointRequest, LaunchProviderRunRequest,
        ListAgentsRequest, ListMetaagentEventsRequest, ListProviderProcessesRequest,
        ListSessionsRequest, ListWorkflowRunsRequest, ListWorkflowWatchdogsRequest,
        ListWorkflowsRequest, LocalDaemonRequest, LocalDaemonResponse, PollRuntimeNoticesRequest,
        PumpTerminalOutputRequest, ReadMetaagentEventRequest, RelayStatusRequest,
        RemoveWorkflowEdgeRequest, ResizeTerminalRequest, ResolveSessionRequest,
        ResolveWorkflowRequest, RunShellCapabilityRequest, SearchMetaagentCommandsRequest,
        SpawnAgentRequest, SteerQueuedPromptRequest, SubmitPromptRequest,
        TeardownProviderProcessesRequest, UpdateSessionConfigRequest,
    };
    use crate::provider::{
        LaunchProviderRequest, OpenCodeProviderCatalog, OpenCodeProviderInfo,
        ProviderClientInterface, RuntimeProviderRun,
    };
    use crate::runtime::command::{
        KernelCaller, KernelCallerKind, KernelCommand, KernelCommandSource,
    };
    use crate::runtime::projection::SESSION_SNAPSHOT_PROJECTION_VERSION;
    use crate::runtime::router::CommandRouter;
    use crate::session::{
        CreateSessionRequest, PromptOrigin, PromptQueueItem, PromptStatus, PromptSubmissionOutcome,
        RuntimeInteraction, RuntimeInteractionChoice, RuntimeInteractionChoiceStyle,
        RuntimeInteractionKind, RuntimeInteractionLevel, SessionStatus, DEFAULT_LOCAL_USER_ID,
    };
    use crate::{DaemonApp, DaemonConfig, DaemonError};

    fn spawn_test_agent(
        app: &mut DaemonApp,
        session_id: &str,
        alias: &str,
        provider: &str,
    ) -> crate::agent::AgentInstance {
        crate::app::KernelSessionService::new(app)
            .spawn_agent(CreateAgentRequest::new(session_id, provider).with_alias(alias))
            .expect("agent should spawn")
    }

    fn launch_test_provider(
        app: &mut DaemonApp,
        session_id: &str,
        agent_id: &str,
        adapter_key: &str,
        provider: &str,
        model: &str,
    ) -> RuntimeProviderRun {
        let provider_run = app
            .launch_provider(
                LaunchProviderRequest::new(session_id, adapter_key, provider, "default", model)
                    .with_agent_id(agent_id),
            )
            .expect("provider run should launch");
        app.update_provider_run_projection(provider_run.clone());
        provider_run
    }

    fn remote_command_for_request(
        request: &LocalDaemonRequest,
        user_id: Option<&str>,
    ) -> KernelCommand {
        KernelCommand::from_local_request_with_caller(
            "remote-command",
            KernelCommandSource::RelayClient,
            KernelCaller {
                caller_id: "client-remote".to_string(),
                caller_kind: KernelCallerKind::RemoteClient,
                user_id: user_id.map(str::to_string),
                client_id: Some("client-remote".to_string()),
                machine_id: None,
                realm_id: Some("realm-1".to_string()),
                public_key_thumbprint: Some("thumbprint-remote".to_string()),
                metaagent_id: None,
            },
            None,
            None,
            request,
        )
    }

    fn focus_test_agent(app: &mut DaemonApp, session_id: &str, agent_id: &str) {
        crate::app::KernelSessionService::new(app)
            .focus_agent(session_id, agent_id)
            .expect("focus should succeed");
    }

    mod agent_messaging;
    mod agent_prompt_schedules;
    mod credential_enrollment;
    mod interactive_command_admission;
    mod m16_runtime_extension_registration;
    mod m23_metaagent_runtime_tools;
    mod provider_projection;
    mod relay_leased_prompt_steer;
    mod remote_authorization;
    mod remote_workspace_live_sync_authorization;
    mod runtime_persistence;
    mod session_actor_projection;
    mod session_lifecycle_projection;
    mod session_read_projection;
    mod status_projection;
    mod terminal_output_projection;
    mod workflow_revision;

    fn attach_request(session_id: &str, client_id: &str) -> LocalDaemonRequest {
        LocalDaemonRequest::AttachToSession(AttachToSessionRequest {
            session_id: session_id.to_string(),
            client_id: client_id.to_string(),
            capability_level: ClientCapabilityLevel::FullTerminal,
        })
    }

    fn focus_request(session_id: &str, agent_id: &str) -> LocalDaemonRequest {
        LocalDaemonRequest::FocusAgent(FocusAgentRequest {
            session_id: session_id.to_string(),
            agent_id: agent_id.to_string(),
        })
    }

    fn assert_ownership_denied(error: DaemonError, user_id: &str, owner_user_id: &str) {
        assert!(
            matches!(
                error,
                DaemonError::OwnershipAccessDenied {
                    user_id: ref denied_user,
                    owner_user_id: ref denied_owner,
                    ..
                } if denied_user == user_id && denied_owner == owner_user_id
            ),
            "unexpected error: {error:?}"
        );
    }
}
