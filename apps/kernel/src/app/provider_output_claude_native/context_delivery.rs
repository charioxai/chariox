//! MP-08 / MP-10 / MP-11: acknowledge hidden context only after official hooks flush it.
use super::*;
use serde::{Deserialize, Serialize};

#[derive(Deserialize, Serialize)]
struct PendingDelivery {
    request_id: String,
    run_id: String,
    prompt_id: String,
    managed: bool,
    #[serde(default)]
    active_prompt_id: Option<String>,
}

pub(super) fn stage(
    context_file: &str,
    request_id: &str,
    run_id: &str,
    prompt_id: &str,
    active_prompt_id: &str,
    managed: bool,
) -> Result<(), DaemonError> {
    permission::validate_context_request_id(request_id)?;
    let pending = PendingDelivery {
        request_id: request_id.into(),
        run_id: run_id.into(),
        prompt_id: prompt_id.into(),
        managed,
        active_prompt_id: Some(active_prompt_id.into()),
    };
    fs::write(
        format!("{context_file}.context-delivery"),
        serde_json::to_vec(&pending).expect("delivery metadata is serializable"),
    )
    .map_err(|error| DaemonError::LocalTransport {
        operation: "claude_context_delivery",
        message: error.to_string(),
    })
}

pub(super) fn request_expired(event: &Value) -> bool {
    event
        .get("hook_context_deadline_ms")
        .and_then(Value::as_u64)
        .is_some_and(|deadline| unix_epoch_ms() >= deadline)
}

impl ProviderOutputClaudeNativeBridge<'_> {
    fn matching_context_delivery(
        &mut self,
        session_id: &str,
        agent_id: &str,
        run_id: &str,
        context_file: &str,
        event: &Value,
    ) -> Result<Option<PendingDelivery>, DaemonError> {
        let pending_path = format!("{context_file}.context-delivery");
        let Some(pending) = fs::read(&pending_path)
            .ok()
            .and_then(|text| serde_json::from_slice::<PendingDelivery>(&text).ok())
        else {
            return Ok(None);
        };
        let active = self
            .app
            .prompt_owner_active_prompt_for_agent(session_id, agent_id)?;
        if event.get("hook_context_request_id").and_then(Value::as_str)
            != Some(pending.request_id.as_str())
            || pending.run_id != run_id
            || active.as_ref().map(|prompt| prompt.id())
                != Some(
                    pending
                        .active_prompt_id
                        .as_deref()
                        .unwrap_or(&pending.prompt_id),
                )
        {
            return Ok(None);
        }
        let current = self.app.providers.get_run(run_id)?;
        if let Some(receipt_session) = event.get("session_id").and_then(Value::as_str) {
            let current_session = current.provider_session_id().or_else(|| {
                current
                    .resume_state()
                    .provider_session_id(current.adapter_key())
            });
            if current_session != Some(receipt_session) {
                return Ok(None);
            }
        }
        Ok(Some(pending))
    }

    pub(super) fn failed_context_delivery(
        &mut self,
        session_id: &str,
        agent_id: &str,
        run_id: &str,
        context_file: &str,
        event: &Value,
    ) -> Result<Option<String>, DaemonError> {
        let Some(pending) =
            self.matching_context_delivery(session_id, agent_id, run_id, context_file, event)?
        else {
            return Ok(None);
        };
        let _ = fs::remove_file(format!("{context_file}.context-delivery"));
        // A rejected steering hook never rejects the already-running target turn.
        if pending.managed && pending.active_prompt_id.as_deref() != Some(&pending.prompt_id) {
            return Ok(None);
        }
        Ok(Some("Claude blocked the prompt because Chariox context delivery failed. Retry after the kernel is ready.".into()))
    }

    pub(super) fn accept_context_delivery(
        &mut self,
        session_id: &str,
        agent_id: &str,
        run_id: &str,
        context_file: &str,
        event: &Value,
    ) -> Result<(), DaemonError> {
        let Some(pending) =
            self.matching_context_delivery(session_id, agent_id, run_id, context_file, event)?
        else {
            return Ok(());
        };
        if pending.managed {
            write_claude_native_marker(context_file, &format!("accepted:{}", pending.prompt_id));
            self.consume_accepted_failed_request_context(session_id, agent_id, context_file)?;
        } else if let Some(policy) = self.prompt_handoff {
            policy.accepted(session_id, agent_id, run_id)?;
        }
        let _ = fs::remove_file(format!("{context_file}.context-delivery"));
        Ok(())
    }
}
