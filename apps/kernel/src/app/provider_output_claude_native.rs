use std::fs;
use std::path::PathBuf;

use serde_json::Value;

use super::provider_output_fanout::ProviderOutputFanout;
use crate::app::DaemonApp;
use crate::app::KernelPromptDispatch;
use crate::error::DaemonError;
use crate::provider::{
    ProviderNativeInteractionBridge, ProviderPromptSignalBatch, ProviderResumeState,
    RuntimeProviderRun,
};
use crate::session::{
    unix_epoch_ms, PromptAttachment, RuntimeInteraction, RuntimeInteractionChoice,
    RuntimeInteractionChoiceStyle, RuntimeInteractionKind, RuntimeInteractionLevel,
};
use crate::terminal::TerminalOutputKind;

mod attachments;
mod permission;
#[cfg(test)]
mod tests;
mod transcript;

use attachments::{
    extract_claude_native_prompt_attachments, format_claude_attachment_context,
    format_claude_native_attachment_prompt_suffix, join_claude_context,
};
use permission::{
    append_claude_headless_debug, claude_headless_bypass_confirmation_visible,
    claude_headless_composer_visible, claude_headless_prompt_waiting_in_composer,
    claude_headless_workspace_trust_visible, claude_native_marker, claude_permission_recent_file,
    claude_rendered_permission_visible, clear_claude_permission_recent,
    extract_native_hidden_instructions, format_claude_permission_message,
    normalize_claude_visible_prompt_for_headless, read_claude_headless_submit_retry,
    redact_native_hidden_instructions, should_bridge_claude_permission,
    take_claude_permission_inputs, timestamp_millis, update_claude_permission_recent,
    write_claude_headless_startup_wait_marker, write_claude_headless_submit_retry,
    write_claude_hook_context_response, write_claude_native_marker, write_claude_permission_input,
    write_claude_permission_response,
};
use transcript::{
    drain_claude_transcript_file, known_claude_transcript_paths, load_claude_transcript_cursor,
    save_claude_transcript_cursor,
};

const CLAUDE_ATTACHMENT_CONTEXT_BYTES: usize = 64 * 1024;

/// Delay between writing a prompt's visible text into the provider PTY and
/// sending the Enter keystroke, giving the terminal time to register the
/// (possibly multi-line, bracket-pasted) text before it is submitted. This
/// wait is taken between short app-lock holds by the async dispatch retry
/// loop rather than by sleeping inside the lock.
const CLAUDE_SUBMIT_DELAY_MS: u64 = 250;

struct ClaudeNativePromptInjection<'a> {
    id: &'a str,
    prompt: &'a str,
    hidden_system_context: &'a str,
    attachments: &'a [PromptAttachment],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ClaudeNativeDispatchAttempt {
    Completed,
    AwaitingInjection,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct ClaudeNativeProcessOutcome {
    /// A claude-headless run reported Stop/SessionEnd this pass. The caller
    /// should drain its transcripts once more after a short delay taken off
    /// the app lock, to capture the final assistant flush without blocking
    /// the whole daemon inside `process`.
    pub(crate) needs_deferred_headless_drain: bool,
}

pub(crate) struct ProviderOutputClaudeNativeBridge<'a> {
    app: &'a mut DaemonApp,
}

