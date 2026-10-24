//! MP-08 / MP-10 / MP-11: the official hook blocks `$name` before a native `/name` retry.
use super::*;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
struct NativeCommandRetry {
    request_id: String,
    prompt_id: String,
    command: String,
    context: String,
    stage: NativeCommandRetryStage,
    #[serde(default)]
    loads: Vec<crate::provider::skill_delivery::TypedSkillLoad>,
}

#[derive(Clone, Copy, Serialize, Deserialize)]
enum NativeCommandRetryStage {
    AwaitingRedirect,
    ClearComposer { ready_at_ms: u64 },
    SubmitCommand { ready_at_ms: u64 },
}

fn retry_path(context_file: &str) -> String {
    format!("{context_file}.typed-native-command")
}
fn read_retry(context_file: &str) -> Result<Option<NativeCommandRetry>, DaemonError> {
    match fs::read(retry_path(context_file)) {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .map(Some)
            .map_err(|_| retry_error("invalid native skill retry")),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(retry_error(error.to_string())),
    }
}
fn retry_error(message: impl Into<String>) -> DaemonError {
    DaemonError::LocalTransport {
        operation: "claude_typed_native_skill",
        message: message.into(),
    }
}
fn write_retry(context_file: &str, retry: &NativeCommandRetry) -> Result<(), DaemonError> {
    fs::write(
        retry_path(context_file),
        serde_json::to_vec(retry).expect("native retry is serializable"),
    )
    .map_err(|error| retry_error(error.to_string()))
}

