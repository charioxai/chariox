use crate::error::DaemonError;
use crate::execution_lease::{LeasedAgent, LeasedWorkflowTurnBinding, RemoteWorkflowTurnContext};
use crate::provider::LaunchProviderRequest;
use crate::session::{PromptAttachment, PromptSubmissionOutcome};
use crate::transport::relay_peer::{
    RelayPromptAttachment, RemoteGitTurnContext, RequiredRemoteMcp,
};

use super::provider_run::LeasedProviderRunMatch;
use super::RemoteLeaseRuntime;

pub(crate) enum PreparedLeasedProviderRun {
    Ready(String),
    LaunchRequired(LaunchProviderRequest),
}

pub(crate) struct PreparedLeasedPromptSubmission {
    pub(crate) leased_agent: LeasedAgent,
    pub(crate) prompt: String,
    pub(crate) materialized_attachments: Vec<PromptAttachment>,
    pub(crate) workflow_context: Option<RemoteWorkflowTurnContext>,
    pub(crate) git_context: Option<RemoteGitTurnContext>,
    pub(crate) provider_run: PreparedLeasedProviderRun,
}

impl<'a> RemoteLeaseRuntime<'a> {
    #[cfg(test)]
    pub(crate) fn submit_leased_prompt(
        &mut self,
        leased_agent_id: &str,
        prompt: &str,
        attachments: Vec<RelayPromptAttachment>,
    ) -> Result<(String, crate::session::PromptSubmissionOutcome), DaemonError> {
        self.submit_leased_prompt_with_workflow_context(
            leased_agent_id,
            prompt,
            attachments,
            None,
            None,
            Vec::new(),
            crate::extension::RemoteExtensionManifest::default(),
        )
    }

    pub(crate) fn submit_leased_prompt_with_workflow_context(
        &mut self,
        leased_agent_id: &str,
        prompt: &str,
        attachments: Vec<RelayPromptAttachment>,
        workflow_context: Option<RemoteWorkflowTurnContext>,
        git_context: Option<RemoteGitTurnContext>,
        required_mcps: Vec<RequiredRemoteMcp>,
        remote_extension_manifest: crate::extension::RemoteExtensionManifest,
    ) -> Result<(String, crate::session::PromptSubmissionOutcome), DaemonError> {
        let prepared = self.prepare_leased_prompt_submission(
            leased_agent_id,
            prompt,
            attachments,
            workflow_context,
            git_context,
            required_mcps,
            remote_extension_manifest,
        )?;
        let provider_run_id = match &prepared.provider_run {
            PreparedLeasedProviderRun::Ready(provider_run_id) => provider_run_id.clone(),
            PreparedLeasedProviderRun::LaunchRequired(request) => {
                self.app.launch_provider(request.clone())?.id().to_string()
            }
        };
        self.finish_prepared_leased_prompt_submission(prepared, provider_run_id)
    }

    pub(crate) fn prepare_leased_prompt_submission(
        &mut self,
        leased_agent_id: &str,
        prompt: &str,
        attachments: Vec<RelayPromptAttachment>,
        workflow_context: Option<RemoteWorkflowTurnContext>,
        git_context: Option<RemoteGitTurnContext>,
        required_mcps: Vec<RequiredRemoteMcp>,
        remote_extension_manifest: crate::extension::RemoteExtensionManifest,
    ) -> Result<PreparedLeasedPromptSubmission, DaemonError> {
        let leased_agent = self
            .app
            .leased_agents
            .get(leased_agent_id)
            .cloned()
            .ok_or_else(|| DaemonError::LeasedAgentNotFound {
                leased_agent_id: leased_agent_id.to_string(),
            })?;
        let materialized_attachments =
            self.materialize_leased_prompt_attachments(&leased_agent, attachments)?;
        self.ensure_required_remote_mcps_available(&leased_agent, &required_mcps)?;
        if let Some(mode) = git_context
            .as_ref()
            .and_then(|context| context.workspace_live_sync_mode)
        {
            let backing_session = self
                .app
                .sessions
                .get_session(&leased_agent.backing_session_id)?;
            if backing_session.workspace_live_sync_mode() != Some(mode) {
                self.app
                    .sessions
                    .write()
                    .set_workspace_live_sync_mode(&leased_agent.backing_session_id, mode)?;
            }
        }
        let provider_run = match self.prepare_leased_provider_run_matches_mcps(
            &leased_agent,
            &required_mcps,
            &remote_extension_manifest,
        )? {
            LeasedProviderRunMatch::Ready(provider_run_id) => {
                PreparedLeasedProviderRun::Ready(provider_run_id)
            }
            LeasedProviderRunMatch::LaunchRequired(request) => {
                PreparedLeasedProviderRun::LaunchRequired(request)
            }
        };
        Ok(PreparedLeasedPromptSubmission {
            leased_agent,
            prompt: prompt.to_string(),
            materialized_attachments,
            workflow_context,
            git_context,
            provider_run,
        })
    }

