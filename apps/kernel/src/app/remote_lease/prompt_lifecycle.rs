use crate::error::DaemonError;
use crate::execution_lease::{LeasedWorkflowTurnBinding, RemoteWorkflowTurnContext};
use crate::transport::relay_peer::{
    RelayPromptAttachment, RemoteGitTurnContext, RequiredRemoteMcp,
};

use super::RemoteLeaseRuntime;

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
    ) -> Result<(String, crate::session::PromptSubmissionOutcome), DaemonError> {
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
        let provider_run_id =
            self.ensure_leased_provider_run_matches_mcps(&leased_agent, &required_mcps)?;
        if let Some(git_context) = git_context {
            if self
                .app
                .prompt_owner_active_prompt_for_agent(
                    &leased_agent.backing_session_id,
                    &leased_agent.backing_agent_id,
                )?
                .is_none()
            {
                self.observe_leased_git_before(&leased_agent, &provider_run_id, git_context);
            }
        }
        let outcome = self.app.submit_prompt(
            &leased_agent.backing_session_id,
            &leased_agent.backing_attachment_id,
            Some(&leased_agent.backing_agent_id),
            prompt,
            materialized_attachments,
        )?;
        if let Some(context) = workflow_context {
            self.app.leased_workflow_turns.insert(
                provider_run_id.clone(),
                LeasedWorkflowTurnBinding {
                    leased_agent_id: leased_agent_id.to_string(),
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
