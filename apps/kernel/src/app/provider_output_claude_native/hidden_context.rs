//! MP-08 / MP-10 / MP-11: assemble current managed hook context before handoff rendering.
use super::*;

use crate::provider::skill_delivery::TypedSkillLoad;
use serde::{Deserialize, Serialize};

#[derive(Deserialize, Serialize)]
pub(super) struct ManagedPromptInputs {
    pub(super) prompt_id: String,
    pub(super) origin_prompt_id: String,
    pub(super) prompt: String,
    pub(super) hidden_system_context: String,
    pub(super) attachments: Vec<PromptAttachment>,
    /// The validated skill delivery rides beside the scheduled inputs: the hook
    /// rebuilds the rest, but only the dispatch resolved and recorded the skills.
    pub(super) skill_context: String,
}

pub(super) fn stage_managed_inputs(
    context_file: &str,
    prompt: &ClaudeNativePromptInjection<'_>,
    skill_context: &str,
) -> Result<(), DaemonError> {
    let inputs = ManagedPromptInputs {
        prompt_id: prompt.id.into(),
        origin_prompt_id: prompt.origin_prompt_id.into(),
        prompt: prompt.prompt.into(),
        hidden_system_context: prompt.hidden_system_context.into(),
        attachments: prompt.attachments.to_vec(),
        skill_context: skill_context.into(),
    };
    fs::write(
        format!("{context_file}.managed-inputs"),
        serde_json::to_vec(&inputs).expect("managed prompt inputs are serializable"),
    )
    .map_err(|error| DaemonError::LocalTransport {
        operation: "claude_managed_inputs_write",
        message: error.to_string(),
    })
}

pub(super) fn load_managed_inputs(context_file: &str) -> Result<ManagedPromptInputs, DaemonError> {
    let raw = fs::read(format!("{context_file}.managed-inputs")).map_err(|error| {
        DaemonError::LocalTransport {
            operation: "claude_managed_inputs_read",
            message: error.to_string(),
        }
    })?;
    serde_json::from_slice(&raw).map_err(|_| DaemonError::LocalTransport {
        operation: "claude_managed_inputs_read",
        message: "Invalid managed prompt input metadata".into(),
    })
}