impl ProviderOutputClaudeNativeBridge<'_> {
    pub(super) fn stage_typed_native_command(
        &mut self,
        context_file: &str,
        request_id: &str,
        prompt: &crate::session::PromptQueueItem,
        command: &str,
        context: &str,
        loads: &[crate::provider::skill_delivery::TypedSkillLoad],
    ) -> Result<(), DaemonError> {
        write_retry(
            context_file,
            &NativeCommandRetry {
                request_id: request_id.into(),
                prompt_id: prompt.id().into(),
                command: command.into(),
                context: context.into(),
                stage: NativeCommandRetryStage::AwaitingRedirect,
                loads: loads.to_vec(),
            },
        )?;
        permission::write_claude_hook_context_rejection(
            context_file,
            request_id,
            "Chariox is submitting the selected native skill command.",
            true,
        )
    }

    pub(super) fn accept_typed_native_redirect(
        &mut self,
        session_id: &str,
        agent_id: &str,
        run: &RuntimeProviderRun,
        context_file: &str,
        event: &Value,
    ) -> Result<(), DaemonError> {
        let Some(mut retry) = read_retry(context_file)? else {
            return Ok(());
        };
        let active = self
            .app
            .prompt_owner_active_prompt_for_agent(session_id, agent_id)?;
        if event.get("hook_context_request_id").and_then(Value::as_str)
            != Some(retry.request_id.as_str())
            || active.as_ref().map(|prompt| prompt.id()) != Some(retry.prompt_id.as_str())
            || !matches!(retry.stage, NativeCommandRetryStage::AwaitingRedirect)
        {
            return Ok(());
        }
        if let Some(session) = event.get("session_id").and_then(Value::as_str) {
            let current = self.app.providers.get_run(run.id())?;
            if current.provider_session_id().or_else(|| {
                current
                    .resume_state()
                    .provider_session_id(current.adapter_key())
            }) != Some(session)
            {
                return Ok(());
            }
        }
        // Let the rejected hook return to Claude's composer before writing its replacement.
        retry.stage = NativeCommandRetryStage::ClearComposer {
            ready_at_ms: unix_epoch_ms() + CLAUDE_SUBMIT_DELAY_MS,
        };
        write_retry(context_file, &retry)?;
        self.app
            .providers
            .output_ready_signal()
            .record_after(run.id(), CLAUDE_SUBMIT_DELAY_MS);
        Ok(())
    }

    pub(super) fn failed_typed_native_command(
        &mut self,
        session_id: &str,
        agent_id: &str,
        context_file: &str,
        event: &Value,
    ) -> Result<Option<String>, DaemonError> {
        let Some(retry) = read_retry(context_file)? else {
            return Ok(None);
        };
        let active = self
            .app
            .prompt_owner_active_prompt_for_agent(session_id, agent_id)?;
        if event.get("hook_context_request_id").and_then(Value::as_str)
            != Some(retry.request_id.as_str())
            || active.as_ref().map(|prompt| prompt.id()) != Some(retry.prompt_id.as_str())
        {
            return Ok(None);
        }
        fs::remove_file(retry_path(context_file))
            .map_err(|error| retry_error(error.to_string()))?;
        Ok(Some(retry.prompt_id))
    }

    pub(super) fn resume_typed_native_command(
        &mut self,
        session_id: &str,
        agent_id: &str,
        run: &RuntimeProviderRun,
        context_file: &str,
    ) -> Result<(), DaemonError> {
        let Some(mut retry) = read_retry(context_file)? else {
            return Ok(());
        };
        let active = self
            .app
            .prompt_owner_active_prompt_for_agent(session_id, agent_id)?;
        let Some(prompt) = active.filter(|prompt| {
            prompt.id() == retry.prompt_id
                && prompt.status() != crate::session::PromptStatus::Cancelling
        }) else {
            fs::remove_file(retry_path(context_file))
                .map_err(|error| retry_error(error.to_string()))?;
            return Ok(());
        };
        let ready_at_ms = match retry.stage {
            NativeCommandRetryStage::AwaitingRedirect => return Ok(()),
            NativeCommandRetryStage::ClearComposer { ready_at_ms }
            | NativeCommandRetryStage::SubmitCommand { ready_at_ms } => ready_at_ms,
        };
        let now = unix_epoch_ms();
        if now < ready_at_ms {
            self.app
                .providers
                .output_ready_signal()
                .record_after(run.id(), ready_at_ms - now);
            return Ok(());
        }
        if matches!(retry.stage, NativeCommandRetryStage::ClearComposer { .. }) {
            // Ctrl-U clears only the current visual line in Claude's multiline
            // composer. Ctrl-C clears the idle draft; give that input its own
            // render cycle before writing the replacement command.
            self.app
                .write_provider_pty_input_for_runtime(run.id(), b"\x03")?;
            retry.stage = NativeCommandRetryStage::SubmitCommand {
                ready_at_ms: now + CLAUDE_SUBMIT_DELAY_MS,
            };
            write_retry(context_file, &retry)?;
            self.app
                .providers
                .output_ready_signal()
                .record_after(run.id(), CLAUDE_SUBMIT_DELAY_MS);
            return Ok(());
        }
        hidden_context::stage_managed_inputs(
            context_file,
            &ClaudeNativePromptInjection {
                id: prompt.id(),
                origin_prompt_id: prompt.id(),
                prompt: prompt.prompt(),
                hidden_system_context: "",
                attachments: prompt.attachments(),
                context_handoff: &|_| String::new(),
            },
            &retry.context,
        )?;
        fs::write(
            format!("{context_file}.typed-native-loads"),
            serde_json::to_vec(&(prompt.id(), &retry.loads))
                .expect("native receipt pins are serializable"),
        )
        .map_err(|error| retry_error(error.to_string()))?;
        fs::write(context_file, &retry.context).map_err(|error| retry_error(error.to_string()))?;
        fs::write(format!("{context_file}.approval-origin"), prompt.id())
            .map_err(|error| retry_error(error.to_string()))?;
        let input = if run.provider() == "claude-headless" {
            normalize_claude_visible_prompt_for_headless(&retry.command)
        } else {
            retry.command
        };
        self.app
            .write_provider_pty_input_for_runtime(run.id(), input.as_bytes())?;
        write_claude_headless_submit_retry(context_file, prompt.id(), 0, now, &input);
        write_claude_native_marker(context_file, &format!("submit-wait:{}:{now}", prompt.id()));
        fs::remove_file(retry_path(context_file))
            .map_err(|error| retry_error(error.to_string()))?;
        self.app
            .providers
            .output_ready_signal()
            .record_after(run.id(), CLAUDE_SUBMIT_DELAY_MS);
        Ok(())
    }
}

/// A native retry's receipt belongs to its original turn, never a later managed prompt.
/// `Some` marks that turn as the typed redirect, even when the command pinned no skill.
pub(super) fn typed_native_redirect(
    context_file: &str,
    prompt_id: &str,
) -> Result<Option<Vec<crate::provider::skill_delivery::TypedSkillLoad>>, DaemonError> {
    let bytes = match fs::read(format!("{context_file}.typed-native-loads")) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(retry_error(error.to_string())),
    };
    let (owner, loads): (String, Vec<crate::provider::skill_delivery::TypedSkillLoad>) =
        serde_json::from_slice(&bytes)
            .map_err(|_| retry_error("invalid typed native receipt pins"))?;
    Ok((owner == prompt_id).then_some(loads))
}
