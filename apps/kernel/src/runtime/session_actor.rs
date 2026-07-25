use crate::local::LocalDaemonRequest;

mod command_executor;
mod focus_projection;
mod lane_resolution;
mod lane_runtime;
mod projection_policy;
mod store;

pub(crate) use focus_projection::FocusedAgentProjection;
pub(crate) use lane_runtime::SessionRuntime;

pub(crate) const SESSION_COMMAND_QUEUE_LIMIT: usize = 128;
pub(crate) const SESSION_CREATE_LANE_ID: &str = "__session_create__";

pub(crate) struct SessionActor;

impl SessionActor {
    pub(crate) fn is_session_interactive_command(request: &LocalDaemonRequest) -> bool {
        matches!(
            request,
            LocalDaemonRequest::CreateSession(_)
                | LocalDaemonRequest::AttachToSession(_)
                | LocalDaemonRequest::DetachFromSession(_)
                | LocalDaemonRequest::FocusAgent(_)
                | LocalDaemonRequest::AcknowledgeAgentOutputSeen(_)
                | LocalDaemonRequest::CycleAgentFocus(_)
                | LocalDaemonRequest::ResizeTerminal(_)
                | LocalDaemonRequest::SendTerminalInput(_)
                | LocalDaemonRequest::PollRuntimeNotices(_)
                | LocalDaemonRequest::RespondToInteraction(_)
                | LocalDaemonRequest::UpdateSessionConfig(_)
                | LocalDaemonRequest::CreateAgentPromptSchedule(_)
                | LocalDaemonRequest::CancelAgentPromptSchedule(_)
                | LocalDaemonRequest::AliasAgent(_)
                | LocalDaemonRequest::UpdateAgentConfig(_)
                | LocalDaemonRequest::UpdateAgentProfile(_)
                | LocalDaemonRequest::UpdateAgentSubstitutes(_)
                | LocalDaemonRequest::AliasSession(_)
                | LocalDaemonRequest::SpawnAgent(_)
                | LocalDaemonRequest::SpawnAgents(_)
                | LocalDaemonRequest::UndoTurn(_)
                | LocalDaemonRequest::ForkAgent(_)
                | LocalDaemonRequest::DestroyAgent(_)
                | LocalDaemonRequest::EndSession(_)
                | LocalDaemonRequest::DeleteSession(_)
        )
    }
}

#[cfg(test)]
mod tests {
    use super::projection_policy::{
        projected_config_update_absence_response, projected_terminal_input_absence_response,
        session_response_projection_action, SessionProjectionAction,
    };
    use crate::agent::{AgentState, CreateAgentRequest};
    use crate::attachment::{AttachRequest, ClientCapabilityLevel};
    use crate::local::{
        AliasSessionRequest, AttachToSessionRequest, CycleAgentFocusRequest, DeleteSessionRequest,
        DestroyAgentRequest, DetachFromSessionRequest, EndSessionRequest,
        ExternalProviderSessionCapabilities, ExternalProviderSessionRecord, FocusAgentRequest,
        ListExternalProviderSessionsRequest, LocalDaemonRequest, LocalDaemonResponse,
        PollRuntimeNoticesRequest, ResizeTerminalRequest, SendTerminalInputRequest,
        UpdateAgentConfigRequest, UpdateSessionConfigRequest,
    };
    use crate::provider::{AgentExecutionMode, AgentPermissionLevel, LaunchProviderRequest};
    use crate::runtime::command::KernelCommand;
    use crate::runtime::projection::{AgentRuntimeProjectionStore, SessionStateProjectionStore};
    use crate::runtime::session_actor::{FocusedAgentProjection, SessionRuntime};
    use crate::runtime::state::KernelRuntimeState;
    use crate::session::{
        CreateSessionRequest, PromptOrigin, PromptQueueItem, PromptStatus, PromptSubmissionOutcome,
        SessionAgentDefaults, DEFAULT_LOCAL_USER_ID,
    };
    use crate::terminal::TerminalOutputKind;
    use crate::{DaemonApp, DaemonConfig, DaemonError};
    use std::sync::Arc;
    use tokio::sync::Mutex;
    use tokio::time::{timeout, Duration};

    fn launch_dev_stub_provider(
        app: &mut DaemonApp,
        session_id: &str,
        agent_id: &str,
        model: &str,
    ) -> crate::provider::RuntimeProviderRun {
        launch_provider_for_adapter(app, session_id, agent_id, "dev-stub", model)
    }

    fn launch_provider_for_adapter(
        app: &mut DaemonApp,
        session_id: &str,
        agent_id: &str,
        adapter_key: &str,
        model: &str,
    ) -> crate::provider::RuntimeProviderRun {
        let provider_run = app
            .launch_provider(
                LaunchProviderRequest::new(
                    session_id,
                    adapter_key,
                    "claude-code",
                    "default",
                    model,
                )
                .with_agent_id(agent_id),
            )
            .expect("provider launch should succeed");
        app.update_provider_run_projection(provider_run.clone());
        provider_run
    }

    async fn owned_runtime_state(app: &Arc<Mutex<DaemonApp>>) -> KernelRuntimeState {
        let (
            config_projection,
            session_store,
            agent_store,
            attachment_store,
            provider_store,
            provider_process_tracking,
            slice_store,
            session_projection,
            provider_run_projection,
            history_store,
            operational_history_store,
            durable_state_store,
            prompt_state_owner,
            active_turns,
            prompt_activity,
            prompt_workspace_claims,
            structured_output_records,
            terminal_stream,
            workflow_design_events,
            metaagent_events,
            workspace_coordinator,
        ) = {
            let app_locked = app.lock().await;
            (
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
        };
        KernelRuntimeState::new_with_owned_state(
            Arc::clone(app),
            config_projection,
            session_store,
            agent_store,
            attachment_store,
            provider_store,
            provider_process_tracking,
            slice_store,
            session_projection,
            provider_run_projection,
            history_store,
            operational_history_store,
            durable_state_store,
            prompt_state_owner,
            active_turns,
            prompt_activity,
            prompt_workspace_claims,
            structured_output_records,
            terminal_stream,
            workflow_design_events,
            metaagent_events,
            workspace_coordinator,
        )
    }
    mod agent_lifecycle_execution;
    mod focus_liveness;
    mod lane_resolution;
    mod projection_policy;
    mod session_command_execution;
}
