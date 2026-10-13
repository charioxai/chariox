use super::permission::valid_claude_hook_request_id;
use super::*;

// MP-08 / MP-10 / MP-11: Esc is ignored by Claude during some prompt-hook
// preparation. Ask that exact hook to decline admission; only its receipt
// below can settle the kernel-owned cancellation.
pub(super) fn request_pending_prompt_hook_cancellation(
    app: &mut DaemonApp,
    session_id: &str,
    provider_run_id: &str,
    context_file: &str,
) -> Result<(), DaemonError> {
    let run = app.providers.get_run(provider_run_id)?;
    if !crate::provider::provider_run_is_claude_native_tui(&run) {
        return Ok(());
    }
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
        || turn.interruption_requested_at_ms.is_none()
        || turn.native_prompt_hook_resolved
        || !app
            .providers
            .get_run_for_agent(session_id, agent_id)
            .is_some_and(|current| current.id() == provider_run_id)
    {
        return Ok(());
    }
    let Some(request_id) = turn.native_prompt_hook_request_id.as_deref() else {
        return Ok(());
    };
    if !valid_claude_hook_request_id(request_id) {
        return Ok(());
    }
    let Some(parent) = std::path::Path::new(context_file).parent() else {
        return Ok(());
    };
    let directory = parent.join("hook-context-responses");
    fs::create_dir_all(&directory).map_err(|error| DaemonError::ProviderProtocol {
        provider_run_id: provider_run_id.to_string(),
        operation: "claude_prompt_hook_cancel",
        message: error.to_string(),
    })?;
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    match options.open(directory.join(format!("{request_id}.cancelled"))) {
        Ok(_) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => Ok(()),
        Err(error) => Err(DaemonError::ProviderProtocol {
            provider_run_id: provider_run_id.to_string(),
            operation: "claude_prompt_hook_cancel",
            message: error.to_string(),
        }),
    }
}

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
    app.active_turns
        .confirm_native_interruption(provider_run_id, prompt.id());
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

const CANCELLATION_RETRY_MS: u64 = 2_000;
const CANCELLATION_DEADLINE_MS: u64 = 10_000;
const CANCELLATION_FAILURE: &str =
    "Claude stopped responding to cancel; the run was ended. Send another prompt to restart it.";

pub(super) fn fail_cancellation(
    app: &mut DaemonApp,
    session_id: &str,
    provider_run_id: &str,
    context_file: &str,
    reason: &str,
) -> Result<(), DaemonError> {
    let run = app.providers.get_run(provider_run_id)?;
    let Some(agent_id) = run.agent_instance_id() else {
        return Ok(());
    };
    let Some(prompt) = app.prompt_owner_active_prompt_for_agent(session_id, agent_id)? else {
        return Ok(());
    };
    if !crate::provider::provider_run_is_claude_native_tui(&run)
        || prompt.status() != crate::session::PromptStatus::Cancelling
        || !app.active_turns.get(provider_run_id).is_some_and(|turn| {
            turn.prompt_id == prompt.id()
                && turn.session_id == session_id
                && turn.agent_id == agent_id
                && !turn.native_interruption_confirmed
        })
        || !app
            .providers
            .get_run_for_agent(session_id, agent_id)
            .is_some_and(|current| current.id() == provider_run_id)
    {
        return Ok(());
    }
    // Project the bounded error while this exact prompt still owns output,
    // before the common failure path can promote a queued successor.
    ProviderOutputFanout::new(app).fan_out(
        session_id,
        provider_run_id,
        TerminalOutputKind::ProviderError,
        Some(format!("native-cancel-error:{}", prompt.id())),
        app.attachments.list_session_attachment_ids(session_id),
        reason.as_bytes(),
    );
    app.end_agent_provider_run(session_id, agent_id)?;
    let providers = app.providers.clone();
    let turns = app.active_turn_store();
    let activity = app.prompt_activity_store();
    let projection = app.agent_runtime_projection_store();
    super::super::provider_output_prompt_settlement::ProviderOutputPromptSettlement::new(
        app, providers, turns, activity, projection,
    )
    .fail_for_terminal_failure_if_matches(
        session_id,
        provider_run_id,
        Some(prompt.id()),
        Some(crate::provider::ProviderRunTermination::runtime_failure(
            reason,
            unix_epoch_ms(),
        )),
        reason,
    )?;
    let _ = fs::write(context_file, "");
    write_claude_native_marker(context_file, "");
    Ok(())
}

