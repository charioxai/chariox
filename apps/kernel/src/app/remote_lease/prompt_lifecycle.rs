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
    pub(crate) hidden_system_context: String,
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
            None,
            crate::extension::RemoteExtensionManifest::default(),
        )
    }

    #[cfg(test)]
    pub(crate) fn submit_leased_prompt_with_workflow_context(
        &mut self,
        leased_agent_id: &str,
        prompt: &str,
        attachments: Vec<RelayPromptAttachment>,
        workflow_context: Option<RemoteWorkflowTurnContext>,
        git_context: Option<RemoteGitTurnContext>,
        required_mcps: Vec<RequiredRemoteMcp>,
        required_skills: Option<Vec<crate::transport::relay_peer::RequiredRemoteSkill>>,
        remote_extension_manifest: crate::extension::RemoteExtensionManifest,
    ) -> Result<(String, crate::session::PromptSubmissionOutcome), DaemonError> {
        if let Some(replayed) =
            self.replay_active_leased_prompt_submission(leased_agent_id, git_context.as_ref())?
        {
            return Ok(replayed);
        }
        let prepared = self.prepare_leased_prompt_submission(
            leased_agent_id,
            prompt,
            attachments,
            workflow_context,
            git_context,
            required_mcps,
            required_skills,
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

    pub(crate) fn replay_active_leased_prompt_submission(
        &mut self,
        leased_agent_id: &str,
        git_context: Option<&RemoteGitTurnContext>,
    ) -> Result<Option<(String, PromptSubmissionOutcome)>, DaemonError> {
        let Some(home_prompt_id) = git_context.map(|context| context.home_prompt_id.as_str())
        else {
            return Ok(None);
        };
        let leased_agent = self
            .app
            .leased_agents
            .get(leased_agent_id)
            .cloned()
            .ok_or_else(|| DaemonError::LeasedAgentNotFound {
                leased_agent_id: leased_agent_id.to_string(),
            })?;
        if leased_agent.active_home_prompt_id.as_deref() != Some(home_prompt_id) {
            return Ok(None);
        }
        let active_prompt = self
            .app
            .prompt_owner_active_prompt_for_agent(
                &leased_agent.backing_session_id,
                &leased_agent.backing_agent_id,
            )?
            .ok_or_else(|| DaemonError::LocalTransport {
                operation: "replay active leased prompt submission",
                message: format!(
                    "leased agent `{leased_agent_id}` remembers home prompt `{home_prompt_id}` but has no active backing prompt"
                ),
            })?;
        let provider_run = self
            .app
            .providers
            .get_run_for_agent(
                &leased_agent.backing_session_id,
                &leased_agent.backing_agent_id,
            )
            .ok_or_else(|| DaemonError::NoActiveProviderRun {
                session_id: leased_agent.backing_session_id.clone(),
            })?;
        if provider_run.state() == crate::provider::ProviderRunState::Ended {
            return Err(DaemonError::NoActiveProviderRun {
                session_id: leased_agent.backing_session_id,
            });
        }
        Ok(Some((
            provider_run.id().to_string(),
            PromptSubmissionOutcome::Started {
                prompt: active_prompt,
            },
        )))
    }

    pub(crate) fn prepare_leased_prompt_submission(
        &mut self,
        leased_agent_id: &str,
        prompt: &str,
        attachments: Vec<RelayPromptAttachment>,
        workflow_context: Option<RemoteWorkflowTurnContext>,
        git_context: Option<RemoteGitTurnContext>,
        required_mcps: Vec<RequiredRemoteMcp>,
        required_skills: Option<Vec<crate::transport::relay_peer::RequiredRemoteSkill>>,
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
        let hidden_system_context = if let Some(required_skills) = required_skills.as_deref() {
            self.apply_required_remote_skills(&leased_agent, required_skills)?;
            self.required_remote_skill_prompt_context(&leased_agent, prompt)?
        } else {
            String::new()
        };
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
            hidden_system_context,
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
            hidden_system_context,
            materialized_attachments,
            workflow_context,
            git_context,
            provider_run: _,
        } = prepared;
        let home_prompt_id = git_context
            .as_ref()
            .map(|context| context.home_prompt_id.clone());
        if let Some(git_context) = git_context {
            self.observe_leased_git_before(&leased_agent, &provider_run_id, git_context);
        }
        let outcome = crate::app::KernelAgentService::new(self.app)
            .submit_prompt_with_hidden_system_context(
                &leased_agent.backing_session_id,
                &leased_agent.backing_attachment_id,
                Some(&leased_agent.backing_agent_id),
                &prompt,
                &hidden_system_context,
                materialized_attachments,
            )?;
        let started = matches!(outcome, PromptSubmissionOutcome::Started { .. });
        let accepted_prompt = match &outcome {
            PromptSubmissionOutcome::Started { prompt }
            | PromptSubmissionOutcome::Queued { prompt } => prompt,
        };
        let backing_active = self.app.prompt_owner_active_prompt_for_agent(
            &leased_agent.backing_session_id,
            &leased_agent.backing_agent_id,
        )?;
        let current_submission_was_promoted = backing_active.as_ref().is_some_and(|active| {
            active.created_at_ms() == accepted_prompt.created_at_ms()
                && active.prompt() == accepted_prompt.prompt()
                && active.hidden_system_context() == accepted_prompt.hidden_system_context()
                && active.attachments() == accepted_prompt.attachments()
        });
        if started {
            crate::transport::flow_control::note_prompt_started(self.app, &provider_run_id);
        }
        let provider_run_projection = self
            .app
            .providers
            .get_run(&provider_run_id)
            .ok()
            .map(|run| (run.id().to_string(), run.state()));
        if started
            || leased_agent.active_home_prompt_id.is_none()
            || backing_active.is_none()
            || current_submission_was_promoted
        {
            if let Some(agent) = self.app.leased_agents.get_mut(&leased_agent.id) {
                if agent.active_home_prompt_id.as_deref() != home_prompt_id.as_deref() {
                    agent.applied_home_steer_ids.clear();
                }
                agent.active_home_prompt_id = home_prompt_id;
                agent.active_home_prompt_started_at_ms =
                    backing_active.as_ref().map(|prompt| prompt.created_at_ms());
            }
        }
        if let Some(provider_run_projection) = provider_run_projection {
            if let Some(agent) = self.app.leased_agents.get_mut(&leased_agent.id) {
                agent.projected_provider_run = Some(provider_run_projection);
            }
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

    pub(crate) fn prepare_leased_prompt_steer(
        &mut self,
        leased_agent_id: &str,
        steer_id: &str,
        target_home_prompt_id: &str,
        prompt: &str,
        hidden_system_context: &str,
        attachments: Vec<RelayPromptAttachment>,
        required_skills: Option<Vec<crate::transport::relay_peer::RequiredRemoteSkill>>,
    ) -> Result<(String, Option<crate::app::KernelPromptDispatch>), DaemonError> {
        let leased_agent = self
            .app
            .leased_agents
            .get(leased_agent_id)
            .cloned()
            .ok_or_else(|| DaemonError::LeasedAgentNotFound {
                leased_agent_id: leased_agent_id.to_string(),
            })?;
        if leased_agent.active_home_prompt_id.as_deref() != Some(target_home_prompt_id) {
            return Err(DaemonError::LocalTransport {
                operation: "steer leased prompt",
                message: format!(
                    "leased agent `{leased_agent_id}` is running home prompt {:?}, not `{target_home_prompt_id}`",
                    leased_agent.active_home_prompt_id
                ),
            });
        }
        let provider_run = self
            .app
            .providers
            .get_run_for_agent(
                &leased_agent.backing_session_id,
                &leased_agent.backing_agent_id,
            )
            .ok_or_else(|| DaemonError::NoActiveProviderRun {
                session_id: leased_agent.backing_session_id.clone(),
            })?;
        if provider_run.state() != crate::provider::ProviderRunState::Running {
            return Err(DaemonError::InvalidProviderRunState {
                provider_run_id: provider_run.id().to_string(),
                state: provider_run.state(),
                operation: "steer leased prompt",
            });
        }
        if leased_agent
            .applied_home_steer_ids
            .iter()
            .any(|applied| applied == steer_id)
        {
            return Ok((provider_run.id().to_string(), None));
        }
        let active_prompt = self
            .app
            .prompt_owner_active_prompt_for_agent(
                &leased_agent.backing_session_id,
                &leased_agent.backing_agent_id,
            )?
            .ok_or_else(|| DaemonError::NoActivePrompt {
                session_id: leased_agent.backing_session_id.clone(),
            })?;
        if let Some(required_skills) = required_skills.as_deref() {
            self.apply_required_remote_skills(&leased_agent, required_skills)?;
        }
        let materialized_attachments =
            self.materialize_leased_prompt_attachments(&leased_agent, attachments)?;
        Ok((
            provider_run.id().to_string(),
            Some(crate::app::KernelPromptDispatch {
                session_id: leased_agent.backing_session_id,
                provider_run_id: provider_run.id().to_string(),
                agent_id: leased_agent.backing_agent_id,
                prompt_id: format!("leased-steer:{steer_id}"),
                target_active_prompt_id: Some(active_prompt.id().to_string()),
                source_attachment_id: leased_agent.backing_attachment_id,
                prompt: prompt.to_string(),
                hidden_system_context: hidden_system_context.to_string(),
                attachments: materialized_attachments,
                prompt_origin: crate::session::PromptOrigin::Arroba,
                external_provider: None,
                external_provider_session_id: None,
                external_provider_turn_id: None,
                steering: true,
            }),
        ))
    }

    pub(crate) fn reserve_leased_prompt_steer(
        &mut self,
        leased_agent_id: &str,
        steer_id: &str,
        target_home_prompt_id: &str,
    ) -> Result<bool, DaemonError> {
        let leased_agent = self
            .app
            .leased_agents
            .get_mut(leased_agent_id)
            .ok_or_else(|| DaemonError::LeasedAgentNotFound {
                leased_agent_id: leased_agent_id.to_string(),
            })?;
        if leased_agent.active_home_prompt_id.as_deref() != Some(target_home_prompt_id) {
            return Err(DaemonError::LocalTransport {
                operation: "steer leased prompt",
                message: format!(
                    "leased agent `{leased_agent_id}` is running home prompt {:?}, not `{target_home_prompt_id}`",
                    leased_agent.active_home_prompt_id
                ),
            });
        }
        if leased_agent
            .applied_home_steer_ids
            .iter()
            .any(|applied| applied == steer_id)
        {
            return Ok(false);
        }
        leased_agent
            .applied_home_steer_ids
            .push(steer_id.to_string());
        Ok(true)
    }

    pub(crate) fn rollback_leased_prompt_steer(&mut self, leased_agent_id: &str, steer_id: &str) {
        if let Some(leased_agent) = self.app.leased_agents.get_mut(leased_agent_id) {
            leased_agent
                .applied_home_steer_ids
                .retain(|applied| applied != steer_id);
        }
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
