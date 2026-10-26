//! MP-08 / MP-10 / MP-11: acknowledge hidden context only after official hooks flush it.
use super::*;
use crate::provider::skill_delivery::TypedSkillLoad;
use serde::{Deserialize, Serialize};

#[derive(Deserialize, Serialize)]
struct PendingDelivery {
    request_id: String,
    run_id: String,
    prompt_id: String,
    managed: bool,
    #[serde(default)]
    active_prompt_id: Option<String>,
    /// The managed resubmission of a typed native command, which no dispatch consumed
    /// the compaction handoff for. It holds even when the command loaded no skill.
    #[serde(default)]
    typed_redirect: bool,
    /// Skills the worker put into this hook context for a `$name` the user typed.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    typed_skills: Vec<TypedSkillLoad>,
}

/// Who produced the hook context being delivered.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum DeliveryOrigin {
    /// A turn typed into the native composer.
    Native,
    /// A turn Chariox submitted itself.
    Managed,
    /// Chariox's resubmission of a typed native command.
    TypedRedirect,
}

pub(super) fn stage(
    context_file: &str,
    request_id: &str,
    run_id: &str,
    prompt_id: &str,
    active_prompt_id: &str,
    origin: DeliveryOrigin,
    typed_skills: Vec<TypedSkillLoad>,
) -> Result<(), DaemonError> {
    permission::validate_context_request_id(request_id)?;
    let pending = PendingDelivery {
        request_id: request_id.into(),
        run_id: run_id.into(),
        prompt_id: prompt_id.into(),
        managed: origin != DeliveryOrigin::Native,
        active_prompt_id: Some(active_prompt_id.into()),
        typed_redirect: origin == DeliveryOrigin::TypedRedirect,
        typed_skills,
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

fn read_pending(context_file: &str) -> Option<PendingDelivery> {
    fs::read(format!("{context_file}.context-delivery"))
        .ok()
        .and_then(|text| serde_json::from_slice(&text).ok())
}

/// Settlement may trail a receipt for another turn; clear only its own binding.
pub(crate) fn clear_claude_context_delivery_for_settled_prompt(
    run: &RuntimeProviderRun,
    prompt_id: &str,
) {
    let Some(context_file) = run.pty_env().get("CHARIOX_CLAUDE_NATIVE_CONTEXT") else {
        return;
    };
    if read_pending(context_file).is_some_and(|pending| {
        pending.run_id == run.id()
            && pending
                .active_prompt_id
                .as_deref()
                .unwrap_or(&pending.prompt_id)
                == prompt_id
    }) {
        let _ = fs::remove_file(format!("{context_file}.context-delivery"));
    }
}

impl ProviderOutputClaudeNativeBridge<'_> {
    fn current_context_delivery(
        &mut self,
        session_id: &str,
        agent_id: &str,
        run_id: &str,
        context_file: &str,
    ) -> Result<Option<PendingDelivery>, DaemonError> {
        let pending = read_pending(context_file);
        let active = self
            .app
            .prompt_owner_active_prompt_for_agent(session_id, agent_id)?;
        if pending.as_ref().is_some_and(|pending| {
            pending.run_id == run_id
                && active.as_ref().map(|prompt| prompt.id())
                    == Some(
                        pending
                            .active_prompt_id
                            .as_deref()
                            .unwrap_or(&pending.prompt_id),
                    )
        }) {
            return Ok(pending);
        }
        // A late receipt must neither retain an obsolete owner nor remove a
        // binding for the current live target merely because its request differs.
        let _ = fs::remove_file(format!("{context_file}.context-delivery"));
        Ok(None)
    }

    /// A late hook cannot deliver context, but its failure must still settle
    /// the exact managed prompt that was admitted before PTY injection.
    pub(super) fn bind_expired_managed_context_failure(
        &mut self,
        session_id: &str,
        agent_id: &str,
        run_id: &str,
        context_file: &str,
        event: &Value,
    ) -> Result<(), DaemonError> {
        if self
            .current_context_delivery(session_id, agent_id, run_id, context_file)?
            .is_some()
        {
            return Ok(());
        }
        let Some(request_id) = event.get("hook_context_request_id").and_then(Value::as_str) else {
            return Ok(());
        };
        let Some(prompt) = event.get("prompt").and_then(Value::as_str) else {
            return Ok(());
        };
        let marker = claude_native_marker(context_file);
        let Some(dispatch_id) = marker.as_deref().and_then(claude_native_dispatch_prompt_id) else {
            return Ok(());
        };
        let active = self
            .app
            .prompt_owner_active_prompt_for_agent(session_id, agent_id)?;
        if active.as_ref().map(|prompt| prompt.id()) != Some(dispatch_id)
            || !claude_headless_dispatch_matches_prompt(context_file, dispatch_id, prompt)
        {
            return Ok(());
        }
        let current = self.app.providers.get_run(run_id)?;
        if let Some(session) = event.get("session_id").and_then(Value::as_str) {
            if current.provider_session_id().or_else(|| {
                current
                    .resume_state()
                    .provider_session_id(current.adapter_key())
            }) != Some(session)
            {
                return Ok(());
            }
        }
        stage(
            context_file,
            request_id,
            run_id,
            dispatch_id,
            dispatch_id,
            DeliveryOrigin::Managed,
            Vec::new(),
        )
    }

    fn matching_context_delivery(
        &mut self,
        session_id: &str,
        agent_id: &str,
        run_id: &str,
        context_file: &str,
        event: &Value,
    ) -> Result<Option<PendingDelivery>, DaemonError> {
        let Some(pending) =
            self.current_context_delivery(session_id, agent_id, run_id, context_file)?
        else {
            return Ok(None);
        };
        if event.get("hook_context_request_id").and_then(Value::as_str)
            != Some(pending.request_id.as_str())
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
        Ok(Some(pending.prompt_id))
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
        let prompt_id = pending
            .active_prompt_id
            .as_deref()
            .unwrap_or(&pending.prompt_id);
        self.app
            .providers
            .acknowledge_skill_hook_delivery(session_id, agent_id, prompt_id, run_id)?;
        if !pending.typed_skills.is_empty() {
            self.acknowledge_typed_skills(session_id, run_id, prompt_id, &pending.typed_skills)?;
        }
        if pending.managed {
            write_claude_native_marker(context_file, &format!("accepted:{}", pending.prompt_id));
            self.consume_accepted_failed_request_context(session_id, agent_id, context_file)?;
        }
        // A typed native command is resubmitted as a managed turn that no
        // dispatch consumed the handoff for, so the acceptance must.
        if !pending.managed || pending.typed_redirect {
            if let Some(policy) = self.prompt_handoff {
                policy.accepted(session_id, agent_id, run_id)?;
            }
        }
        let _ = fs::remove_file(format!("{context_file}.context-delivery"));
        Ok(())
    }
}