// MP-08 / MP-10 / MP-11: called after draining protected official artifacts.
// Re-delivery is not acknowledgement. Only a receipt cancels; an unusable
// run reaches the shared error/termination path before admitting more work.
pub(super) fn resolve_pending_cancellation(
    app: &mut DaemonApp,
    session_id: &str,
    provider_run_id: &str,
    context_file: &str,
) -> Result<(), DaemonError> {
    let run = app.providers.get_run(provider_run_id)?;
    if !crate::provider::provider_run_is_claude_native_tui(&run) {
        return Ok(());
    }
    let Some(agent_id) = run.agent_instance_id() else {
        return Ok(());
    };
    let Some(prompt) = app.prompt_owner_active_prompt_for_agent(session_id, agent_id)? else {
        return Ok(());
    };
    let Some(turn) = app.active_turns.get(provider_run_id) else {
        return Ok(());
    };
    let Some(requested_at_ms) = turn.interruption_requested_at_ms else {
        return Ok(());
    };
    if prompt.status() != crate::session::PromptStatus::Cancelling
        || turn.prompt_id != prompt.id()
        || turn.session_id != session_id
        || turn.agent_id != agent_id
        || turn.native_interruption_confirmed
        || !app
            .providers
            .get_run_for_agent(session_id, agent_id)
            .is_some_and(|current| current.id() == provider_run_id)
        || claude_transcript_stop_drain_marker(context_file).is_some()
    {
        return Ok(());
    }
    let now = unix_epoch_ms();
    if now.saturating_sub(requested_at_ms) >= CANCELLATION_DEADLINE_MS {
        // End only this owned run, then settle with an explicit, bounded reason.
        // Queue advancement and next-run recovery stay in the normal kernel path.
        fail_cancellation(
            app,
            session_id,
            provider_run_id,
            context_file,
            if turn.interruption_retry_sent_at_ms.is_some() {
                CANCELLATION_FAILURE
            } else {
                "Claude cancellation could not be delivered before the deadline; the run was ended."
            },
        )?;
        return Ok(());
    }
    // MP-08 / MP-10 / MP-11: decline a still-pending hook; after its response
    // is written, deliver Esc without waiting for a completed assistant block.
    // This is interrupt readiness, never cancellation acknowledgement.
    if !turn.native_response_started && !turn.native_prompt_hook_resolved {
        return Ok(());
    }
    if turn
        .interruption_retry_sent_at_ms
        .is_some_and(|sent_at_ms| now.saturating_sub(sent_at_ms) < CANCELLATION_RETRY_MS)
    {
        return Ok(());
    }
    // A successor hook belongs to a different provider prompt. Never send it
    // an interrupt for the previous canonical prompt.
    if let Some(request_id) = turn.native_prompt_hook_request_id.as_deref() {
        if fs::read_to_string(format!("{context_file}.hook-batch.json"))
            .ok()
            .and_then(|raw| serde_json::from_str::<serde_json::Value>(&raw).ok())
            .is_some_and(|batch| {
                batch.get("id").and_then(serde_json::Value::as_str) != Some(request_id)
            })
        {
            return Ok(());
        }
    }
    app.active_turns
        .note_native_interruption_delivery(provider_run_id, prompt.id(), now);
    app.terminal.record_input(
        session_id,
        provider_run_id,
        prompt.source_attachment_id(),
        crate::provider::provider_prompt_abort_input(&run),
    );
    if app
        .write_provider_pty_input_for_runtime(
            provider_run_id,
            crate::provider::provider_prompt_abort_input(&run),
        )
        .is_err()
    {
        fail_cancellation(
            app,
            session_id,
            provider_run_id,
            context_file,
            CANCELLATION_FAILURE,
        )?;
    }
    Ok(())
}

pub(super) fn bind_prompt_hook_request(
    app: &mut DaemonApp,
    session_id: &str,
    provider_run_id: &str,
    context_file: &str,
    prompt_id: &str,
    event: &serde_json::Value,
) -> Result<(), DaemonError> {
    if let Some(request_id) = event
        .get("hook_context_request_id")
        .and_then(serde_json::Value::as_str)
        .filter(|id| valid_claude_hook_request_id(id))
    {
        app.active_turns
            .bind_native_prompt_hook_request(provider_run_id, prompt_id, request_id);
    } else {
        return Ok(());
    }
    // Cancellation may precede hook publication. Decline in this same pass,
    // before the context response allows the pending hook to finish.
    request_pending_prompt_hook_cancellation(app, session_id, provider_run_id, context_file)?;
    if let Some(turn) = app.active_turns.get(provider_run_id) {
        if turn.prompt_id == prompt_id
            && app
                .providers
                .get_run_for_agent(session_id, &turn.agent_id)
                .is_some_and(|current| current.id() == provider_run_id)
        {
            if let Some(request_id) = turn.native_prompt_hook_request_id.as_deref() {
                let context = fs::read_to_string(context_file).unwrap_or_default();
                write_claude_hook_context_response(context_file, request_id, &context);
                return Ok(());
            }
        }
    }
    reply_ignored_prompt_hook(context_file, event);
    Ok(())
}

pub(super) fn reply_ignored_prompt_hook(context_file: &str, event: &serde_json::Value) {
    if let Some(request_id) = event
        .get("hook_context_request_id")
        .and_then(serde_json::Value::as_str)
        .filter(|id| valid_claude_hook_request_id(id))
    {
        write_claude_hook_context_response(context_file, request_id, "");
    }
}

pub(super) fn observe_prompt_hook_ready(
    app: &mut DaemonApp,
    provider_run_id: &str,
    context_file: &str,
    event: &serde_json::Value,
) {
    if let Some(request_id) = event
        .get("hook_context_request_id")
        .and_then(serde_json::Value::as_str)
        .filter(|id| valid_claude_hook_request_id(id))
    {
        app.active_turns
            .note_native_prompt_hook_resolved(provider_run_id, request_id);
        // MP-08 / MP-10 / MP-11: resolved hooks cannot consume later markers.
        if let Some(parent) = std::path::Path::new(context_file).parent() {
            let _ = fs::remove_file(
                parent
                    .join("hook-context-responses")
                    .join(format!("{request_id}.cancelled")),
            );
        }
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
    ) && event.get("decision").and_then(serde_json::Value::as_str) != Some("block")
    {
        return Ok(());
    }
    if !app
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
    mark_native_interruption_for_cancellation(app, session_id, provider_run_id, context_file, at)?;
    observe_prompt_hook_ready(app, provider_run_id, context_file, event);
    Ok(())
}
