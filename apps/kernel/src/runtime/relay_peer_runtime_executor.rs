use std::sync::Arc;

use tokio::sync::Mutex;

use crate::agent::GitWorktreePlacement;
use crate::app::{DaemonApp, KernelSessionReadService, RemoteLeaseRuntime};
use crate::error::DaemonError;
use crate::execution_lease::{ExecutionLease, LeasedAgent, RemoteWorkflowTurnContext};
use crate::provider::{AgentExecutionMode, AgentPermissionLevel};
use crate::runtime::projection::{
    ProviderRunProjectionStore, SessionSnapshotProjection, SessionStateProjectionStore,
};
use crate::runtime_transport::WatchResult;
use crate::session::{PromptCancellation, PromptCompletion, PromptSubmissionOutcome};
use crate::skill::ArrobaSkillPackage;
use crate::transport::relay_peer::{
    RelayPeerEvent, RelayProjectedCompletion, RelayProjectedOutputChunk, RelayProjectedPrompt,
    RelayPromptAttachment, RemoteGitObservation, RemoteGitTurnContext, RemoteMcpAvailability,
    RemoteMcpCheckContext, RemoteSkillMaterialization, RemoteSkillSyncContext, RequiredRemoteMcp,
};

pub(crate) async fn ensure_relay_subscription_attachment(
    app: &Arc<Mutex<DaemonApp>>,
    session_projection: &SessionStateProjectionStore,
    session_id: &str,
    attachment_id: &str,
) -> Result<(), DaemonError> {
    if let Some(session) = session_projection.get(session_id) {
        if session.has_attachment(attachment_id) {
            return Ok(());
        }
        return Err(DaemonError::AttachmentNotInSession {
            session_id: session_id.to_string(),
            attachment_id: attachment_id.to_string(),
        });
    }
    let app = app.lock().await;
    KernelSessionReadService::new(&app)
        .ensure_attachment_in_session(session_id, attachment_id)
        .map(|_| ())
}

pub(crate) async fn watch_relay_subscription_state(
    app: &Arc<Mutex<DaemonApp>>,
    session_id: &str,
    attachment_id: &str,
    tick: u64,
    previous_snapshot: Option<SessionSnapshotProjection>,
    last_workflow_design_sequence: u64,
) -> WatchResult {
    let mut app = app.lock().await;
    crate::runtime_transport::watch_subscription_state(
        &mut app,
        session_id,
        attachment_id,
        tick,
        previous_snapshot,
        last_workflow_design_sequence,
    )
}

pub(crate) async fn create_relay_execution_lease(
    app: &Arc<Mutex<DaemonApp>>,
    home_kernel_id: &str,
    home_session_id: &str,
    home_agent_id: &str,
    owner_user_id: &str,
) -> Result<ExecutionLease, DaemonError> {
    let mut app = app.lock().await;
    RemoteLeaseRuntime::new(&mut app).create_execution_lease(
        home_kernel_id,
        home_session_id,
        home_agent_id,
        owner_user_id,
    )
}

pub(crate) async fn destroy_relay_execution_lease(
    app: &Arc<Mutex<DaemonApp>>,
    lease_id: &str,
) -> Result<ExecutionLease, DaemonError> {
    let mut app = app.lock().await;
    RemoteLeaseRuntime::new(&mut app).destroy_execution_lease(lease_id)
}

pub(crate) async fn create_relay_leased_agent(
    app: &Arc<Mutex<DaemonApp>>,
    lease_id: &str,
    provider: &str,
    model: Option<String>,
    effort: Option<String>,
    execution_mode: Option<AgentExecutionMode>,
    permission_level: Option<AgentPermissionLevel>,
    worktree_id: Option<String>,
    worktree_placement: Option<GitWorktreePlacement>,
) -> Result<LeasedAgent, DaemonError> {
    let mut app = app.lock().await;
    RemoteLeaseRuntime::new(&mut app).create_leased_agent(
        lease_id,
        provider,
        model,
        effort,
        execution_mode,
        permission_level,
        worktree_id,
        worktree_placement,
    )
}

pub(crate) async fn destroy_relay_leased_agent(
    app: &Arc<Mutex<DaemonApp>>,
    leased_agent_id: &str,
) -> Result<LeasedAgent, DaemonError> {
    let mut app = app.lock().await;
    RemoteLeaseRuntime::new(&mut app).destroy_leased_agent(leased_agent_id)
}

