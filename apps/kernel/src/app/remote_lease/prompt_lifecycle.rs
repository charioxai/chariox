use crate::error::DaemonError;
use crate::execution_lease::{LeasedAgent, LeasedWorkflowTurnBinding, RemoteWorkflowTurnContext};
use crate::execution_lease::{LeasedPromptSteerReceipt, LeasedPromptSteerReceiptPhase};
use crate::provider::LaunchProviderRequest;
use crate::session::{PromptAttachment, PromptSubmissionOutcome};
use crate::transport::relay_peer::{
    LeasedPromptReceipt, LeasedPromptReceiptPhase, RelayPromptAttachment, RemoteGitTurnContext,
    RequiredRemoteMcp,
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

fn leased_prompt_steer_receipt_projection(
    receipt: &LeasedPromptSteerReceipt,
) -> LeasedPromptReceipt {
    let phase = match receipt.phase {
        LeasedPromptSteerReceiptPhase::Dispatching => LeasedPromptReceiptPhase::SteerDispatching,
        LeasedPromptSteerReceiptPhase::Accepted => LeasedPromptReceiptPhase::SteerAccepted,
        LeasedPromptSteerReceiptPhase::Rejected => LeasedPromptReceiptPhase::SteerRejected,
    };
    LeasedPromptReceipt {
        home_prompt_id: receipt.steer_id.clone(),
        worker_provider_run_id: receipt.worker_provider_run_id.clone(),
        phase,
        target_home_prompt_id: Some(receipt.target_home_prompt_id.clone()),
        execution_lease_id: Some(receipt.execution_lease_id.clone()),
    }
}

impl<'a> RemoteLeaseRuntime<'a> {
    pub(crate) fn leased_prompt_receipt_provider_run_id(
        &self,
        leased_agent_id: &str,
        home_prompt_id: &str,
    ) -> Result<Option<String>, DaemonError> {
        let leased_agent = self
            .app
            .leased_agents
            .get(leased_agent_id)
            .ok_or_else(|| DaemonError::LeasedAgentNotFound {
                leased_agent_id: leased_agent_id.to_string(),
            })?;
        if let Some(receipt) = leased_agent
            .home_steer_receipts
            .iter()
            .find(|receipt| receipt.steer_id == home_prompt_id)
        {
            return Ok(Some(receipt.worker_provider_run_id.clone()));
        }
        Ok(self
            .app
            .providers
            .get_run_for_agent(
                &leased_agent.backing_session_id,
                &leased_agent.backing_agent_id,
            )
            .map(|run| run.id().to_string()))
    }

    pub(crate) fn leased_prompt_receipt(
        &mut self,
        leased_agent_id: &str,
        home_prompt_id: &str,
    ) -> Result<Option<LeasedPromptReceipt>, DaemonError> {
        if home_prompt_id.is_empty() {
            return Ok(None);
        }
        let leased_agent = self
            .app
            .leased_agents
            .get(leased_agent_id)
            .cloned()
            .ok_or_else(|| DaemonError::LeasedAgentNotFound {
                leased_agent_id: leased_agent_id.to_string(),
            })?;

        if let Some(receipt) = leased_agent
            .home_steer_receipts
            .iter()
            .find(|receipt| receipt.steer_id == home_prompt_id)
        {
            if receipt.execution_lease_id != leased_agent.lease_id
                || receipt.worker_provider_run_id.trim().is_empty()
            {
                return Ok(None);
            }
            return Ok(Some(leased_prompt_steer_receipt_projection(receipt)));
        }

        let active_match = leased_agent.active_home_prompt_id.as_deref() == Some(home_prompt_id);
        let completed_receipt = leased_agent
            .replayable_completion
            .as_ref()
            .filter(|receipt| receipt.home_prompt_id.as_deref() == Some(home_prompt_id));
        if active_match && completed_receipt.is_some() {
            return Ok(None);
        }

        if let Some(receipt) = completed_receipt {
            let Some(provider_run) = self.app.providers.get_run(&receipt.provider_run_id).ok()
            else {
                return Ok(None);
            };
            if receipt.provider_run_id.is_empty()
                || provider_run.session_id() != leased_agent.backing_session_id.as_str()
                || provider_run.agent_instance_id()
                    != Some(leased_agent.backing_agent_id.as_str())
            {
                return Ok(None);
            }
            return Ok(Some(LeasedPromptReceipt {
                home_prompt_id: home_prompt_id.to_string(),
                worker_provider_run_id: receipt.provider_run_id.clone(),
                phase: LeasedPromptReceiptPhase::Completed,
                target_home_prompt_id: None,
                execution_lease_id: None,
            }));
        }

        if !active_match {
            return Ok(None);
        }
        let Some(active_prompt_started_at_ms) = leased_agent.active_home_prompt_started_at_ms else {
            return Ok(None);
        };
        let active_prompt = self.app.prompt_owner_active_prompt_for_agent(
            &leased_agent.backing_session_id,
            &leased_agent.backing_agent_id,
        )?;
        let Some(active_prompt) = active_prompt else {
            return Ok(None);
        };
        if active_prompt.created_at_ms() != active_prompt_started_at_ms
            || !matches!(
                active_prompt.status(),
                crate::session::PromptStatus::Dispatching | crate::session::PromptStatus::Running
            )
        {
            return Ok(None);
        }
        let Some(provider_run) = self
            .app
            .providers
            .get_run_for_agent(&leased_agent.backing_session_id, &leased_agent.backing_agent_id)
        else {
            return Ok(None);
        };
        if provider_run.agent_instance_id() != Some(leased_agent.backing_agent_id.as_str())
            || !matches!(
                provider_run.state(),
                crate::provider::ProviderRunState::Starting
                    | crate::provider::ProviderRunState::Running
            )
        {
            return Ok(None);
        }
        Ok(Some(LeasedPromptReceipt {
            home_prompt_id: home_prompt_id.to_string(),
            worker_provider_run_id: provider_run.id().to_string(),
            phase: LeasedPromptReceiptPhase::Active,
            target_home_prompt_id: None,
            execution_lease_id: None,
        }))
    }

    pub(crate) fn reconcile_leased_prompt_steer_receipt(
        &mut self,
        leased_agent_id: &str,
        steer_id: &str,
        target_home_prompt_id: &str,
        worker_provider_run_id: &str,
        execution_lease_id: &str,
    ) -> Result<LeasedPromptReceipt, DaemonError> {
        if steer_id.is_empty()
            || target_home_prompt_id.is_empty()
            || worker_provider_run_id.is_empty()
            || execution_lease_id.is_empty()
        {
            return Err(DaemonError::LocalTransport {
                operation: "reconcile leased prompt steer receipt",
                message: "steer, target prompt, worker run, and execution lease identities are required".to_string(),
            });
        }
        let leased_agent = self
            .app
            .leased_agents
            .get(leased_agent_id)
            .cloned()
            .ok_or_else(|| DaemonError::LeasedAgentNotFound {
                leased_agent_id: leased_agent_id.to_string(),
            })?;
        if leased_agent.lease_id != execution_lease_id {
            return Err(DaemonError::LocalTransport {
                operation: "reconcile leased prompt steer receipt",
                message: "requested execution lease does not match the leased worker".to_string(),
            });
        }
        if let Some(receipt) = leased_agent
            .home_steer_receipts
            .iter()
            .find(|receipt| receipt.steer_id == steer_id)
        {
            if receipt.target_home_prompt_id != target_home_prompt_id
                || receipt.worker_provider_run_id != worker_provider_run_id
                || receipt.execution_lease_id != execution_lease_id
            {
                return Err(DaemonError::LocalTransport {
                    operation: "reconcile leased prompt steer receipt",
                    message: "existing worker receipt conflicts with the requested steer identity".to_string(),
                });
            }
            return Ok(leased_prompt_steer_receipt_projection(receipt));
        }
        if leased_agent
            .applied_home_steer_ids
            .iter()
            .any(|applied| applied == steer_id)
        {
            return Err(DaemonError::LocalTransport {
                operation: "reconcile leased prompt steer receipt",
                message: "legacy applied steer has no exact durable receipt; refusing to infer its outcome".to_string(),
            });
        }

        // This operation runs under the worker provider-run lane. Once this
        // tombstone is committed, a delayed original relay request with this
        // identity can only observe Rejected and cannot reach provider input.
        let receipt = LeasedPromptSteerReceipt {
            steer_id: steer_id.to_string(),
            target_home_prompt_id: target_home_prompt_id.to_string(),
            worker_provider_run_id: worker_provider_run_id.to_string(),
            execution_lease_id: execution_lease_id.to_string(),
            phase: LeasedPromptSteerReceiptPhase::Rejected,
        };
        self.app
            .leased_agents
            .get_mut(leased_agent_id)
            .expect("leased agent was checked above")
            .home_steer_receipts
            .push(receipt.clone());
        Ok(leased_prompt_steer_receipt_projection(&receipt))
    }

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
            "",
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
                let run = self.app.launch_leased_provider(request.clone())?;
                run.id().to_string()
            }
        };
        self.finish_prepared_leased_prompt_submission(prepared, provider_run_id)
    }

    #[cfg(test)]
    pub(crate) fn submit_leased_prompt_with_hidden_context(
        &mut self,
        leased_agent_id: &str,
        prompt: &str,
        hidden_system_context: &str,
    ) -> Result<(String, crate::session::PromptSubmissionOutcome), DaemonError> {
        let prepared = self.prepare_leased_prompt_submission(
            leased_agent_id,
            prompt,
            hidden_system_context,
            Vec::new(),
            None,
            None,
            Vec::new(),
            None,
            crate::extension::RemoteExtensionManifest::default(),
        )?;
        let provider_run_id = match &prepared.provider_run {
            PreparedLeasedProviderRun::Ready(provider_run_id) => provider_run_id.clone(),
            PreparedLeasedProviderRun::LaunchRequired(request) => self
                .app
                .launch_leased_provider(request.clone())?
                .id()
                .to_string(),
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
        hidden_system_context: &str,
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
        let required_skill_context = if let Some(required_skills) = required_skills.as_deref() {
            self.apply_required_remote_skills(&leased_agent, required_skills)?;
            self.required_remote_skill_prompt_context(&leased_agent, prompt)?
        } else {
            String::new()
        };
        let hidden_system_context =
            join_hidden_context(hidden_system_context, &required_skill_context);
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
        let (event_reply_enabled, event_context_enabled, event_actions_enabled) = workflow_context
            .as_ref()
            .map(|context| {
                (
                    context.event_reply_enabled,
                    context.event_context_enabled,
                    context.event_actions_enabled,
                )
            })
            .unwrap_or((false, false, false));
        let provider_run = match self.prepare_leased_provider_run_matches_mcps(
            &leased_agent,
            &required_mcps,
            &remote_extension_manifest,
            event_reply_enabled,
            event_context_enabled,
            event_actions_enabled,
        )? {
            LeasedProviderRunMatch::Ready(provider_run_id) => {
                self.app.mark_leased_provider_run(&provider_run_id);
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
                    agent.replayable_completion = None;
                }
                agent.active_home_prompt_id = home_prompt_id.clone();
                agent.active_home_prompt_started_at_ms = Some(accepted_prompt.created_at_ms());
            }
        }
        if let Some(provider_run_projection) = provider_run_projection {
            if let Some(agent) = self.app.leased_agents.get_mut(&leased_agent.id) {
                agent.projected_provider_run = Some(provider_run_projection);
            }
        }
        if let Some(context) = workflow_context {
            let binding_home_prompt_id = home_prompt_id
                .clone()
                .unwrap_or_else(|| accepted_prompt.id().to_string());
            // The home kernel may allocate the same prompt id independently
            // for multiple leases.  The worker prompt id is allocated by this
            // kernel and is therefore the identity of this binding.  Keeping
            // the home id in the value preserves the context without allowing
            // one lease to overwrite another lease's queued turn.
            let binding_key = accepted_prompt.id().to_string();
            self.app.leased_workflow_turns.insert(
                binding_key,
                LeasedWorkflowTurnBinding {
                    leased_agent_id: leased_agent.id.clone(),
                    provider_run_id: provider_run_id.clone(),
                    home_prompt_id: binding_home_prompt_id,
                    backing_prompt_id: accepted_prompt.id().to_string(),
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
        if let Some(receipt) = leased_agent
            .home_steer_receipts
            .iter()
            .find(|receipt| receipt.steer_id == steer_id)
        {
            if receipt.target_home_prompt_id != target_home_prompt_id
                || receipt.worker_provider_run_id != provider_run.id()
                || receipt.execution_lease_id != leased_agent.lease_id
            {
                return Err(DaemonError::LocalTransport {
                    operation: "steer leased prompt",
                    message: "steer ID already has a receipt for a different worker prompt, run, or lease".to_string(),
                });
            }
            return match receipt.phase {
                LeasedPromptSteerReceiptPhase::Accepted => {
                    Ok((provider_run.id().to_string(), None))
                }
                LeasedPromptSteerReceiptPhase::Dispatching => Err(DaemonError::LocalTransport {
                    operation: "steer leased prompt",
                    message: format!(
                        "steer `{steer_id}` has an unresolved dispatch receipt; it will not be replayed"
                    ),
                }),
                LeasedPromptSteerReceiptPhase::Rejected => Err(DaemonError::LocalTransport {
                    operation: "steer leased prompt",
                    message: format!("steer `{steer_id}` was already rejected by this worker"),
                }),
            };
        }
        if leased_agent
            .applied_home_steer_ids
            .iter()
            .any(|applied| applied == steer_id)
        {
            return Err(DaemonError::LocalTransport {
                operation: "steer leased prompt",
                message: format!(
                    "steer `{steer_id}` is marked applied without an exact durable receipt; it will not be replayed"
                ),
            });
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
                prompt_origin: crate::session::PromptOrigin::Chariox,
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
        worker_provider_run_id: &str,
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
        if let Some(receipt) = leased_agent
            .home_steer_receipts
            .iter()
            .find(|receipt| receipt.steer_id == steer_id)
        {
            if receipt.target_home_prompt_id != target_home_prompt_id
                || receipt.worker_provider_run_id != worker_provider_run_id
                || receipt.execution_lease_id != leased_agent.lease_id
            {
                return Err(DaemonError::LocalTransport {
                    operation: "steer leased prompt",
                    message: "steer ID already has a receipt for a different worker prompt, run, or lease".to_string(),
                });
            }
            return Ok(false);
        }
        if leased_agent
            .applied_home_steer_ids
            .iter()
            .any(|applied| applied == steer_id)
        {
            return Err(DaemonError::LocalTransport {
                operation: "steer leased prompt",
                message: format!(
                    "steer `{steer_id}` is marked applied without an exact durable receipt"
                ),
            });
        }
        leased_agent
            .applied_home_steer_ids
            .push(steer_id.to_string());
        leased_agent.home_steer_receipts.push(LeasedPromptSteerReceipt {
            steer_id: steer_id.to_string(),
            target_home_prompt_id: target_home_prompt_id.to_string(),
            worker_provider_run_id: worker_provider_run_id.to_string(),
            execution_lease_id: leased_agent.lease_id.clone(),
            phase: LeasedPromptSteerReceiptPhase::Dispatching,
        });
        Ok(true)
    }

    pub(crate) fn mark_leased_prompt_steer_accepted(
        &mut self,
        leased_agent_id: &str,
        steer_id: &str,
        target_home_prompt_id: &str,
        worker_provider_run_id: &str,
    ) -> Result<(), DaemonError> {
        self.update_leased_prompt_steer_receipt(
            leased_agent_id,
            steer_id,
            target_home_prompt_id,
            worker_provider_run_id,
            LeasedPromptSteerReceiptPhase::Accepted,
        )
    }

    pub(crate) fn mark_leased_prompt_steer_rejected(
        &mut self,
        leased_agent_id: &str,
        steer_id: &str,
        target_home_prompt_id: &str,
        worker_provider_run_id: &str,
    ) -> Result<bool, DaemonError> {
        let Some(leased_agent) = self.app.leased_agents.get(leased_agent_id) else {
            return Ok(false);
        };
        let lease_id = leased_agent.lease_id.clone();
        if leased_agent
            .home_steer_receipts
            .iter()
            .any(|receipt| receipt.steer_id == steer_id)
        {
            // A caller without a dispatch-specific proof must never turn an
            // existing in-flight or accepted receipt into a rejection.
            return Ok(false);
        }
        if leased_agent
            .applied_home_steer_ids
            .iter()
            .any(|applied| applied == steer_id)
        {
            return Ok(false);
        }
        self.app
            .leased_agents
            .get_mut(leased_agent_id)
            .expect("leased agent checked above")
            .home_steer_receipts
            .push(LeasedPromptSteerReceipt {
                steer_id: steer_id.to_string(),
                target_home_prompt_id: target_home_prompt_id.to_string(),
                worker_provider_run_id: worker_provider_run_id.to_string(),
                execution_lease_id: lease_id,
                phase: LeasedPromptSteerReceiptPhase::Rejected,
            });
        Ok(true)
    }

    pub(crate) fn mark_leased_prompt_steer_definitely_not_accepted(
        &mut self,
        leased_agent_id: &str,
        steer_id: &str,
        target_home_prompt_id: &str,
        worker_provider_run_id: &str,
    ) -> Result<bool, DaemonError> {
        let Some(leased_agent) = self.app.leased_agents.get(leased_agent_id) else {
            return Ok(false);
        };
        let lease_id = leased_agent.lease_id.clone();
        let receipt = leased_agent
            .home_steer_receipts
            .iter()
            .find(|receipt| receipt.steer_id == steer_id)
            .cloned();
        if let Some(receipt) = receipt {
            if receipt.target_home_prompt_id != target_home_prompt_id
                || receipt.worker_provider_run_id != worker_provider_run_id
                || receipt.execution_lease_id != lease_id
            {
                return Ok(false);
            }
            return match receipt.phase {
                LeasedPromptSteerReceiptPhase::Rejected => Ok(true),
                LeasedPromptSteerReceiptPhase::Accepted => Ok(false),
                LeasedPromptSteerReceiptPhase::Dispatching => {
                    self.update_leased_prompt_steer_receipt(
                        leased_agent_id,
                        steer_id,
                        target_home_prompt_id,
                        worker_provider_run_id,
                        LeasedPromptSteerReceiptPhase::Rejected,
                    )?;
                    Ok(true)
                }
            };
        }
        if leased_agent
            .applied_home_steer_ids
            .iter()
            .any(|applied| applied == steer_id)
        {
            return Ok(false);
        }
        self.app
            .leased_agents
            .get_mut(leased_agent_id)
            .expect("leased agent checked above")
            .home_steer_receipts
            .push(LeasedPromptSteerReceipt {
                steer_id: steer_id.to_string(),
                target_home_prompt_id: target_home_prompt_id.to_string(),
                worker_provider_run_id: worker_provider_run_id.to_string(),
                execution_lease_id: lease_id,
                phase: LeasedPromptSteerReceiptPhase::Rejected,
            });
        Ok(true)
    }

    fn update_leased_prompt_steer_receipt(
        &mut self,
        leased_agent_id: &str,
        steer_id: &str,
        target_home_prompt_id: &str,
        worker_provider_run_id: &str,
        phase: LeasedPromptSteerReceiptPhase,
    ) -> Result<(), DaemonError> {
        let leased_agent = self
            .app
            .leased_agents
            .get_mut(leased_agent_id)
            .ok_or_else(|| DaemonError::LeasedAgentNotFound {
                leased_agent_id: leased_agent_id.to_string(),
            })?;
        let lease_id = leased_agent.lease_id.clone();
        let receipt = leased_agent
            .home_steer_receipts
            .iter_mut()
            .find(|receipt| receipt.steer_id == steer_id)
            .ok_or_else(|| DaemonError::LocalTransport {
                operation: "settle leased prompt steer",
                message: format!("steer `{steer_id}` has no durable dispatch receipt"),
            })?;
        if receipt.target_home_prompt_id != target_home_prompt_id
            || receipt.worker_provider_run_id != worker_provider_run_id
            || receipt.execution_lease_id != lease_id
        {
            return Err(DaemonError::LocalTransport {
                operation: "settle leased prompt steer",
                message: "steer receipt identity changed before settlement".to_string(),
            });
        }
        if receipt.phase != LeasedPromptSteerReceiptPhase::Dispatching
            && receipt.phase != phase
        {
            return Err(DaemonError::LocalTransport {
                operation: "settle leased prompt steer",
                message: "steer receipt already settled with a different outcome".to_string(),
            });
        }
        receipt.phase = phase;
        Ok(())
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
        let active_home_prompt_id = leased_agent.active_home_prompt_id.clone();
        let completion = self.app.complete_active_prompt(
            &leased_agent.backing_session_id,
            &leased_agent.backing_agent_id,
            provider_run_id.as_deref(),
        )?;
        if let Some(active_home_prompt_id) = active_home_prompt_id {
            let binding_key = self
                .app
                .leased_workflow_turns
                .iter()
                .find(|(_, binding)| {
                    binding.leased_agent_id == leased_agent_id
                        && binding.home_prompt_id == active_home_prompt_id
                        && provider_run_id.as_deref().is_some_and(|provider_run_id| {
                            binding.provider_run_id == provider_run_id
                        })
                })
                .map(|(key, _)| key.clone());
            if let Some(binding_key) = binding_key {
                self.app.leased_workflow_turns.remove(&binding_key);
            }
        }
        Ok(completion)
    }

    pub(crate) fn cancel_leased_prompt(
        &mut self,
        leased_agent_id: &str,
        expected_home_prompt_id: &str,
        expected_worker_provider_run_id: &str,
    ) -> Result<crate::session::PromptCancellation, DaemonError> {
        if expected_home_prompt_id.is_empty() || expected_worker_provider_run_id.is_empty() {
            return Err(DaemonError::LocalTransport {
                operation: "cancel leased prompt",
                message: "cancellation requires an exact home prompt and worker provider run"
                    .to_string(),
            });
        }
        let leased_agent = self
            .app
            .leased_agents
            .get(leased_agent_id)
            .cloned()
            .ok_or_else(|| DaemonError::LeasedAgentNotFound {
                leased_agent_id: leased_agent_id.to_string(),
            })?;
        if leased_agent.active_home_prompt_id.as_deref() != Some(expected_home_prompt_id) {
            return Err(DaemonError::LocalTransport {
                operation: "cancel leased prompt",
                message: "active home prompt did not match the cancellation identity".to_string(),
            });
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
        if leased_agent.active_home_prompt_started_at_ms != Some(active_prompt.created_at_ms())
            || !matches!(
                active_prompt.status(),
                crate::session::PromptStatus::Dispatching
                    | crate::session::PromptStatus::Running
                    | crate::session::PromptStatus::Cancelling
            )
        {
            return Err(DaemonError::LocalTransport {
                operation: "cancel leased prompt",
                message: "active worker prompt did not match the cancellation identity".to_string(),
            });
        }
        let provider_run_id = self
            .app
            .providers
            .get_run_for_agent(
                &leased_agent.backing_session_id,
                &leased_agent.backing_agent_id,
            )
            .map(|run| run.id().to_string());
        if provider_run_id.as_deref() != Some(expected_worker_provider_run_id) {
            return Err(DaemonError::LocalTransport {
                operation: "cancel leased prompt",
                message: "active worker provider run did not match the cancellation identity"
                    .to_string(),
            });
        }
        let cancellation = self.app.cancel_active_prompt_internal(
            &leased_agent.backing_session_id,
            &leased_agent.backing_agent_id,
            None,
        )?;
        self.app.leased_workflow_turns.retain(|_, binding| {
            binding.leased_agent_id != leased_agent_id
                || binding.home_prompt_id != expected_home_prompt_id
                || binding.provider_run_id != expected_worker_provider_run_id
        });
        Ok(cancellation)
    }

    pub(crate) fn complete_leased_workflow_prompt_for_provider_run(
        &mut self,
        provider_run_id: &str,
    ) -> Result<Option<crate::session::PromptCompletion>, DaemonError> {
        let Some((binding_key, binding)) = self
            .app
            .leased_workflow_turns
            .iter()
            .find(|(_, binding)| {
                binding.provider_run_id == provider_run_id
                    && self
                        .app
                        .leased_agents
                        .get(&binding.leased_agent_id)
                        .and_then(|agent| agent.active_home_prompt_id.as_deref())
                        == Some(binding.home_prompt_id.as_str())
            })
            .map(|(key, binding)| (key.clone(), binding.clone()))
        else {
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
            self.app.leased_workflow_turns.remove(&binding_key);
            return Ok(None);
        }
        let completion = self.app.complete_active_prompt(
            &leased_agent.backing_session_id,
            &leased_agent.backing_agent_id,
            Some(provider_run_id),
        )?;
        self.app.leased_workflow_turns.remove(&binding_key);
        Ok(Some(completion))
    }
}

fn join_hidden_context(first: &str, second: &str) -> String {
    match (first.trim(), second.trim()) {
        ("", "") => String::new(),
        ("", second) => second.to_string(),
        (first, "") => first.to_string(),
        (first, second) => format!("{first}\n\n{second}"),
    }
}

#[cfg(test)]
mod receipt_tests {
    use super::*;

    fn submit_cancellable_prompt(
        app: &mut crate::app::DaemonApp,
        suffix: &str,
    ) -> (String, String, String, String, String) {
        let lease = RemoteLeaseRuntime::new(app)
            .create_execution_lease(
                "home-kernel-cancel-test",
                &format!("home-session-{suffix}"),
                &format!("home-agent-{suffix}"),
                false,
                &format!("user-{suffix}"),
            )
            .expect("execution lease should be created");
        let leased_agent = RemoteLeaseRuntime::new(app)
            .create_leased_agent(
                &lease.id,
                "managed-dev-stub",
                "default",
                Some("sonnet".to_string()),
                None,
                None,
                None,
                None,
                None,
                None,
            )
            .expect("leased agent should be created");
        let home_prompt_id = format!("home-prompt-{suffix}");
        let context = RemoteGitTurnContext {
            home_session_id: lease.home_session_id.clone(),
            home_agent_id: lease.home_agent_id.clone(),
            home_prompt_id: home_prompt_id.clone(),
            home_turn_id: format!("home-turn-{suffix}"),
            source_attachment_id: None,
            workspace_live_sync_mode: None,
            prompt_origin: None,
            external_provider: None,
            external_provider_session_id: None,
            external_provider_turn_id: None,
            prompt_summary: "run-scoped cancellation test".to_string(),
        };
        let (provider_run_id, outcome) = RemoteLeaseRuntime::new(app)
            .submit_leased_prompt_with_workflow_context(
                &leased_agent.id,
                "run-scoped cancellation prompt\n",
                Vec::new(),
                None,
                Some(context),
                Vec::new(),
                None,
                crate::extension::RemoteExtensionManifest::default(),
            )
            .expect("worker should accept the prompt");
        assert!(matches!(
            outcome,
            PromptSubmissionOutcome::Started { .. }
        ));
        (
            leased_agent.id,
            leased_agent.backing_session_id,
            leased_agent.backing_agent_id,
            home_prompt_id,
            provider_run_id,
        )
    }

    #[test]
    fn leased_prompt_cancellation_rejects_mismatched_identity_without_touching_active_prompt() {
        let mut config = crate::config::DaemonConfig::for_tests();
        config.accept_remote_leases = true;
        let mut app = crate::app::DaemonApp::bootstrap(config).expect("worker should boot");
        let (leased_agent_id, session_id, agent_id, home_prompt_id, provider_run_id) =
            submit_cancellable_prompt(&mut app, "cancel-identity-test");
        let active_before = app
            .prompt_owner_active_prompt_for_agent(&session_id, &agent_id)
            .expect("active prompt lookup should succeed")
            .expect("leased prompt should be active");

        for (requested_home_prompt_id, requested_run_id) in [
            ("successor-home-prompt", provider_run_id.as_str()),
            (home_prompt_id.as_str(), "successor-provider-run"),
        ] {
            assert!(
                RemoteLeaseRuntime::new(&mut app)
                    .cancel_leased_prompt(
                        &leased_agent_id,
                        requested_home_prompt_id,
                        requested_run_id,
                    )
                    .is_err(),
                "cancellation must reject a different home prompt or provider run"
            );
            let active_after = app
                .prompt_owner_active_prompt_for_agent(&session_id, &agent_id)
                .expect("active prompt lookup should succeed")
                .expect("mismatched cancellation must leave the active prompt in place");
            assert_eq!(active_after.id(), active_before.id());
            assert_eq!(active_after.status(), active_before.status());
            assert_eq!(
                app.leased_agents
                    .get(&leased_agent_id)
                    .and_then(|leased_agent| leased_agent.active_home_prompt_id.as_deref()),
                Some(home_prompt_id.as_str()),
                "mismatched cancellation must retain the actual home-prompt binding"
            );
            let active_run_id = app
                .providers
                .get_run_for_agent(&session_id, &agent_id)
                .map(|run| run.id().to_string());
            assert_eq!(
                active_run_id.as_deref(),
                Some(provider_run_id.as_str()),
                "mismatched cancellation must not change the active provider run"
            );
        }
    }

    #[test]
    fn leased_prompt_receipt_requires_exact_active_or_completed_worker_evidence() {
        let mut config = crate::config::DaemonConfig::for_tests();
        config.accept_remote_leases = true;
        let mut app = crate::app::DaemonApp::bootstrap(config).expect("worker should boot");
        let lease = RemoteLeaseRuntime::new(&mut app)
            .create_execution_lease(
                "home-kernel-receipt-test",
                "home-session-receipt-test",
                "home-agent-receipt-test",
                false,
                "user-receipt-test",
            )
            .expect("execution lease should be created");
        let leased_agent = RemoteLeaseRuntime::new(&mut app)
            .create_leased_agent(
                &lease.id,
                "managed-dev-stub",
                "default",
                Some("sonnet".to_string()),
                None,
                None,
                None,
                None,
                None,
                None,
            )
            .expect("leased agent should be created");
        let home_prompt_id = "home-prompt-receipt-test";
        let context = RemoteGitTurnContext {
            home_session_id: lease.home_session_id.clone(),
            home_agent_id: lease.home_agent_id.clone(),
            home_prompt_id: home_prompt_id.to_string(),
            home_turn_id: "home-turn-receipt-test".to_string(),
            source_attachment_id: None,
            workspace_live_sync_mode: None,
            prompt_origin: None,
            external_provider: None,
            external_provider_session_id: None,
            external_provider_turn_id: None,
            prompt_summary: "receipt query test".to_string(),
        };
        let (provider_run_id, outcome) = RemoteLeaseRuntime::new(&mut app)
            .submit_leased_prompt_with_workflow_context(
                &leased_agent.id,
                "receipt query prompt\n",
                Vec::new(),
                None,
                Some(context),
                Vec::new(),
                None,
                crate::extension::RemoteExtensionManifest::default(),
            )
            .expect("worker should accept the prompt");
        assert!(matches!(outcome, PromptSubmissionOutcome::Started { .. }));

        let active = RemoteLeaseRuntime::new(&mut app)
            .leased_prompt_receipt(&leased_agent.id, home_prompt_id)
            .expect("active receipt query should succeed")
            .expect("exact active prompt should have a receipt");
        assert_eq!(active.home_prompt_id, home_prompt_id);
        assert_eq!(active.worker_provider_run_id, provider_run_id);
        assert_eq!(active.phase, LeasedPromptReceiptPhase::Active);
        assert!(RemoteLeaseRuntime::new(&mut app)
            .leased_prompt_receipt(&leased_agent.id, "different-home-prompt")
            .expect("unmatched receipt query should succeed")
            .is_none());

        RemoteLeaseRuntime::new(&mut app)
            .complete_leased_prompt(&leased_agent.id)
            .expect("worker prompt should complete");
        let agent = app
            .leased_agents
            .get_mut(&leased_agent.id)
            .expect("leased agent should remain registered");
        agent.active_home_prompt_id = None;
        agent.active_home_prompt_started_at_ms = None;
        agent.replayable_completion = Some(crate::execution_lease::LeasedCompletionReplay {
            provider_run_id: provider_run_id.clone(),
            message_id: "worker-completion-receipt-test".to_string(),
            completed_at_ms: crate::session::unix_epoch_ms(),
            home_prompt_id: Some(home_prompt_id.to_string()),
            provider_termination: None,
        });

        let completed = RemoteLeaseRuntime::new(&mut app)
            .leased_prompt_receipt(&leased_agent.id, home_prompt_id)
            .expect("completed receipt query should succeed")
            .expect("exact completed prompt should have a receipt");
        assert_eq!(completed.home_prompt_id, home_prompt_id);
        assert_eq!(completed.worker_provider_run_id, provider_run_id);
        assert_eq!(completed.phase, LeasedPromptReceiptPhase::Completed);

        app.leased_agents
            .get_mut(&leased_agent.id)
            .expect("leased agent should remain registered")
            .replayable_completion
            .as_mut()
            .expect("completion receipt should exist")
            .provider_run_id = "unrelated-worker-run".to_string();
        assert!(RemoteLeaseRuntime::new(&mut app)
            .leased_prompt_receipt(&leased_agent.id, home_prompt_id)
            .expect("invalid completion receipt query should succeed")
            .is_none());
    }

    #[test]
    fn queued_steer_receipt_is_exact_durable_and_rejected_only_after_proven_non_admission() {
        let mut config = crate::config::DaemonConfig::for_tests();
        config.accept_remote_leases = true;
        let mut app = crate::app::DaemonApp::bootstrap(config).expect("worker should boot");
        let lease = RemoteLeaseRuntime::new(&mut app)
            .create_execution_lease(
                "home-kernel-steer-receipt-test",
                "home-session-steer-receipt-test",
                "home-agent-steer-receipt-test",
                false,
                "user-steer-receipt-test",
            )
            .expect("execution lease should be created");
        let leased_agent = RemoteLeaseRuntime::new(&mut app)
            .create_leased_agent(
                &lease.id,
                "managed-dev-stub",
                "default",
                Some("sonnet".to_string()),
                None,
                None,
                None,
                None,
                None,
                None,
            )
            .expect("leased agent should be created");
        let target_home_prompt_id = "home-prompt-steer-receipt-test";
        let (worker_provider_run_id, outcome) = RemoteLeaseRuntime::new(&mut app)
            .submit_leased_prompt(
                &leased_agent.id,
                "active receipt target\n",
                Vec::new(),
            )
            .expect("worker should accept the active prompt");
        assert!(matches!(outcome, PromptSubmissionOutcome::Started { .. }));
        app.leased_agents
            .get_mut(&leased_agent.id)
            .expect("leased agent should remain registered")
            .active_home_prompt_id = Some(target_home_prompt_id.to_string());

        let steer_id = "home-queued-steer-receipt-test";
        let (_, dispatch) = RemoteLeaseRuntime::new(&mut app)
            .prepare_leased_prompt_steer(
                &leased_agent.id,
                steer_id,
                target_home_prompt_id,
                "queued steer payload\n",
                "",
                Vec::new(),
                None,
            )
            .expect("worker should prepare exact steer");
        assert!(dispatch.is_some(), "first delivery must prepare one dispatch");
        assert!(RemoteLeaseRuntime::new(&mut app)
            .reserve_leased_prompt_steer(
                &leased_agent.id,
                steer_id,
                target_home_prompt_id,
                &worker_provider_run_id,
            )
            .expect("worker should durably reserve exact steer"));
        RemoteLeaseRuntime::new(&mut app)
            .mark_leased_prompt_steer_accepted(
                &leased_agent.id,
                steer_id,
                target_home_prompt_id,
                &worker_provider_run_id,
            )
            .expect("worker should settle accepted delivery");

        let (_, duplicate_dispatch) = RemoteLeaseRuntime::new(&mut app)
            .prepare_leased_prompt_steer(
                &leased_agent.id,
                steer_id,
                target_home_prompt_id,
                "queued steer payload\n",
                "",
                Vec::new(),
                None,
            )
            .expect("exact retry should read the prior receipt");
        assert!(duplicate_dispatch.is_none(), "accepted steer must never dispatch twice");
        assert!(RemoteLeaseRuntime::new(&mut app)
            .reserve_leased_prompt_steer(
                &leased_agent.id,
                steer_id,
                target_home_prompt_id,
                "different-worker-run",
            )
            .is_err(), "a receipt cannot be reused for a different worker run");

        let rejected_steer_id = "home-rejected-steer-receipt-test";
        let (_, rejected_dispatch) = RemoteLeaseRuntime::new(&mut app)
            .prepare_leased_prompt_steer(
                &leased_agent.id,
                rejected_steer_id,
                target_home_prompt_id,
                "rejected steer payload\n",
                "",
                Vec::new(),
                None,
            )
            .expect("second exact steer should prepare");
        assert!(rejected_dispatch.is_some());
        assert!(RemoteLeaseRuntime::new(&mut app)
            .reserve_leased_prompt_steer(
                &leased_agent.id,
                rejected_steer_id,
                target_home_prompt_id,
                &worker_provider_run_id,
            )
            .expect("second steer should reserve"));
        assert!(!RemoteLeaseRuntime::new(&mut app)
            .mark_leased_prompt_steer_rejected(
                &leased_agent.id,
                rejected_steer_id,
                target_home_prompt_id,
                &worker_provider_run_id,
            )
            .expect("an ambiguous Dispatching receipt must remain held"));
        let dispatching = RemoteLeaseRuntime::new(&mut app)
            .leased_prompt_receipt(&leased_agent.id, rejected_steer_id)
            .expect("dispatching receipt query should succeed")
            .expect("reservation should leave an exact dispatching receipt");
        assert_eq!(dispatching.phase, LeasedPromptReceiptPhase::SteerDispatching);
        assert!(RemoteLeaseRuntime::new(&mut app)
            .mark_leased_prompt_steer_definitely_not_accepted(
                &leased_agent.id,
                rejected_steer_id,
                target_home_prompt_id,
                &worker_provider_run_id,
            )
            .expect("known non-admission should settle rejection"));
        assert!(RemoteLeaseRuntime::new(&mut app)
            .mark_leased_prompt_steer_accepted(
                &leased_agent.id,
                rejected_steer_id,
                target_home_prompt_id,
                &worker_provider_run_id,
            )
            .is_err(), "a rejected receipt cannot be upgraded to acceptance");

        let tombstone_steer_id = "home-tombstone-steer-receipt-test";
        let tombstone = RemoteLeaseRuntime::new(&mut app)
            .reconcile_leased_prompt_steer_receipt(
                &leased_agent.id,
                tombstone_steer_id,
                target_home_prompt_id,
                &worker_provider_run_id,
                &lease.id,
            )
            .expect("exact reconciliation should create a non-acceptance tombstone");
        assert_eq!(tombstone.phase, LeasedPromptReceiptPhase::SteerRejected);
        assert!(RemoteLeaseRuntime::new(&mut app)
            .prepare_leased_prompt_steer(
                &leased_agent.id,
                tombstone_steer_id,
                target_home_prompt_id,
                "late original relay request\n",
                "",
                Vec::new(),
                None,
            )
            .is_err(), "a delayed original request must not dispatch after a rejection tombstone");
        assert!(RemoteLeaseRuntime::new(&mut app)
            .reconcile_leased_prompt_steer_receipt(
                &leased_agent.id,
                tombstone_steer_id,
                target_home_prompt_id,
                "different-worker-run",
                &lease.id,
            )
            .is_err(), "a tombstone cannot be queried under a different worker run");

        RemoteLeaseRuntime::new(&mut app)
            .complete_leased_prompt(&leased_agent.id)
            .expect("target provider prompt should complete");
        let agent = app
            .leased_agents
            .get_mut(&leased_agent.id)
            .expect("leased agent should remain registered");
        agent.active_home_prompt_id = None;
        agent.active_home_prompt_started_at_ms = None;

        for (queried_steer_id, expected_phase) in [
            (steer_id, LeasedPromptReceiptPhase::SteerAccepted),
            (rejected_steer_id, LeasedPromptReceiptPhase::SteerRejected),
            (tombstone_steer_id, LeasedPromptReceiptPhase::SteerRejected),
        ] {
            let receipt = RemoteLeaseRuntime::new(&mut app)
                .leased_prompt_receipt(&leased_agent.id, queried_steer_id)
                .expect("exact queued-steer query should succeed")
                .expect("worker should retain the durable steer receipt after completion");
            assert_eq!(receipt.home_prompt_id, queried_steer_id);
            assert_eq!(receipt.worker_provider_run_id, worker_provider_run_id);
            assert_eq!(receipt.phase, expected_phase);
            assert_eq!(
                receipt.target_home_prompt_id.as_deref(),
                Some(target_home_prompt_id)
            );
            assert_eq!(
                receipt.execution_lease_id.as_deref(),
                Some(lease.id.as_str())
            );
        }
    }
}
