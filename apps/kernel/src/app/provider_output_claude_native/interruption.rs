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

// MP-08 / MP-10 / MP-11: run after receipt/transcript observation so an
// already observed interruption cannot become an idle double-Esc/rewind.
pub(super) fn retry_closed_hook_interruption_after_observation(
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
        || !app
            .providers
            .get_run_for_agent(session_id, agent_id)
            .is_some_and(|current| current.id() == provider_run_id)
        || claude_native_marker(context_file).is_some_and(|marker| {
            marker.starts_with(&format!(
                "{CLAUDE_TRANSCRIPT_STOP_DRAIN_MARKER_PREFIX}{}:",
                prompt.id()
            ))
        })
    {
        return Ok(());
    }
    let Some(request_id) = turn
        .native_prompt_hook_request_id
        .as_deref()
        .filter(|id| valid_claude_hook_request_id(id))
    else {
        return Ok(());
    };
    retry_closed_hook_interruption(app, &run, context_file, &turn, request_id)
}

// MP-08 / MP-10 / MP-11: Claude can finish UserPromptSubmit before its
// interactive interrupt handler is ready. A single Esc in that gap is lost.
// Retry delivery only while both kernel ownership and the provider's current
// hook batch still identify this request. Settlement still requires a receipt.
fn retry_closed_hook_interruption(
    app: &mut DaemonApp,
    run: &RuntimeProviderRun,
    context_file: &str,
    turn: &crate::app::ActiveTurnState,
    request_id: &str,
) -> Result<(), DaemonError> {
    let batch_path = format!("{context_file}.hook-batch.json");
    let batch = fs::read_to_string(batch_path)
        .ok()
        .and_then(|raw| serde_json::from_str::<serde_json::Value>(&raw).ok());
    if !batch.is_some_and(|batch| {
        batch["id"].as_str() == Some(request_id)
            && batch["closed"].as_bool() == Some(true)
            && batch["pid"].as_u64().is_some_and(|pid| pid > 1)
    }) {
        return Ok(());
    }
    let now = unix_epoch_ms();
    if now.saturating_sub(turn.interruption_requested_at_ms.unwrap_or(now)) < 100 {
        return Ok(());
    }
    let path = std::path::Path::new(context_file)
        .parent()
        .unwrap()
        .join("hook-context-responses")
        .join(format!("{request_id}.interrupt-delivery"));
    let previous = fs::read_to_string(&path)
        .ok()
        .and_then(|raw| serde_json::from_str::<(u8, u64)>(&raw).ok())
        .unwrap_or((0, 0));
    if previous.0 >= 8 || now.saturating_sub(previous.1) < 250 {
        return Ok(());
    }
    app.write_provider_pty_input_for_runtime(run.id(), b"\x1b")?;
    let mut options = fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    use std::io::Write;
    let mut file = options
        .open(path)
        .map_err(|error| DaemonError::ProviderProtocol {
            provider_run_id: run.id().to_string(),
            operation: "claude_native_interrupt_delivery",
            message: error.to_string(),
        })?;
    file.write_all(format!("[{},{}]", previous.0 + 1, now).as_bytes())
        .map_err(|error| DaemonError::ProviderProtocol {
            provider_run_id: run.id().to_string(),
            operation: "claude_native_interrupt_delivery",
            message: error.to_string(),
        })
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
            }
        }
    }
    Ok(())
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
    mark_native_interruption_for_cancellation(app, session_id, provider_run_id, context_file, at)
}
