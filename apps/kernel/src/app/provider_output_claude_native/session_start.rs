//! MP-08 / MP-10 / MP-11: official SessionStart identity precedes transcript creation.
use super::*;

impl ProviderOutputClaudeNativeBridge<'_> {
    pub(crate) fn refresh_session_start(
        &mut self,
        provider_run_id: &str,
    ) -> Result<(), DaemonError> {
        let run = self.app.providers.get_run(provider_run_id)?;
        let Some(events) = run.pty_env().get("CHARIOX_CLAUDE_NATIVE_EVENTS") else {
            return Ok(());
        };
        // SessionStart can arrive between the early peek and event consumption.
        // Its separate receipt survives that drain until the next native reset.
        let receipt = run
            .pty_env()
            .get("CHARIOX_CLAUDE_NATIVE_CONTEXT")
            .and_then(|context| fs::read_to_string(format!("{context}.session-start-id")).ok())
            .map(|session| session.trim().to_string())
            .filter(|session| !session.is_empty());
        let raw = fs::read_to_string(events).unwrap_or_default();
        let session = receipt.or_else(|| {
            raw.lines()
                .filter_map(|line| serde_json::from_str::<Value>(line).ok())
                .filter(|event| {
                    event.get("hook_event_name").and_then(Value::as_str) == Some("SessionStart")
                })
                .filter_map(|event| {
                    event
                        .get("session_id")
                        .and_then(Value::as_str)
                        .map(str::trim)
                        .filter(|id| !id.is_empty())
                        .map(str::to_string)
                })
                .next_back()
        });
        let Some(session) = session else {
            return Ok(());
        };
        if run.provider_session_id() == Some(session.as_str()) {
            return Ok(());
        }
        let metadata = ProviderPromptSignalBatch {
            resolved_resume_state: Some(ProviderResumeState::from_claude_session_id(session)),
            ..Default::default()
        };
        self.app
            .providers
            .apply_structured_output_metadata(provider_run_id, &metadata)?;
        let run = self.app.providers.get_run(provider_run_id)?;
        super::super::provider_output_resume::persist_resolved_resume_state(
            self.app, &run, &metadata,
        )?;
        self.app.update_provider_run_projection(run);
        Ok(())
    }
}
