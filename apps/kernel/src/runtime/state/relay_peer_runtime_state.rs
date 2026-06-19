use crate::app::RemoteLeaseRuntime;
use crate::execution_lease::{ExecutionLease, LeasedAgent, RemoteWorkflowTurnContext};
use crate::provider::{AgentExecutionMode, AgentPermissionLevel};
use crate::runtime::projection::SessionSnapshotProjection;
use crate::runtime_transport::WatchResult;
use crate::skill::ArrobaSkillPackage;
use crate::transport::relay_peer::{
    RelayPeerEvent, RelayProjectedCompletion, RelayProjectedOutputChunk, RelayProjectedPrompt,
    RelayPromptAttachment, RemoteGitObservation, RemoteGitTurnContext, RemoteMcpAvailability,
    RemoteMcpCheckContext, RemoteSkillMaterialization, RemoteSkillSyncContext, RequiredRemoteMcp,
};

use super::*;

impl KernelRuntimeState {
    pub(crate) async fn relay_registration(&self) -> arroba_relay::protocol::DaemonRegistration {
        self.with_app_side_effect(|app| app.relay_registration())
            .await
    }

    pub(crate) async fn remote_native_interaction_context(
        &self,
        session_id: &str,
        agent_id: &str,
    ) -> Result<
        Option<(
            crate::config::DaemonConfig,
            String,
            crate::transport::relay_peer::RemoteNativeInteractionContext,
        )>,
        DaemonError,
    > {
        let session_id = session_id.to_string();
        let agent_id = agent_id.to_string();
        self.with_app_side_effect(move |app| {
            let target = RemoteLeaseRuntime::new(app).native_interaction_context_for_backing_agent(
                &session_id,
                &agent_id,
                "unknown",
            );
            Ok::<_, DaemonError>(
                target.map(|(daemon_id, context)| (app.config().clone(), daemon_id, context)),
            )
        })
        .await
    }

    pub(crate) async fn watch_relay_subscription_state(
        &self,
        session_id: &str,
        attachment_id: &str,
        should_check_snapshot: bool,
        previous_snapshot: Option<SessionSnapshotProjection>,
        last_workflow_design_sequence: u64,
    ) -> WatchResult {
        let session_id = session_id.to_string();
        let attachment_id = attachment_id.to_string();
        let previous_snapshot_for_compare = previous_snapshot.clone();
        let mut result = self
            .with_app_side_effect({
                let session_id = session_id.clone();
                let attachment_id = attachment_id.clone();
                move |app| {
                    crate::runtime_transport::watch_subscription_state(
                        app,
                        &session_id,
                        &attachment_id,
                        false,
                        None,
                        last_workflow_design_sequence,
                    )
                }
            })
            .await;
        if !should_check_snapshot {
            return result;
        }
        let projected_snapshot = match self.read_only_session_snapshot_projection_for_attachment(
            &session_id,
            &attachment_id,
            0,
        ) {
            Ok(snapshot) => Box::new(
                (previous_snapshot_for_compare.as_ref() != Some(&snapshot)).then_some(snapshot),
            ),
            Err(DaemonError::SessionNotFound { .. })
            | Err(DaemonError::AttachmentNotFound { .. })
            | Err(DaemonError::AttachmentNotInSession { .. }) => {
                return WatchResult::Unavailable(
                    "Current session is no longer available.".to_string(),
                );
            }
            Err(error) => {
                crate::logging::warn_with_fields(
                    "daemon.runtime_transport",
                    "kernel event loop failed to build owned session snapshot",
                    serde_json::json!({
                        "session_id": session_id,
                        "attachment_id": attachment_id,
                        "error": error.to_string(),
                    }),
                );
                Box::new(None)
            }
        };
        if let WatchResult::Ok { snapshot, .. } = &mut result {
            *snapshot = projected_snapshot;
        }
        result
    }

