use std::fs;

use serde_json::Value;

use crate::app::DaemonApp;
use crate::error::DaemonError;
use crate::provider::RuntimeProviderRun;

fn claude_native_marker(context_file: &str) -> Option<String> {
    let marker = std::path::Path::new(context_file).with_file_name("active-prompt-id");
    fs::read_to_string(marker)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn write_claude_native_marker(context_file: &str, value: &str) {
    let marker = std::path::Path::new(context_file).with_file_name("active-prompt-id");
    let _ = fs::write(marker, value);
}

fn extract_native_hidden_instructions(prompt: &str) -> String {
    let start = crate::provider::NATIVE_TUI_HIDDEN_INSTRUCTIONS_START;
    let end = crate::provider::NATIVE_TUI_HIDDEN_INSTRUCTIONS_END;
    let Some(start_index) = prompt.find(start) else {
        return String::new();
    };
    let after_start = start_index + start.len();
    let Some(end_index) = prompt[after_start..]
        .find(end)
        .map(|index| after_start + index)
    else {
        return prompt[after_start..].trim().to_string();
    };
    prompt[after_start..end_index].trim().to_string()
}

fn redact_native_hidden_instructions(prompt: &str) -> String {
    let start = crate::provider::NATIVE_TUI_HIDDEN_INSTRUCTIONS_START;
    let end = crate::provider::NATIVE_TUI_HIDDEN_INSTRUCTIONS_END;
    let Some(start_index) = prompt.find(start) else {
        return prompt.to_string();
    };
    let after_start = start_index + start.len();
    let Some(end_index) = prompt[after_start..]
        .find(end)
        .map(|index| after_start + index + end.len())
    else {
        return prompt[..start_index].to_string();
    };
    let mut redacted = String::new();
    redacted.push_str(&prompt[..start_index]);
    redacted.push_str(&prompt[end_index..]);
    redacted.replace("\n\n\n", "\n\n")
}

pub(crate) struct ProviderOutputClaudeNativeBridge<'a> {
    app: &'a mut DaemonApp,
}

impl<'a> ProviderOutputClaudeNativeBridge<'a> {
    pub(crate) fn new(app: &'a mut DaemonApp) -> Self {
        Self { app }
    }

    pub(crate) fn process(
        &mut self,
        session_id: &str,
        provider_run_id: &str,
        provider_run: &RuntimeProviderRun,
    ) -> Result<(), DaemonError> {
        let Some(agent_id) = provider_run.agent_instance_id().map(str::to_string) else {
            return Ok(());
        };
        let Some(events_file) = provider_run.pty_env().get("ARROBA_CLAUDE_NATIVE_EVENTS") else {
            return Ok(());
        };
        let Some(context_file) = provider_run.pty_env().get("ARROBA_CLAUDE_NATIVE_CONTEXT") else {
            return Ok(());
        };

        self.inject_pending_prompt(session_id, provider_run_id, &agent_id, context_file)?;

        let events_path = std::path::Path::new(events_file);
        let raw = fs::read_to_string(events_path).unwrap_or_default();
        if raw.trim().is_empty() {
            return Ok(());
        }
        let _ = fs::write(events_path, "");
        let attachment_id = self
            .app
            .attachments
            .list_session_attachment_ids(session_id)
            .into_iter()
            .next();

        for line in raw.lines().filter(|line| !line.trim().is_empty()) {
            let Ok(event) = serde_json::from_str::<Value>(line) else {
                continue;
            };
            let event_name = event
                .get("hook_event_name")
                .and_then(Value::as_str)
                .unwrap_or_default();
            if event_name == "UserPromptSubmit" {
                let Some(prompt) = event
                    .get("prompt")
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|prompt| !prompt.is_empty())
                else {
                    continue;
                };
                let active_prompt_id = self
                    .app
                    .prompt_owner_active_prompt_for_agent(session_id, &agent_id)?
                    .map(|prompt| prompt.id().to_string());
                let marker = claude_native_marker(context_file);
                if active_prompt_id
                    .as_deref()
                    .is_some_and(|id| marker.as_deref() == Some(&format!("injected:{id}")))
                {
                    continue;
                }
                let Some(attachment_id) = attachment_id.as_deref() else {
                    continue;
                };
                let outcome = self.app.record_native_prompt_started(
                    session_id,
                    attachment_id,
                    &agent_id,
                    prompt,
                )?;
                if let crate::session::PromptSubmissionOutcome::Started { prompt } = outcome {
                    write_claude_native_marker(context_file, &format!("native:{}", prompt.id()));
                }
            } else if matches!(event_name, "Stop" | "StopFailure" | "SessionEnd") {
                let _ = fs::write(context_file, "");
                write_claude_native_marker(context_file, "");
                let _ =
                    self.app
                        .complete_active_prompt(session_id, &agent_id, Some(provider_run_id));
            }
        }
        Ok(())
    }

    fn inject_pending_prompt(
        &mut self,
        session_id: &str,
        provider_run_id: &str,
        agent_id: &str,
        context_file: &str,
    ) -> Result<(), DaemonError> {
        let Some(prompt) = self
            .app
            .prompt_owner_active_prompt_for_agent(session_id, agent_id)?
        else {
            return Ok(());
        };
        let marker = claude_native_marker(context_file);
        if marker.as_deref() == Some(&format!("typed:{}", prompt.id())) {
            self.app
                .write_provider_pty_input_for_runtime(provider_run_id, b"\r")?;
            write_claude_native_marker(context_file, &format!("injected:{}", prompt.id()));
            return Ok(());
        }
        if marker
            .as_deref()
            .is_some_and(|value| value.ends_with(prompt.id()))
        {
            return Ok(());
        }
        let hidden = extract_native_hidden_instructions(prompt.prompt());
        let _ = fs::write(context_file, hidden);
        let visible = redact_native_hidden_instructions(prompt.prompt())
            .trim()
            .to_string();
        if !visible.is_empty() {
            self.app
                .write_provider_pty_input_for_runtime(provider_run_id, visible.as_bytes())?;
        }
        write_claude_native_marker(context_file, &format!("typed:{}", prompt.id()));
        Ok(())
    }
}