    pub(crate) fn finish_prepared_leased_prompt_submission(
        &mut self,
        prepared: PreparedLeasedPromptSubmission,
        provider_run_id: String,
    ) -> Result<(String, PromptSubmissionOutcome), DaemonError> {
        let PreparedLeasedPromptSubmission {
            leased_agent,
            prompt,
            materialized_attachments,
            workflow_context,
            git_context,
            provider_run: _,
        } = prepared;
        if let Some(git_context) = git_context {
            self.observe_leased_git_before(&leased_agent, &provider_run_id, git_context);
        }
        let outcome = self.app.submit_prompt(
            &leased_agent.backing_session_id,
            &leased_agent.backing_attachment_id,
            Some(&leased_agent.backing_agent_id),
            &prompt,
            materialized_attachments,
        )?;
        if matches!(outcome, PromptSubmissionOutcome::Started { .. }) {
            crate::transport::flow_control::note_prompt_started(self.app, &provider_run_id);
        }
        if let Some(context) = workflow_context {
            self.app.leased_workflow_turns.insert(
                provider_run_id.clone(),
                LeasedWorkflowTurnBinding {
                    leased_agent_id: leased_agent.id.clone(),
                    provider_run_id: provider_run_id.clone(),
                    context,
                },
            );
        }
        Ok((provider_run_id, outcome))
    }

    pub(crate) fn complete_leased_prompt(
        &mut self,
        leased_agent_id: &str,
    ) -> Result<crate::session::PromptCompletion, DaemonError> {
        let leased_agent = self
            .app
            .leased_agents
            .get(leased_agent_id)
            .cloned()
            .ok_or_else(|| DaemonError::LeasedAgentNotFound {
                leased_agent_id: leased_agent_id.to_string(),
            })?;
        let provider_run_id = self
            .app
            .providers
            .get_run_for_agent(
                &leased_agent.backing_session_id,
                &leased_agent.backing_agent_id,
            )
            .map(|run| run.id().to_string());
        let completion = self.app.complete_active_prompt(
            &leased_agent.backing_session_id,
            &leased_agent.backing_agent_id,
            provider_run_id.as_deref(),
        )?;
        if let Some(provider_run_id) = provider_run_id {
            self.app.leased_workflow_turns.remove(&provider_run_id);
        }
        Ok(completion)
    }

    pub(crate) fn cancel_leased_prompt(
        &mut self,
        leased_agent_id: &str,
    ) -> Result<crate::session::PromptCancellation, DaemonError> {
        let leased_agent = self
            .app
            .leased_agents
            .get(leased_agent_id)
            .cloned()
            .ok_or_else(|| DaemonError::LeasedAgentNotFound {
                leased_agent_id: leased_agent_id.to_string(),
            })?;
        let cancellation = self.app.cancel_active_prompt_internal(
            &leased_agent.backing_session_id,
            &leased_agent.backing_agent_id,
            None,
        )?;
        self.app
            .leased_workflow_turns
            .retain(|_, binding| binding.leased_agent_id != leased_agent_id);
        Ok(cancellation)
    }

    pub(crate) fn complete_leased_workflow_prompt_for_provider_run(
        &mut self,
        provider_run_id: &str,
    ) -> Result<Option<crate::session::PromptCompletion>, DaemonError> {
        let Some(binding) = self.app.leased_workflow_turns.get(provider_run_id).cloned() else {
            return Ok(None);
        };
        let leased_agent = self
            .app
            .leased_agents
            .get(&binding.leased_agent_id)
            .cloned()
            .ok_or_else(|| DaemonError::LeasedAgentNotFound {
                leased_agent_id: binding.leased_agent_id.clone(),
            })?;
        if self
            .app
            .prompt_owner_active_prompt_for_agent(
                &leased_agent.backing_session_id,
                &leased_agent.backing_agent_id,
            )?
            .is_none()
        {
            self.app.leased_workflow_turns.remove(provider_run_id);
            return Ok(None);
        }
        let completion = self.app.complete_active_prompt(
            &leased_agent.backing_session_id,
            &leased_agent.backing_agent_id,
            Some(provider_run_id),
        )?;
        self.app.leased_workflow_turns.remove(provider_run_id);
        Ok(Some(completion))
    }
}