fn claude_native_history_source_attachment_id(
    app: &DaemonApp,
    session_id: &str,
    provider_run_id: &str,
    fallback_attachment_id: &str,
) -> String {
    app.terminal()
        .input_records()
        .into_iter()
        .rev()
        .find(|record| record.session_id == session_id && record.provider_run_id == provider_run_id)
        .map(|record| record.source_attachment_id)
        .unwrap_or_else(|| fallback_attachment_id.to_string())
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
        native_interaction_bridge: Option<std::sync::Arc<dyn ProviderNativeInteractionBridge>>,
    ) -> Result<ClaudeNativeProcessOutcome, DaemonError> {
        let mut outcome = ClaudeNativeProcessOutcome::default();
        let Some(agent_id) = provider_run.agent_instance_id().map(str::to_string) else {
            return Ok(outcome);
        };
        let Some(events_file) = provider_run.pty_env().get("ARROBA_CLAUDE_NATIVE_EVENTS") else {
            return Ok(outcome);
        };
        let Some(context_file) = provider_run.pty_env().get("ARROBA_CLAUDE_NATIVE_CONTEXT") else {
            return Ok(outcome);
        };

        for input in take_claude_permission_inputs(context_file) {
            self.app
                .write_provider_pty_input_for_runtime(provider_run_id, &input)?;
            write_claude_native_marker(context_file, "");
        }
        self.inject_pending_prompt(
            session_id,
            provider_run_id,
            &agent_id,
            context_file,
            provider_run,
        )?;
        if provider_run.provider() == "claude-headless" {
            self.drain_known_headless_transcripts(session_id, provider_run_id, context_file)?;
        }

        let events_path = std::path::Path::new(events_file);
        let raw = fs::read_to_string(events_path).unwrap_or_default();
        if raw.trim().is_empty() {
            return Ok(outcome);
        }
        let _ = fs::write(events_path, "");
        let runtime_attachment_id = self
            .app
            .attachments
            .list_session_attachment_ids(session_id)
            .into_iter()
            .next();

        for line in raw.lines().filter(|line| !line.trim().is_empty()) {
            let Ok(event) = serde_json::from_str::<Value>(line) else {
                continue;
            };
            if provider_run.provider() == "claude-headless" {
                if let Some(transcript_path) = event
                    .get("transcript_path")
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                {
                    self.drain_headless_transcript(
                        session_id,
                        provider_run_id,
                        context_file,
                        transcript_path,
                    )?;
                }
            }
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
                if let Some(request_id) =
                    event.get("hook_context_request_id").and_then(Value::as_str)
                {
                    let context =
                        self.claude_native_prompt_context(session_id, &agent_id, prompt)?;
                    write_claude_hook_context_response(context_file, request_id, &context);
                }
                let Some(runtime_attachment_id) = runtime_attachment_id.as_deref() else {
                    continue;
                };
                let history_source_attachment_id = claude_native_history_source_attachment_id(
                    self.app,
                    session_id,
                    provider_run_id,
                    runtime_attachment_id,
                );
                let attachments = extract_claude_native_prompt_attachments(
                    prompt,
                    provider_run.working_directory().map(PathBuf::as_path),
                );
                let outcome = self.app.record_native_prompt_started_with_attachments(
                    session_id,
                    runtime_attachment_id,
                    &history_source_attachment_id,
                    &agent_id,
                    prompt,
                    attachments,
                )?;
                if let crate::session::PromptSubmissionOutcome::Started { prompt } = outcome {
                    write_claude_native_marker(context_file, &format!("native:{}", prompt.id()));
                }
            } else if matches!(event_name, "Stop" | "StopFailure" | "SessionEnd") {
                if provider_run.provider() == "claude-headless" {
                    // Drain whatever the transcript holds now; the final flush
                    // can land shortly after Stop, so ask the caller to drain
                    // again after a brief delay taken off the app lock rather
                    // than sleeping here and stalling the daemon.
                    self.drain_known_headless_transcripts(
                        session_id,
                        provider_run_id,
                        context_file,
                    )?;
                    outcome.needs_deferred_headless_drain = true;
                }
                let _ = fs::write(context_file, "");
                write_claude_native_marker(context_file, "");
                let _ =
                    self.app
                        .complete_active_prompt(session_id, &agent_id, Some(provider_run_id));
                if provider_run.provider() == "claude-headless" {
                    if let Some(next_prompt) = self
                        .app
                        .prompt_owner_active_prompt_for_agent(session_id, &agent_id)?
                    {
                        crate::logging::debug_with_fields(
                            "daemon.claude_headless",
                            "marked post-stop queued prompt ready",
                            serde_json::json!({
                                "session_id": session_id,
                                "provider_run_id": provider_run_id,
                                "agent_id": agent_id,
                                "prompt_id": next_prompt.id(),
                            }),
                        );
                        write_claude_native_marker(
                            context_file,
                            &format!("post-stop-ready:{}", next_prompt.id()),
                        );
                    }
                }
            } else if matches!(event_name, "PreToolUse" | "PermissionRequest") {
                self.resolve_permission_event(
                    session_id,
                    provider_run_id,
                    &agent_id,
                    context_file,
                    native_interaction_bridge.clone(),
                    &event,
                )?;
            }
        }
        if provider_run.provider() == "claude-headless" {
            self.drain_known_headless_transcripts(session_id, provider_run_id, context_file)?;
        }
        Ok(outcome)
    }

    pub(crate) fn drain_headless_transcripts_for_context(
        &mut self,
        session_id: &str,
        provider_run_id: &str,
        provider_run: &RuntimeProviderRun,
    ) -> Result<(), DaemonError> {
        if provider_run.provider() != "claude-headless" {
            return Ok(());
        }
        let Some(context_file) = provider_run.pty_env().get("ARROBA_CLAUDE_NATIVE_CONTEXT") else {
            return Ok(());
        };
        self.drain_known_headless_transcripts(session_id, provider_run_id, context_file)
    }

    fn drain_known_headless_transcripts(
        &mut self,
        session_id: &str,
        provider_run_id: &str,
        context_file: &str,
    ) -> Result<(), DaemonError> {
        let paths = known_claude_transcript_paths(context_file);
        for path in paths {
            self.drain_headless_transcript(session_id, provider_run_id, context_file, &path)?;
        }
        Ok(())
    }

    fn drain_headless_transcript(
        &mut self,
        session_id: &str,
        provider_run_id: &str,
        context_file: &str,
        transcript_path: &str,
    ) -> Result<(), DaemonError> {
        let mut cursor = load_claude_transcript_cursor(context_file);
        let drain = drain_claude_transcript_file(transcript_path, &mut cursor);
        save_claude_transcript_cursor(context_file, &cursor);
        if drain.chunks.is_empty()
            && drain.assistant_message_ids.is_empty()
            && drain.session_id.is_none()
            && drain.model.is_none()
        {
            return Ok(());
        }

        let mut metadata = ProviderPromptSignalBatch::default();
        if let Some(session_id) = drain.session_id {
            metadata.resolved_resume_state =
                Some(ProviderResumeState::from_claude_session_id(session_id));
        }
        if let Some(model) = drain.model {
            metadata.resolved_model = Some(model);
            metadata.resolved_model_source = Some("claude.headless.transcript");
        }
        if metadata.resolved_resume_state.is_some() || metadata.resolved_model.is_some() {
            self.app
                .providers
                .apply_structured_output_metadata(provider_run_id, &metadata)?;
            if let Ok(run) = self.app.providers.get_run(provider_run_id) {
                self.app.update_provider_run_projection(run);
            }
        }

        let recipient_attachment_ids = self.app.attachments.list_session_attachment_ids(session_id);
        let fanout = ProviderOutputFanout::new(self.app);
        let mut saw_response_content = false;
        let mut saw_runtime_activity = false;
        for chunk in drain.chunks {
            if chunk.text.is_empty() {
                continue;
            }
            if matches!(
                chunk.kind,
                TerminalOutputKind::ProviderOutput | TerminalOutputKind::ProviderReasoning
            ) {
                saw_response_content = true;
            }
            if matches!(
                chunk.kind,
                TerminalOutputKind::ProviderOutput
                    | TerminalOutputKind::ProviderReasoning
                    | TerminalOutputKind::ProviderTool
                    | TerminalOutputKind::ProviderStatus
            ) {
                saw_runtime_activity = true;
            }
            fanout.fan_out(
                session_id,
                provider_run_id,
                chunk.kind,
                Some(format!(
                    "claude-headless:{provider_run_id}:{}",
                    chunk.merge_key_suffix
                )),
                recipient_attachment_ids.clone(),
                chunk.text.as_bytes(),
            );
        }
        if saw_response_content {
            crate::transport::flow_control::note_prompt_response_content(self.app, provider_run_id);
        } else if saw_runtime_activity {
            crate::transport::flow_control::note_prompt_output(self.app, provider_run_id);
        }
        for message_id in drain.assistant_message_ids {
            ProviderOutputFanout::new(self.app).record_assistant_message_completion(
                session_id,
                provider_run_id,
                recipient_attachment_ids.clone(),
                &message_id,
                unix_epoch_ms(),
            );
            crate::transport::flow_control::mark_prompt_completion_recorded(
                self.app,
                provider_run_id,
            );
        }
        Ok(())
    }

    pub(crate) fn process_terminal_output(
        &mut self,
        session_id: &str,
        provider_run_id: &str,
        provider_run: &RuntimeProviderRun,
        native_interaction_bridge: Option<std::sync::Arc<dyn ProviderNativeInteractionBridge>>,
        rendered: &str,
    ) -> Result<(), DaemonError> {
        let Some(bridge) = native_interaction_bridge else {
            return Ok(());
        };
        let Some(agent_id) = provider_run.agent_instance_id().map(str::to_string) else {
            return Ok(());
        };
        let Some(context_file) = provider_run.pty_env().get("ARROBA_CLAUDE_NATIVE_CONTEXT") else {
            return Ok(());
        };
        let visible = claude_rendered_permission_visible(rendered);
        if provider_run.provider() == "claude-headless" && !rendered.is_empty() {
            append_claude_headless_debug(context_file, "pty", rendered);
            self.drain_known_headless_transcripts(session_id, provider_run_id, context_file)?;
        }
        if provider_run.provider() == "claude-headless" {
            let recent = update_claude_permission_recent(context_file, rendered);
            if claude_headless_workspace_trust_visible(&recent) {
                append_claude_headless_debug(context_file, "auto_confirm", "workspace_trust");
                self.app
                    .write_provider_pty_input_for_runtime(provider_run_id, b"\r")?;
                write_claude_headless_startup_wait_marker(context_file);
                clear_claude_permission_recent(context_file);
                return Ok(());
            }
            if claude_headless_bypass_confirmation_visible(&recent) {
                append_claude_headless_debug(context_file, "auto_confirm", "bypass_permissions");
                self.app
                    .write_provider_pty_input_for_runtime(provider_run_id, b"\x1b[B\r")?;
                write_claude_headless_startup_wait_marker(context_file);
                clear_claude_permission_recent(context_file);
                return Ok(());
            }
        }
        let recent = if visible {
            rendered.to_string()
        } else {
            update_claude_permission_recent(context_file, rendered)
        };
        if !visible && !claude_rendered_permission_visible(&recent) {
            return Ok(());
        }
        if claude_native_marker(context_file)
            .as_deref()
            .is_some_and(|value| value.starts_with("permission:"))
        {
            return Ok(());
        }
        let interaction_id = format!(
            "claude-rendered-permission-{provider_run_id}-{}",
            timestamp_millis()
        );
        write_claude_native_marker(context_file, &format!("permission:{interaction_id}"));
        clear_claude_permission_recent(context_file);
        let interaction = RuntimeInteraction::new(
            interaction_id.clone(),
            agent_id,
            RuntimeInteractionKind::Permission,
            RuntimeInteractionLevel::Warning,
            Some("Approve Claude Code Bash?".to_string()),
            "Claude Code is showing a native Bash permission prompt.",
            vec![
                RuntimeInteractionChoice::new(
                    "allow_once",
                    "Allow once",
                    "allow",
                    Some(RuntimeInteractionChoiceStyle::Primary),
                ),
                RuntimeInteractionChoice::new(
                    "deny",
                    "Deny",
                    "deny",
                    Some(RuntimeInteractionChoiceStyle::Danger),
                ),
            ],
            None,
            Some(300),
            Some("deny".to_string()),
        );
        let session_id = session_id.to_string();
        let context_file = context_file.to_string();
        std::thread::spawn(move || {
            let input = match bridge.request_blocking(&session_id, interaction) {
                Ok(resolution)
                    if resolution.reply.as_deref() == Some("allow")
                        || resolution.choice_id.as_deref() == Some("allow_once") =>
                {
                    b"\r".to_vec()
                }
                Ok(_) => vec![0x03],
                Err(error) => {
                    crate::logging::warn_with_fields(
                        "daemon.provider_output",
                        "Claude rendered permission bridge failed",
                        serde_json::json!({
                            "session_id": session_id,
                            "interaction_id": interaction_id,
                            "error": error.to_string(),
                        }),
                    );
                    vec![0x03]
                }
            };
            write_claude_permission_input(&context_file, &interaction_id, &input);
        });
        Ok(())
    }

    fn resolve_permission_event(
        &mut self,
        session_id: &str,
        provider_run_id: &str,
        agent_id: &str,
        context_file: &str,
        native_interaction_bridge: Option<std::sync::Arc<dyn ProviderNativeInteractionBridge>>,
        event: &Value,
    ) -> Result<(), DaemonError> {
        let Some(bridge) = native_interaction_bridge else {
            return Ok(());
        };
        if !should_bridge_claude_permission(event) {
            return Ok(());
        }
        let Some(request_id) = event
            .get("hook_context_request_id")
            .and_then(Value::as_str)
            .filter(|value| !value.trim().is_empty())
        else {
            return Ok(());
        };
        let tool_name = event
            .get("tool_name")
            .and_then(Value::as_str)
            .unwrap_or("tool");
        let interaction = RuntimeInteraction::new(
            format!("claude-native-permission-{provider_run_id}-{request_id}"),
            agent_id.to_string(),
            RuntimeInteractionKind::Permission,
            RuntimeInteractionLevel::Warning,
            Some(format!("Approve Claude Code {tool_name}?")),
            format_claude_permission_message(event),
            vec![
                RuntimeInteractionChoice::new(
                    "allow_once",
                    "Allow once",
                    "allow",
                    Some(RuntimeInteractionChoiceStyle::Primary),
                ),
                RuntimeInteractionChoice::new(
                    "deny",
                    "Deny",
                    "deny",
                    Some(RuntimeInteractionChoiceStyle::Danger),
                ),
            ],
            None,
            Some(300),
            Some("deny".to_string()),
        );
        let session_id = session_id.to_string();
        let context_file = context_file.to_string();
        let request_id = request_id.to_string();
        std::thread::spawn(
            move || match bridge.request_blocking(&session_id, interaction) {
                Ok(resolution) => {
                    let allowed = resolution.reply.as_deref() == Some("allow")
                        || resolution.choice_id.as_deref() == Some("allow_once");
                    write_claude_permission_response(
                        &context_file,
                        &request_id,
                        allowed,
                        if allowed {
                            "Approved through Arroba."
                        } else if resolution.status == "timed_out" {
                            "Timed out waiting for Arroba approval."
                        } else {
                            "Denied through Arroba."
                        },
                    );
                }
                Err(error) => {
                    crate::logging::warn_with_fields(
                        "daemon.provider_output",
                        "Claude native permission bridge failed",
                        serde_json::json!({
                            "session_id": session_id,
                            "request_id": request_id,
                            "error": error.to_string(),
                        }),
                    );
                    write_claude_permission_response(
                        &context_file,
                        &request_id,
                        false,
                        "Arroba permission bridge failed.",
                    );
                }
            },
        );
        Ok(())
    }

    /// One injection attempt for a prompt dispatch. Claude-headless confirms
    /// injection asynchronously through the context-file marker, so the caller
    /// retries `AwaitingInjection` outcomes off the app lock instead of this
    /// method sleeping while the whole daemon is blocked.
    pub(crate) fn process_prompt_dispatch_attempt(
        &mut self,
        session_id: &str,
        provider_run_id: &str,
        provider_run: &RuntimeProviderRun,
        dispatch: &KernelPromptDispatch,
    ) -> Result<ClaudeNativeDispatchAttempt, DaemonError> {
        let Some(agent_id) = provider_run.agent_instance_id().map(str::to_string) else {
            return Ok(ClaudeNativeDispatchAttempt::Completed);
        };
        let Some(context_file) = provider_run.pty_env().get("ARROBA_CLAUDE_NATIVE_CONTEXT") else {
            return Ok(ClaudeNativeDispatchAttempt::Completed);
        };
        let prompt = ClaudeNativePromptInjection {
            id: &dispatch.prompt_id,
            prompt: &dispatch.prompt,
            hidden_system_context: &dispatch.hidden_system_context,
            attachments: &dispatch.attachments,
        };
        self.inject_prompt(
            session_id,
            provider_run_id,
            &agent_id,
            context_file,
            provider_run,
            &prompt,
        )?;
        // Injection completes once the Enter keystroke has been submitted and
        // the marker reads `injected`. Both TUI and headless runs now defer
        // that keystroke via `submit-wait`, so the async caller retries off
        // the app lock until the PTY-settle delay elapses.
        if claude_native_marker(context_file).as_deref() == Some(&format!("injected:{}", prompt.id))
        {
            return Ok(ClaudeNativeDispatchAttempt::Completed);
        }
        Ok(ClaudeNativeDispatchAttempt::AwaitingInjection)
    }

    fn claude_native_prompt_context(
        &self,
        session_id: &str,
        agent_id: &str,
        prompt: &str,
    ) -> Result<String, DaemonError> {
        let agent = self.app.agents.get_agent(agent_id)?;
        let session = self.app.sessions.get_session(session_id)?;
        let skill_grants = agent.skill_grants();
        crate::skill::format_granted_skill_prompt_context(
            agent.agent_ref(),
            &skill_grants,
            session.workspace_id(),
            prompt,
        )
    }

    fn inject_pending_prompt(
        &mut self,
        session_id: &str,
        provider_run_id: &str,
        agent_id: &str,
        context_file: &str,
        provider_run: &RuntimeProviderRun,
    ) -> Result<(), DaemonError> {
        let Some(prompt) = self
            .app
            .prompt_owner_active_prompt_for_agent(session_id, agent_id)?
        else {
            return Ok(());
        };
        let prompt = ClaudeNativePromptInjection {
            id: prompt.id(),
            prompt: prompt.prompt(),
            hidden_system_context: prompt.hidden_system_context(),
            attachments: prompt.attachments(),
        };
        self.inject_prompt(
            session_id,
            provider_run_id,
            agent_id,
            context_file,
            provider_run,
            &prompt,
        )
    }

    fn inject_prompt(
        &mut self,
        session_id: &str,
        provider_run_id: &str,
        agent_id: &str,
        context_file: &str,
        provider_run: &RuntimeProviderRun,
        prompt: &ClaudeNativePromptInjection<'_>,
    ) -> Result<(), DaemonError> {
        let mut marker = claude_native_marker(context_file);
        let force_post_stop_ready = provider_run.provider() == "claude-headless"
            && marker
                .as_deref()
                .is_some_and(|value| value == format!("post-stop-ready:{}", prompt.id));
        if force_post_stop_ready {
            crate::logging::debug_with_fields(
                "daemon.claude_headless",
                "forcing post-stop queued prompt injection",
                serde_json::json!({
                    "session_id": session_id,
                    "provider_run_id": provider_run_id,
                    "agent_id": agent_id,
                    "prompt_id": prompt.id,
                }),
            );
            write_claude_native_marker(context_file, "");
            marker = None;
        }
        // A prior injection wrote the visible text and marked `submit-wait`;
        // submit the Enter keystroke once the PTY-settle delay has elapsed.
        // The wait itself happens off the app lock: the async dispatch retry
        // loop (and the output pump for `process`) revisit this until the
        // delay passes, so the daemon is never blocked mid-injection.
        match submit_wait_state(marker.as_deref(), prompt.id, unix_epoch_ms()) {
            SubmitWaitState::Waiting => {
                append_claude_headless_debug(context_file, "submit_wait", prompt.id);
                return Ok(());
            }
            SubmitWaitState::ReadyToSubmit => {
                append_claude_headless_debug(context_file, "submit_enter", prompt.id);
                self.app
                    .write_provider_pty_input_for_runtime(provider_run_id, b"\r")?;
                write_claude_native_marker(context_file, &format!("injected:{}", prompt.id));
                if provider_run.provider() == "claude-headless" {
                    write_claude_headless_submit_retry(context_file, prompt.id, 0, unix_epoch_ms());
                }
                return Ok(());
            }
            SubmitWaitState::NotSubmitWait => {}
        }
        let prompt_typed_for_headless = provider_run.provider() == "claude-headless"
            && marker.as_deref() == Some(&format!("typed:{}", prompt.id));
        if let Some(started_at_ms) = marker
            .as_deref()
            .and_then(|value| value.strip_prefix("startup-wait:"))
            .and_then(|value| value.parse::<u64>().ok())
        {
            if unix_epoch_ms().saturating_sub(started_at_ms) < 2_500 {
                append_claude_headless_debug(context_file, "startup_wait", prompt.id);
                return Ok(());
            }
            write_claude_native_marker(context_file, "");
            marker = None;
        }
        if provider_run.provider() == "claude-headless"
            && marker.is_none()
            && unix_epoch_ms().saturating_sub(provider_run.started_at_ms()) < 4_000
        {
            append_claude_headless_debug(context_file, "inject_wait", prompt.id);
            return Ok(());
        }
        if provider_run.provider() == "claude-headless" {
            let recent = claude_permission_recent_file(context_file)
                .and_then(|path| fs::read_to_string(path).ok())
                .unwrap_or_default();
            if claude_headless_workspace_trust_visible(&recent) {
                append_claude_headless_debug(
                    context_file,
                    "inject_auto_confirm",
                    "workspace_trust",
                );
                self.app
                    .write_provider_pty_input_for_runtime(provider_run_id, b"\r")?;
                write_claude_headless_startup_wait_marker(context_file);
                clear_claude_permission_recent(context_file);
                return Ok(());
            }
            if claude_headless_bypass_confirmation_visible(&recent) {
                append_claude_headless_debug(
                    context_file,
                    "inject_auto_confirm",
                    "bypass_permissions",
                );
                self.app
                    .write_provider_pty_input_for_runtime(provider_run_id, b"\x1b[B\r")?;
                write_claude_headless_startup_wait_marker(context_file);
                clear_claude_permission_recent(context_file);
                return Ok(());
            }
            if !force_post_stop_ready
                && !prompt_typed_for_headless
                && !claude_headless_composer_visible(&recent)
                && unix_epoch_ms().saturating_sub(provider_run.started_at_ms()) < 4_000
            {
                append_claude_headless_debug(context_file, "inject_wait_composer", prompt.id);
                return Ok(());
            }
        }
        if marker.as_deref() == Some(&format!("typed:{}", prompt.id)) {
            append_claude_headless_debug(context_file, "inject_enter", prompt.id);
            self.app
                .write_provider_pty_input_for_runtime(provider_run_id, b"\r")?;
            write_claude_native_marker(context_file, &format!("injected:{}", prompt.id));
            if provider_run.provider() == "claude-headless" {
                write_claude_headless_submit_retry(context_file, prompt.id, 0, unix_epoch_ms());
            }
            return Ok(());
        }
        if provider_run.provider() == "claude-headless"
            && marker.as_deref() == Some(&format!("injected:{}", prompt.id))
        {
            let retry = read_claude_headless_submit_retry(context_file);
            let now = unix_epoch_ms();
            let recent = claude_permission_recent_file(context_file)
                .and_then(|path| fs::read_to_string(path).ok())
                .unwrap_or_default();
            let count = if retry.prompt_id == prompt.id {
                retry.count
            } else {
                0
            };
            let last_attempt_ms = if retry.prompt_id == prompt.id {
                retry.last_attempt_ms
            } else {
                0
            };
            if count < 3
                && now.saturating_sub(last_attempt_ms) >= 2_000
                && claude_headless_prompt_waiting_in_composer(
                    &recent,
                    redact_native_hidden_instructions(prompt.prompt).trim(),
                )
            {
                append_claude_headless_debug(
                    context_file,
                    "inject_enter_retry",
                    &format!("{}:{}", prompt.id, count + 1),
                );
                self.app
                    .write_provider_pty_input_for_runtime(provider_run_id, b"\r")?;
                write_claude_headless_submit_retry(context_file, prompt.id, count + 1, now);
            }
            return Ok(());
        }
        if marker
            .as_deref()
            .is_some_and(|value| value.ends_with(prompt.id))
        {
            return Ok(());
        }
        let native_attachment_suffix =
            format_claude_native_attachment_prompt_suffix(prompt.attachments, context_file);
        let visible = redact_native_hidden_instructions(prompt.prompt)
            .trim()
            .to_string();
        let native_hidden = extract_native_hidden_instructions(prompt.prompt);
        let attachment_context = format_claude_attachment_context(prompt.attachments, context_file);
        let hidden_context = if provider_run.provider() == "claude-headless" {
            let envelope = crate::prompt_assembly::PromptAssemblyService::from_env()?
                .assemble_provider_turn(
                    provider_run,
                    &visible,
                    Some(prompt.hidden_system_context),
                    prompt.attachments.to_vec(),
                    crate::prompt_assembly::PromptAssemblyMode::NormalProviderTurn,
                )?;
            let skill_context =
                self.claude_native_prompt_context(session_id, agent_id, &visible)?;
            join_claude_context([
                envelope.hidden_system_context,
                skill_context,
                native_hidden,
                attachment_context,
            ])
        } else {
            join_claude_context([native_hidden, attachment_context])
        };
        let _ = fs::write(context_file, hidden_context);
        let visible = join_claude_context([native_attachment_suffix, visible]);
        if !visible.is_empty() {
            let input = if provider_run.provider() == "claude-headless" {
                normalize_claude_visible_prompt_for_headless(&visible)
            } else {
                visible.clone()
            };
            append_claude_headless_debug(context_file, "inject_prompt", &input);
            self.app
                .write_provider_pty_input_for_runtime(provider_run_id, input.as_bytes())?;
            // Defer the Enter keystroke: mark `submit-wait` with the write
            // time so a later pass (off the app lock) submits it once the PTY
            // has had CLAUDE_SUBMIT_DELAY_MS to register the pasted text.
            write_claude_native_marker(
                context_file,
                &format!("submit-wait:{}:{}", prompt.id, unix_epoch_ms()),
            );
        } else {
            append_claude_headless_debug(context_file, "inject_empty", prompt.id);
            write_claude_native_marker(context_file, &format!("injected:{}", prompt.id));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SubmitWaitState {
    NotSubmitWait,
    Waiting,
    ReadyToSubmit,
}

/// Decide whether a deferred Enter keystroke is due for the given prompt,
/// based on a `submit-wait:{prompt_id}:{written_at_ms}` marker. An
/// unparseable timestamp submits immediately rather than stalling forever.
fn submit_wait_state(marker: Option<&str>, prompt_id: &str, now_ms: u64) -> SubmitWaitState {
    let Some(rest) = marker.and_then(|value| value.strip_prefix("submit-wait:")) else {
        return SubmitWaitState::NotSubmitWait;
    };
    let Some((marked_prompt_id, started_at)) = rest.rsplit_once(':') else {
        return SubmitWaitState::NotSubmitWait;
    };
    if marked_prompt_id != prompt_id {
        return SubmitWaitState::NotSubmitWait;
    }
    match started_at.parse::<u64>() {
        Ok(started_at_ms) if now_ms.saturating_sub(started_at_ms) < CLAUDE_SUBMIT_DELAY_MS => {
            SubmitWaitState::Waiting
        }
        _ => SubmitWaitState::ReadyToSubmit,
    }
}
