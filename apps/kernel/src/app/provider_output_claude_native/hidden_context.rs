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
}

pub(super) fn stage_managed_inputs(
    context_file: &str,
    prompt: &ClaudeNativePromptInjection<'_>,
) -> Result<(), DaemonError> {
    let inputs = ManagedPromptInputs {
        prompt_id: prompt.id.into(),
        origin_prompt_id: prompt.origin_prompt_id.into(),
        prompt: prompt.prompt.into(),
        hidden_system_context: prompt.hidden_system_context.into(),
        attachments: prompt.attachments.to_vec(),
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
        agent_id: &str,
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
            let skill_context =
                self.claude_native_prompt_context(session_id, agent_id, &visible)?;
            join_claude_context([
                envelope.hidden_system_context,
                origin_context,
                skill_context,
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
}