    pub(crate) async fn create_relay_execution_lease(
        &self,
        home_kernel_id: &str,
        home_session_id: &str,
        home_agent_id: &str,
        home_agent_metaagent: bool,
        owner_user_id: &str,
    ) -> Result<ExecutionLease, DaemonError> {
        let home_kernel_id = home_kernel_id.to_string();
        let home_session_id = home_session_id.to_string();
        let home_agent_id = home_agent_id.to_string();
        let owner_user_id = owner_user_id.to_string();
        self.with_app_side_effect(move |app| {
            RemoteLeaseRuntime::new(app).create_execution_lease(
                &home_kernel_id,
                &home_session_id,
                &home_agent_id,
                home_agent_metaagent,
                &owner_user_id,
            )
        })
        .await
    }

    pub(crate) async fn destroy_relay_execution_lease(
        &self,
        lease_id: &str,
    ) -> Result<ExecutionLease, DaemonError> {
        let lease_id = lease_id.to_string();
        self.with_app_side_effect(move |app| {
            RemoteLeaseRuntime::new(app).destroy_execution_lease(&lease_id)
        })
        .await
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn create_relay_leased_agent(
        &self,
        lease_id: &str,
        provider: &str,
        model: Option<String>,
        effort: Option<String>,
        execution_mode: Option<AgentExecutionMode>,
        permission_level: Option<AgentPermissionLevel>,
        workspace_live_sync_mode: Option<crate::config::WorkspaceLiveSyncMode>,
        worktree_id: Option<String>,
        worktree_placement: Option<crate::agent::GitWorktreePlacement>,
    ) -> Result<LeasedAgent, DaemonError> {
        let lease_id = lease_id.to_string();
        let provider = provider.to_string();
        self.with_app_side_effect(move |app| {
            RemoteLeaseRuntime::new(app).create_leased_agent(
                &lease_id,
                &provider,
                model,
                effort,
                execution_mode,
                permission_level,
                workspace_live_sync_mode,
                worktree_id,
                worktree_placement,
            )
        })
        .await
    }

    pub(crate) async fn destroy_relay_leased_agent(
        &self,
        leased_agent_id: &str,
    ) -> Result<LeasedAgent, DaemonError> {
        let leased_agent_id = leased_agent_id.to_string();
        self.with_app_side_effect(move |app| {
            RemoteLeaseRuntime::new(app).destroy_leased_agent(&leased_agent_id)
        })
        .await
    }

    pub(crate) async fn update_relay_leased_agent_config(
        &self,
        leased_agent_id: &str,
        execution_mode: AgentExecutionMode,
        permission_level: AgentPermissionLevel,
    ) -> Result<LeasedAgent, DaemonError> {
        let leased_agent_id = leased_agent_id.to_string();
        self.with_app_side_effect(move |app| {
            RemoteLeaseRuntime::new(app).update_leased_agent_config(
                &leased_agent_id,
                execution_mode,
                permission_level,
            )
        })
        .await
    }

    pub(crate) async fn update_relay_leased_agent_remote_extension_manifest(
        &self,
        leased_agent_id: &str,
        remote_extension_manifest: crate::extension::RemoteExtensionManifest,
    ) -> Result<(), DaemonError> {
        let leased_agent_id = leased_agent_id.to_string();
        self.with_app_side_effect(move |app| {
            RemoteLeaseRuntime::new(app).update_leased_agent_remote_extension_manifest(
                &leased_agent_id,
                remote_extension_manifest,
            )
        })
        .await
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn launch_relay_leased_native_provider_run(
        &self,
        leased_agent_id: &str,
        adapter_key: &str,
        provider: &str,
        account_profile: &str,
        model: &str,
        variant: Option<String>,
        structured_endpoint: Option<String>,
        provider_session_id: Option<String>,
        required_mcps: Vec<RequiredRemoteMcp>,
        remote_extension_manifest: crate::extension::RemoteExtensionManifest,
    ) -> Result<crate::provider::RuntimeProviderRun, DaemonError> {
        let leased_agent_id = leased_agent_id.to_string();
        let adapter_key = adapter_key.to_string();
        let provider = provider.to_string();
        let account_profile = account_profile.to_string();
        let model = model.to_string();
        self.with_app_side_effect(move |app| {
            RemoteLeaseRuntime::new(app).launch_leased_native_provider_run(
                &leased_agent_id,
                &adapter_key,
                &provider,
                &account_profile,
                &model,
                variant,
                structured_endpoint,
                provider_session_id,
                required_mcps,
                remote_extension_manifest,
            )
        })
        .await
    }

    pub(crate) async fn send_relay_leased_native_provider_input(
        &self,
        leased_agent_id: &str,
        provider_run_id: &str,
        attachment_id: &str,
        data_base64: &str,
    ) -> Result<usize, DaemonError> {
        let leased_agent_id = leased_agent_id.to_string();
        let provider_run_id = provider_run_id.to_string();
        let attachment_id = attachment_id.to_string();
        let data_base64 = data_base64.to_string();
        self.with_app_side_effect(move |app| {
            RemoteLeaseRuntime::new(app).send_leased_native_provider_input(
                &leased_agent_id,
                &provider_run_id,
                &attachment_id,
                &data_base64,
            )
        })
        .await
    }

    pub(crate) async fn submit_relay_leased_prompt(
        &self,
        leased_agent_id: &str,
        prompt: &str,
        attachments: Vec<RelayPromptAttachment>,
        workflow_context: Option<RemoteWorkflowTurnContext>,
        git_context: Option<RemoteGitTurnContext>,
        required_mcps: Vec<RequiredRemoteMcp>,
        remote_extension_manifest: crate::extension::RemoteExtensionManifest,
    ) -> Result<(String, crate::session::PromptSubmissionOutcome), DaemonError> {
        let leased_agent_id = leased_agent_id.to_string();
        let prompt = prompt.to_string();
        let prepared = self
            .with_app_side_effect(move |app| {
                RemoteLeaseRuntime::new(app).prepare_leased_prompt_submission(
                    &leased_agent_id,
                    &prompt,
                    attachments,
                    workflow_context,
                    git_context,
                    required_mcps,
                    remote_extension_manifest,
                )
            })
            .await?;
        let provider_run_id = match &prepared.provider_run {
            crate::app::PreparedLeasedProviderRun::Ready(provider_run_id) => {
                provider_run_id.clone()
            }
            crate::app::PreparedLeasedProviderRun::LaunchRequired(request) => self
                .launch_provider_for_remote_lease_detached(request.clone())
                .await?
                .id()
                .to_string(),
        };
        self.with_app_side_effect(move |app| {
            RemoteLeaseRuntime::new(app)
                .finish_prepared_leased_prompt_submission(prepared, provider_run_id)
        })
        .await
    }

    pub(crate) async fn ensure_relay_remote_skill_packages(
        &self,
        context: RemoteSkillSyncContext,
        packages: Vec<ArrobaSkillPackage>,
    ) -> Result<Vec<RemoteSkillMaterialization>, DaemonError> {
        self.with_app_side_effect(move |app| {
            RemoteLeaseRuntime::new(app).ensure_remote_skill_packages(context, packages)
        })
        .await
    }

    pub(crate) async fn check_relay_remote_mcp_availability(
        &self,
        context: RemoteMcpCheckContext,
        required_mcps: Vec<RequiredRemoteMcp>,
    ) -> Result<Vec<RemoteMcpAvailability>, DaemonError> {
        self.with_app_side_effect(move |app| {
            RemoteLeaseRuntime::new(app).check_remote_mcp_availability(context, required_mcps)
        })
        .await
    }

    pub(crate) async fn complete_relay_leased_prompt(
        &self,
        leased_agent_id: &str,
    ) -> Result<crate::session::PromptCompletion, DaemonError> {
        let leased_agent_id = leased_agent_id.to_string();
        self.with_app_side_effect(move |app| {
            RemoteLeaseRuntime::new(app).complete_leased_prompt(&leased_agent_id)
        })
        .await
    }

    pub(crate) async fn observe_relay_leased_git_after(
        &self,
        leased_agent_id: &str,
        provider_run_id: &str,
    ) -> Result<
        (
            Vec<RemoteGitObservation>,
            Option<crate::git_observer::WorkspaceLiveSyncChange>,
        ),
        DaemonError,
    > {
        let leased_agent_id = leased_agent_id.to_string();
        let provider_run_id = provider_run_id.to_string();
        self.with_app_side_effect(move |app| {
            RemoteLeaseRuntime::new(app)
                .observe_leased_git_after(&leased_agent_id, &provider_run_id)
        })
        .await
    }

    pub(crate) async fn cancel_relay_leased_prompt(
        &self,
        leased_agent_id: &str,
    ) -> Result<crate::session::PromptCancellation, DaemonError> {
        self.cancel_remote_home_extension_invocations_for_leased_agent(leased_agent_id)
            .await;
        let leased_agent_id = leased_agent_id.to_string();
        self.with_app_side_effect(move |app| {
            RemoteLeaseRuntime::new(app).cancel_leased_prompt(&leased_agent_id)
        })
        .await
    }

    pub(crate) async fn relay_leased_agent_provider_run_id(
        &self,
        leased_agent_id: &str,
    ) -> Result<Option<String>, DaemonError> {
        let leased_agent_id = leased_agent_id.to_string();
        self.with_app_side_effect(move |app| {
            RemoteLeaseRuntime::new(app).leased_agent_provider_run_id(&leased_agent_id)
        })
        .await
    }

    pub(crate) async fn pump_relay_leased_runtime_projections(
        &self,
    ) -> Result<Vec<(String, RelayPeerEvent)>, DaemonError> {
        self.with_app_side_effect(|app| {
            RemoteLeaseRuntime::new(app).pump_leased_runtime_projections()
        })
        .await
    }

    pub(crate) async fn drain_relay_leased_runtime_projection(
        &self,
        leased_agent_id: &str,
        provider_run_id: &str,
        pump_output: bool,
    ) -> Result<Option<(String, RelayPeerEvent)>, DaemonError> {
        let leased_agent_id = leased_agent_id.to_string();
        let provider_run_id = provider_run_id.to_string();
        self.with_app_side_effect(move |app| {
            RemoteLeaseRuntime::new(app).drain_leased_runtime_projection(
                &leased_agent_id,
                &provider_run_id,
                pump_output,
            )
        })
        .await
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn project_relay_remote_runtime_projection(
        &self,
        session_id: &str,
        agent_id: &str,
        provider_run_id: &str,
        provider_run: Option<crate::provider::RuntimeProviderRun>,
        prompts: Vec<RelayProjectedPrompt>,
        output_chunks: Vec<RelayProjectedOutputChunk>,
        notices: Vec<String>,
        completions: Vec<RelayProjectedCompletion>,
    ) -> Result<(), DaemonError> {
        let session_id = session_id.to_string();
        let agent_id = agent_id.to_string();
        let provider_run_id = provider_run_id.to_string();
        let projection_session_id = session_id.clone();
        let projection_agent_id = agent_id.clone();
        let projection_provider_run_id = provider_run_id.clone();
        let outcome = self
            .with_app_side_effect(move |app| {
                RemoteLeaseRuntime::new(app).project_remote_runtime_projection(
                    &projection_session_id,
                    &projection_agent_id,
                    &projection_provider_run_id,
                    provider_run,
                    prompts,
                    output_chunks,
                    notices,
                    completions,
                )
            })
            .await?;
        for completion in outcome.completions {
            self.inject_metaagent_turn_completion_event(&session_id, &agent_id, &completion)?;
        }
        Ok(())
    }
}
