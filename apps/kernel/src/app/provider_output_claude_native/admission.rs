//! MP-08 / MP-10 / MP-11: context requests precede hook admission. They do
//! not acknowledge a managed dispatch or create a native prompt lifecycle.
use super::*;

impl ProviderOutputClaudeNativeBridge<'_> {
    pub(super) fn respond_to_prompt_context_request(
        &mut self,
        session_id: &str,
        provider_run_id: &str,
        agent_id: &str,
        context_file: &str,
        event: &Value,
    ) -> Result<(), DaemonError> {
        let Some(request_id) = event.get("hook_context_request_id").and_then(Value::as_str) else {
            return Ok(());
        };
        let Some(prompt) = event
            .get("prompt")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|prompt| !prompt.is_empty())
        else {
            return Ok(());
        };
        if claude_native_prompt_is_internal_control(prompt) {
            return Ok(());
        }
        let active = self
            .app
            .prompt_owner_active_prompt_for_agent(session_id, agent_id)?;
        let marker = claude_native_marker(context_file);
        let managed = marker
            .as_deref()
            .and_then(claude_native_dispatch_prompt_id)
            .is_some_and(|id| {
                active.as_ref().is_some_and(|active| active.id() == id)
                    || claude_headless_dispatch_matches_prompt(context_file, id, prompt)
            });
        // Preserve the managed turn's already assembled context. This request
        // is not its acknowledgement, even if the context file is nonempty.
        let context = if managed {
            fs::read_to_string(context_file).unwrap_or_default()
        } else {
            // MP-08 / MP-10 / MP-11: no legacy grant context. The shared V2
            // assembler must be wired here before advertising runtime support.
            String::new()
        };
        crate::provider::ensure_claude_native_hidden_context_fits(provider_run_id, &context)?;
        fs::write(context_file, &context).map_err(|error| DaemonError::ProviderProtocol {
            provider_run_id: provider_run_id.to_string(),
            operation: "claude_hidden_context_write",
            message: error.to_string(),
        })?;
        write_claude_hook_context_response(context_file, request_id, &context);
        Ok(())
    }

    pub(super) fn reject_hook_prompt(
        &mut self,
        session_id: &str,
        provider_run_id: &str,
        agent_id: &str,
        context_file: &str,
        event: &Value,
    ) -> Result<(), DaemonError> {
        // An unadmitted native user turn has no prompt to settle. A managed
        // turn already exists: reject only the exact injection's prompt ID.
        let Some(id) = event.get("native_prompt_id").and_then(Value::as_str) else {
            return Ok(());
        };
        let Some(observed) = event.get("prompt").and_then(Value::as_str) else {
            return Ok(());
        };
        let marker = claude_native_marker(context_file);
        if marker.as_deref().and_then(claude_native_dispatch_prompt_id) != Some(id)
            || !self
                .app
                .prompt_owner_active_prompt_for_agent(session_id, agent_id)?
                .is_some_and(|prompt| {
                    prompt.id() == id
                        && (claude_headless_prompt_matches(prompt.prompt(), observed)
                            || claude_headless_dispatch_matches_prompt(context_file, id, observed))
                })
        {
            return Ok(());
        }
        let _ = fs::write(context_file, "");
        write_claude_native_marker(context_file, "");
        let _ = fs::remove_file(format!("{context_file}.approval-origin"));
        self.app.record_notice(
            session_id,
            Some(provider_run_id),
            self.app.attachments.list_session_attachment_ids(session_id),
            "Chariox skill context was not delivered. Refresh Skills and retry this prompt.",
        );
        self.app
            .fail_active_prompt(session_id, agent_id, Some(provider_run_id))?;
        Ok(())
    }
}