pub(crate) async fn update_relay_leased_agent_config(
    app: &Arc<Mutex<DaemonApp>>,
    leased_agent_id: &str,
    execution_mode: AgentExecutionMode,
    permission_level: AgentPermissionLevel,
) -> Result<LeasedAgent, DaemonError> {
    let mut app = app.lock().await;
    RemoteLeaseRuntime::new(&mut app).update_leased_agent_config(
        leased_agent_id,
        execution_mode,
        permission_level,
    )
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn launch_relay_leased_native_provider_run(
    app: &Arc<Mutex<DaemonApp>>,
    leased_agent_id: &str,
    adapter_key: &str,
    provider: &str,
    account_profile: &str,
    model: &str,
    variant: Option<String>,
    structured_endpoint: Option<String>,
    provider_session_id: Option<String>,
    required_mcps: Vec<crate::transport::relay_peer::RequiredRemoteMcp>,
) -> Result<crate::provider::RuntimeProviderRun, DaemonError> {
    let mut app = app.lock().await;
    RemoteLeaseRuntime::new(&mut app).launch_leased_native_provider_run(
        leased_agent_id,
        adapter_key,
        provider,
        account_profile,
        model,
        variant,
        structured_endpoint,
        provider_session_id,
        required_mcps,
    )
}

pub(crate) async fn send_relay_leased_native_provider_input(
    app: &Arc<Mutex<DaemonApp>>,
    leased_agent_id: &str,
    provider_run_id: &str,
    attachment_id: &str,
    data_base64: &str,
) -> Result<usize, DaemonError> {
    let mut app = app.lock().await;
    RemoteLeaseRuntime::new(&mut app).send_leased_native_provider_input(
        leased_agent_id,
        provider_run_id,
        attachment_id,
        data_base64,
    )
}

pub(crate) async fn submit_relay_leased_prompt(
    app: &Arc<Mutex<DaemonApp>>,
    leased_agent_id: &str,
    prompt: &str,
    attachments: Vec<RelayPromptAttachment>,
    workflow_context: Option<RemoteWorkflowTurnContext>,
    git_context: Option<RemoteGitTurnContext>,
    required_mcps: Vec<RequiredRemoteMcp>,
) -> Result<(String, PromptSubmissionOutcome), DaemonError> {
    let mut app = app.lock().await;
    RemoteLeaseRuntime::new(&mut app).submit_leased_prompt_with_workflow_context(
        leased_agent_id,
        prompt,
        attachments,
        workflow_context,
        git_context,
        required_mcps,
    )
}

pub(crate) async fn ensure_relay_remote_skill_packages(
    app: &Arc<Mutex<DaemonApp>>,
    context: RemoteSkillSyncContext,
    packages: Vec<ArrobaSkillPackage>,
) -> Result<Vec<RemoteSkillMaterialization>, DaemonError> {
    let mut app = app.lock().await;
    RemoteLeaseRuntime::new(&mut app).ensure_remote_skill_packages(context, packages)
}

pub(crate) async fn check_relay_remote_mcp_availability(
    app: &Arc<Mutex<DaemonApp>>,
    context: RemoteMcpCheckContext,
    required_mcps: Vec<RequiredRemoteMcp>,
) -> Result<Vec<RemoteMcpAvailability>, DaemonError> {
    let mut app = app.lock().await;
    RemoteLeaseRuntime::new(&mut app).check_remote_mcp_availability(context, required_mcps)
}

pub(crate) async fn complete_relay_leased_prompt(
    app: &Arc<Mutex<DaemonApp>>,
    leased_agent_id: &str,
) -> Result<PromptCompletion, DaemonError> {
    let mut app = app.lock().await;
    RemoteLeaseRuntime::new(&mut app).complete_leased_prompt(leased_agent_id)
}

pub(crate) async fn observe_relay_leased_git_after(
    app: &Arc<Mutex<DaemonApp>>,
    leased_agent_id: &str,
    provider_run_id: &str,
) -> Result<Vec<RemoteGitObservation>, DaemonError> {
    let mut app = app.lock().await;
    RemoteLeaseRuntime::new(&mut app).observe_leased_git_after(leased_agent_id, provider_run_id)
}

pub(crate) async fn cancel_relay_leased_prompt(
    app: &Arc<Mutex<DaemonApp>>,
    leased_agent_id: &str,
) -> Result<PromptCancellation, DaemonError> {
    let mut app = app.lock().await;
    RemoteLeaseRuntime::new(&mut app).cancel_leased_prompt(leased_agent_id)
}

pub(crate) async fn relay_leased_agent_provider_run_id(
    app: &Arc<Mutex<DaemonApp>>,
    leased_agent_id: &str,
) -> Result<Option<String>, DaemonError> {
    let mut app = app.lock().await;
    RemoteLeaseRuntime::new(&mut app).leased_agent_provider_run_id(leased_agent_id)
}

pub(crate) fn relay_provider_run_terminal_diagnostic(
    provider_run_projection: &ProviderRunProjectionStore,
    provider_run_id: &str,
) -> Option<String> {
    provider_run_projection
        .get(provider_run_id)
        .and_then(|run| run.terminal_diagnostic().map(str::to_string))
        .filter(|message| !message.trim().is_empty())
}

pub(crate) async fn pump_relay_leased_runtime_projections(
    app: &Arc<Mutex<DaemonApp>>,
) -> Result<Vec<(String, RelayPeerEvent)>, DaemonError> {
    let mut app = app.lock().await;
    RemoteLeaseRuntime::new(&mut app).pump_leased_runtime_projections()
}

pub(crate) async fn drain_relay_leased_runtime_projection(
    app: &Arc<Mutex<DaemonApp>>,
    leased_agent_id: &str,
    provider_run_id: &str,
    pump_output: bool,
) -> Result<Option<(String, RelayPeerEvent)>, DaemonError> {
    let mut app = app.lock().await;
    RemoteLeaseRuntime::new(&mut app).drain_leased_runtime_projection(
        leased_agent_id,
        provider_run_id,
        pump_output,
    )
}

pub(crate) async fn project_relay_remote_runtime_projection(
    app: &Arc<Mutex<DaemonApp>>,
    session_id: &str,
    agent_id: &str,
    provider_run_id: &str,
    prompts: Vec<RelayProjectedPrompt>,
    output_chunks: Vec<RelayProjectedOutputChunk>,
    notices: Vec<String>,
    completions: Vec<RelayProjectedCompletion>,
) -> Result<(), DaemonError> {
    let mut app = app.lock().await;
    RemoteLeaseRuntime::new(&mut app).project_remote_runtime_projection(
        session_id,
        agent_id,
        provider_run_id,
        prompts,
        output_chunks,
        notices,
        completions,
    )
}