impl ProviderOutputClaudeNativeBridge<'_> {
    pub(super) fn managed_base_context(
        &mut self,
        session_id: &str,
        context_file: &str,
        provider_run: &RuntimeProviderRun,
        prompt: &ClaudeNativePromptInjection<'_>,
    ) -> Result<String, DaemonError> {
        let (text, hidden, attachments) = match self.prompt_handoff {
            Some(policy) => policy.protect_inputs(
                session_id,
                prompt.prompt.into(),
                prompt.hidden_system_context.into(),
                prompt.attachments.to_vec(),
            )?,
            None => (
                prompt.prompt.into(),
                prompt.hidden_system_context.into(),
                prompt.attachments.to_vec(),
            ),
        };
        let (attachments, notes_context) =
            crate::prompt_notes::split_for_provider(attachments, provider_run.agent_instance_id());
        let visible = redact_native_hidden_instructions(&text).trim().to_string();
        let native_hidden = extract_native_hidden_instructions(&text);
        let attachment_context = format_claude_attachment_context(&attachments, context_file);
        let origin_context =
            crate::prompt_assembly::agent_message_origin_context(prompt.origin_prompt_id);
        let hidden_context = if provider_run.provider() == "claude-headless" {
            let envelope = crate::prompt_assembly::PromptAssemblyService::from_env()?
                .assemble_provider_turn(
                    provider_run,
                    &visible,
                    Some(&hidden),
                    attachments.clone(),
                    crate::prompt_assembly::PromptAssemblyMode::NormalProviderTurn,
                )?;
            join_claude_context([
                envelope.hidden_system_context,
                origin_context,
                native_hidden,
                attachment_context,
                notes_context.unwrap_or_default(),
            ])
        } else {
            let scheduled_hidden = crate::prompt_assembly::strip_prompt_manifest_entries(&hidden);
            join_claude_context([
                scheduled_hidden,
                origin_context,
                native_hidden,
                attachment_context,
                notes_context.unwrap_or_default(),
            ])
        };
        // MP-08 / MP-10 / MP-11: hook ACK rebuilds the original scheduled inputs.
        // Fit optional lease replay again after adding current worker instructions.
        crate::provider::fit_claude_native_hidden_context(provider_run.id(), hidden_context)
    }

    /// A `$name` typed in Claude's own composer reaches the kernel only as the
    /// hook prompt. It resolves like any user prompt and its instructions ride the
    /// hook context. Native commands instead replace the blocked original prompt.
    pub(super) fn typed_skill_context(
        &mut self,
        session_id: &str,
        agent_id: &str,
        provider_run: &RuntimeProviderRun,
        prompt: &crate::session::PromptQueueItem,
        context: String,
    ) -> Result<TypedSkillContext, DaemonError> {
        if let Some(leased_agent_id) = crate::app::RemoteLeaseRuntime::new(self.app)
            .leased_agent_id_for_backing(session_id, agent_id)
        {
            return self.leased_typed_skill_context(
                &leased_agent_id,
                provider_run,
                prompt,
                context,
            );
        }
        let envelope = crate::prompt_assembly::PromptEnvelope::new(
            prompt.prompt(),
            "",
            Vec::new(),
            crate::prompt_assembly::PromptManifest::default(),
        );
        let delivery = self
            .app
            .providers
            .prepare_skill_prompt(
                session_id,
                agent_id,
                prompt.id(),
                prompt.id(),
                provider_run,
                envelope,
            )?
            .skill_delivery;
        let Some(delivery) = delivery else {
            return Ok(TypedSkillContext::plain(context));
        };
        let native_command = (!delivery.requires_native_load_ack.is_empty()
            && delivery.visible_prompt.starts_with('/'))
        .then_some(delivery.visible_prompt);
        Ok(TypedSkillContext {
            context: crate::provider::fit_claude_native_hidden_context(
                provider_run.id(),
                join_claude_context([context, delivery.hidden_context]),
            )?,
            loads: Vec::new(),
            native_command,
        })
    }

    /// The home resolves current and active selections; the worker materializes
    /// only its lease-bound packages and records loads after hook delivery.
    fn leased_typed_skill_context(
        &mut self,
        leased_agent_id: &str,
        provider_run: &RuntimeProviderRun,
        prompt: &crate::session::PromptQueueItem,
        context: String,
    ) -> Result<TypedSkillContext, DaemonError> {
        let worktree = self
            .app
            .sessions
            .get_session(provider_run.session_id())?
            .worktree_id()
            .to_string();
        let lease = crate::app::RemoteLeaseRuntime::new(self.app);
        let mut authority = lease
            .leased_workspace_live_sync_context_for_provider_run(
                provider_run.id(),
                crate::io::WorkspaceIdentity::local(&worktree),
            )
            .ok_or_else(|| DaemonError::LocalTransport {
                operation: "prepare native worker skills",
                message: "the native execution lease ended; reattach the provider".into(),
            })?;
        authority.home_prompt_id = None;
        let agent = self.app.leased_agents.get(leased_agent_id).ok_or_else(|| {
            DaemonError::LeasedAgentNotFound {
                leased_agent_id: leased_agent_id.into(),
            }
        })?;
        let lease_id = agent.lease_id.clone();
        let names = agent.skill_names.clone();
        let delivery = self.app.providers.prepare_typed_leased_skills(
            provider_run,
            &authority,
            prompt,
            &lease_id,
            names.as_ref(),
        )?;
        if let Some(names) = delivery.leased_names {
            if let Some(agent) = self.app.leased_agents.get_mut(leased_agent_id) {
                if names.catalog_revision >= agent.skill_view_revision {
                    agent.skill_view_revision = names.catalog_revision;
                    agent.skill_names = Some(names);
                }
            }
        }
        let native_command = (!delivery.requires_native_load_ack.is_empty()
            && delivery.visible_prompt.starts_with('/'))
        .then_some(delivery.visible_prompt);
        Ok(TypedSkillContext {
            context: crate::provider::fit_claude_native_hidden_context(
                provider_run.id(),
                join_claude_context([context, delivery.hidden_context]),
            )?,
            loads: delivery.typed_leased_loads,
            native_command,
        })
    }

    pub(super) fn skills_not_delivered(
        &mut self,
        session_id: &str,
        provider_run: &RuntimeProviderRun,
        error: &DaemonError,
    ) {
        self.app.record_notice(
            session_id,
            Some(provider_run.id()),
            self.app.attachments.list_session_attachment_ids(session_id),
            format!("Skills were not delivered to this turn: {error}"),
        );
    }

    /// The load of a typed skill is the home's to record; a refused acknowledgement
    /// is a visible notice and never ends the hook that already delivered the body.
    pub(super) fn acknowledge_typed_skills(
        &mut self,
        session_id: &str,
        run_id: &str,
        prompt_id: &str,
        skills: &[TypedSkillLoad],
    ) -> Result<(), DaemonError> {
        let worktree = self
            .app
            .sessions
            .get_session(session_id)?
            .worktree_id()
            .to_string();
        let Some(mut context) = crate::app::RemoteLeaseRuntime::new(self.app)
            .leased_workspace_live_sync_context_for_provider_run(
                run_id,
                crate::io::WorkspaceIdentity::local(&worktree),
            )
        else {
            return Ok(());
        };
        // The home has no dispatched turn for text typed on the worker.
        context.home_prompt_id = None;
        if let Err(error) = self
            .app
            .providers
            .acknowledge_typed_leased_skills(run_id, &context, prompt_id, skills)
        {
            self.app.record_notice(
                session_id,
                Some(run_id),
                self.app.attachments.list_session_attachment_ids(session_id),
                format!("A typed skill load was not recorded: {error}"),
            );
        }
        Ok(())
    }
}

/// The hook context of a typed prompt, with the skills the worker put into it.
pub(super) struct TypedSkillContext {
    pub(super) context: String,
    pub(super) loads: Vec<TypedSkillLoad>,
    pub(super) native_command: Option<String>,
}

impl TypedSkillContext {
    fn plain(context: String) -> Self {
        Self {
            context,
            loads: Vec::new(),
            native_command: None,
        }
    }
}
