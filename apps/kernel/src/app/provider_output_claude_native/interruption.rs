use super::*;

// MP-08 / MP-10 / MP-11: Stop is absent after native interruption. Accept
// its transcript receipt only after this exact kernel-owned cancellation.
pub(super) fn mark_native_interruption_for_cancellation(
    app: &mut DaemonApp,
    session_id: &str,
    provider_run_id: &str,
    context_file: &str,
    observed_at_ms: u64,
) -> Result<(), DaemonError> {
    let run = app.providers.get_run(provider_run_id)?;
    let Some(agent_id) = run.agent_instance_id() else {
        return Ok(());
    };
    let Some(prompt) = app.prompt_owner_active_prompt_for_agent(session_id, agent_id)? else {
        return Ok(());
    };
    let Some(turn) = app.active_turns.get(provider_run_id) else {
        return Ok(());
    };
    if prompt.status() != crate::session::PromptStatus::Cancelling
        || turn.prompt_id != prompt.id()
        || turn.session_id != session_id
        || turn.agent_id != agent_id
        || !turn
            .interruption_requested_at_ms
            .is_some_and(|at| observed_at_ms >= at)
        || !app
            .providers
            .get_run_for_agent(session_id, agent_id)
            .is_some_and(|current| current.id() == provider_run_id)
    {
        return Ok(());
    }
    write_claude_native_marker(
        context_file,
        &format!(
            "{CLAUDE_TRANSCRIPT_STOP_DRAIN_MARKER_PREFIX}{}:{}",
            prompt.id(),
            unix_epoch_ms()
        ),
    );
    Ok(())
}

pub(super) fn bind_prompt_hook_request(
    app: &DaemonApp,
    provider_run_id: &str,
    prompt_id: &str,
    event: &serde_json::Value,
) {
    if let Some(request_id) = event
        .get("hook_context_request_id")
        .and_then(serde_json::Value::as_str)
    {
        app.active_turns
            .bind_native_prompt_hook_request(provider_run_id, prompt_id, request_id);
    }
}

pub(super) fn observe_prompt_hook_interruption(
    app: &mut DaemonApp,
    session_id: &str,
    provider_run_id: &str,
    context_file: &str,
    event: &serde_json::Value,
) -> Result<(), DaemonError> {
    if !matches!(
        event.get("signal").and_then(serde_json::Value::as_str),
        Some("SIGINT" | "SIGTERM")
    ) || !app
        .providers
        .get_run(provider_run_id)
        .is_ok_and(|run| !run.client_interface().is_chariox())
    {
        return Ok(());
    }
    let Some(request_id) = event
        .get("hook_context_request_id")
        .and_then(serde_json::Value::as_str)
    else {
        return Ok(());
    };
    if !app
        .active_turns
        .get(provider_run_id)
        .is_some_and(|turn| turn.native_prompt_hook_request_id.as_deref() == Some(request_id))
    {
        return Ok(());
    }
    let Some(at) = event
        .get("at")
        .and_then(serde_json::Value::as_str)
        .and_then(|at| chrono::DateTime::parse_from_rfc3339(at).ok())
        .and_then(|at| u64::try_from(at.timestamp_millis()).ok())
    else {
        return Ok(());
    };
    mark_native_interruption_for_cancellation(app, session_id, provider_run_id, context_file, at)
}
