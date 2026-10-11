//! MP-08 / MP-10 / MP-11: assemble current managed hook context before handoff rendering.
use super::*;

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
            ])
        } else {
            let scheduled_hidden = crate::prompt_assembly::strip_prompt_manifest_entries(&hidden);
            join_claude_context([
                scheduled_hidden,
                origin_context,
                native_hidden,
                attachment_context,
            ])
        };
        Ok(hidden_context)
    }
}
