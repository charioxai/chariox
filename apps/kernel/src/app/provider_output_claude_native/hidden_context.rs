//! MP-08 / MP-10 / MP-11: assemble current managed hook context before handoff rendering.
use super::*;

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
    /// hook context; a refusal is a visible notice, never a silently empty context.
    pub(super) fn typed_skill_context(
        &mut self,
        session_id: &str,
        agent_id: &str,
        provider_run: &RuntimeProviderRun,
        prompt: &crate::session::PromptQueueItem,
        context: String,
    ) -> Result<String, DaemonError> {
        let envelope = crate::prompt_assembly::PromptEnvelope::new(
            prompt.prompt(),
            "",
            Vec::new(),
            crate::prompt_assembly::PromptManifest::default(),
        );
        let delivery = match self.app.providers.prepare_skill_prompt(
            session_id,
            agent_id,
            prompt.id(),
            prompt.id(),
            provider_run,
            envelope,
        ) {
            Ok(envelope) => envelope.skill_delivery,
            Err(error) => {
                self.app.record_notice(
                    session_id,
                    Some(provider_run.id()),
                    self.app.attachments.list_session_attachment_ids(session_id),
                    format!("Skills were not delivered to this turn: {error}"),
                );
                return Ok(context);
            }
        };
        let Some(delivery) = delivery.filter(|delivery| !delivery.hidden_context.is_empty()) else {
            return Ok(context);
        };
        crate::provider::fit_claude_native_hidden_context(
            provider_run.id(),
            join_claude_context([context, delivery.hidden_context]),
        )
    }
}
