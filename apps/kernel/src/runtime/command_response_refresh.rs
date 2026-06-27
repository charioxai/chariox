use std::sync::Arc;

use tokio::sync::Mutex;

use crate::app::DaemonApp;
use crate::error::DaemonError;
use crate::local::LocalDaemonResponse;
use crate::runtime::agent_actor::AgentRuntime;
use crate::runtime::projection::{
    AgentRuntimeProjectionStore, ProviderProcessProjectionStore, ProviderRunProjectionStore,
    SessionStateProjectionStore,
};
use crate::runtime::provider_launch_executor::ProviderLaunchPendingTracker;
use crate::runtime::provider_run_control::refresh_provider_run_projection_from_response;
use crate::runtime::runtime_lane_cleanup::cleanup_runtime_lanes_after_response;
use crate::runtime::session_actor::FocusedAgentProjection;
use crate::runtime::session_projection_refresh::{
    apply_focus_projection_refresh, apply_session_projection_refresh, FocusProjectionRefresh,
    SessionProjectionRefresh, SessionProjectionRefreshContext,
};
use crate::runtime::workflow_actor::WorkflowRuntime;

pub(crate) struct CommandResponseRefreshContext<'a> {
    pub(crate) app: &'a Arc<Mutex<DaemonApp>>,
    pub(crate) session_projection: &'a SessionStateProjectionStore,
    pub(crate) agent_runtime_projection: &'a AgentRuntimeProjectionStore,
    pub(crate) focus_projection: &'a FocusedAgentProjection,
    pub(crate) provider_process_projection: &'a ProviderProcessProjectionStore,
    pub(crate) provider_launch_pending: &'a ProviderLaunchPendingTracker,
    pub(crate) provider_run_projection: &'a ProviderRunProjectionStore,
    pub(crate) agent_runtime: &'a AgentRuntime,
    pub(crate) workflow_runtime: &'a WorkflowRuntime,
}

pub(crate) async fn refresh_command_response_state(
    context: CommandResponseRefreshContext<'_>,
    session_refresh: SessionProjectionRefresh,
    focus_refresh: FocusProjectionRefresh,
    result: &Result<LocalDaemonResponse, DaemonError>,
) {
    apply_session_projection_refresh(
        SessionProjectionRefreshContext {
            app: context.app,
            session_projection: context.session_projection,
            agent_runtime_projection: context.agent_runtime_projection,
            provider_process_projection: context.provider_process_projection,
            provider_launch_pending: context.provider_launch_pending,
            provider_run_projection: context.provider_run_projection,
        },
        session_refresh,
        result,
    )
    .await;
    apply_focus_projection_refresh(
        context.app,
        context.focus_projection,
        context.session_projection,
        focus_refresh,
        result,
    )
    .await;
    refresh_provider_run_projection_from_response(
        context.provider_run_projection,
        context.provider_process_projection,
        result,
    );
    context.provider_launch_pending.track_response(result).await;
    cleanup_runtime_lanes_after_response(context.agent_runtime, context.workflow_runtime, result)
        .await;
}
