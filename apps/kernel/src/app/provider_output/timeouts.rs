use crate::app::provider_output_prompt_settlement::ProviderOutputPromptSettlement;
use crate::app::DaemonApp;
use crate::error::DaemonError;
use crate::provider::ProviderRunState;

pub(super) fn reap_provider_first_output_timeouts(
    app: &mut DaemonApp,
    session_id: &str,
) -> Result<(), DaemonError> {
    let timed_out = app_first_output_timeout_candidates(app, session_id);
    for timeout in timed_out {
        let diagnostic = crate::app::provider_first_output_timeout_diagnostic(timeout.elapsed_ms);
        let run = app
            .providers
            .record_terminal_diagnostic(&timeout.provider_run_id, diagnostic.clone())?;
        app.update_provider_run_projection(run);
        app.record_notice(
            session_id,
            Some(&timeout.provider_run_id),
            app.attachments.list_session_attachment_ids(session_id),
            diagnostic.clone(),
        );
        crate::logging::warn_with_fields(
            "daemon.provider",
            "provider prompt produced no first output before timeout",
            serde_json::json!({
                "session_id": session_id,
                "agent_id": timeout.agent_id,
                "provider_run_id": timeout.provider_run_id,
                "elapsed_ms": timeout.elapsed_ms,
            }),
        );
        let provider_store = app.providers.clone();
        let active_turns = app.active_turns.clone();
        let prompt_activity = app.prompt_activity.clone();
        let agent_runtime_projection = app.agent_runtime_projection_store();
        ProviderOutputPromptSettlement::new(
            app,
            provider_store,
            active_turns,
            prompt_activity,
            agent_runtime_projection,
        )
        .fail_for_terminal_failure(session_id, &timeout.provider_run_id, &diagnostic)?;
    }
    Ok(())
}

pub(super) fn reap_provider_inactivity_timeouts(
    app: &mut DaemonApp,
    session_id: &str,
) -> Result<(), DaemonError> {
    let timed_out = app_inactivity_timeout_candidates(app, session_id);
    for timeout in timed_out {
        let diagnostic = crate::app::provider_inactivity_timeout_diagnostic(timeout.elapsed_ms);
        let run = app
            .providers
            .record_terminal_diagnostic(&timeout.provider_run_id, diagnostic.clone())?;
        app.update_provider_run_projection(run);
        app.record_notice(
            session_id,
            Some(&timeout.provider_run_id),
            app.attachments.list_session_attachment_ids(session_id),
            diagnostic.clone(),
        );
        crate::logging::warn_with_fields(
            "daemon.provider",
            "provider prompt produced no output after prior activity before timeout",
            serde_json::json!({
                "session_id": session_id,
                "agent_id": timeout.agent_id,
                "provider_run_id": timeout.provider_run_id,
                "elapsed_ms": timeout.elapsed_ms,
            }),
        );
        let provider_store = app.providers.clone();
        let active_turns = app.active_turns.clone();
        let prompt_activity = app.prompt_activity.clone();
        let agent_runtime_projection = app.agent_runtime_projection_store();
        ProviderOutputPromptSettlement::new(
            app,
            provider_store,
            active_turns,
            prompt_activity,
            agent_runtime_projection,
        )
        .fail_for_terminal_failure(session_id, &timeout.provider_run_id, &diagnostic)?;
    }
    Ok(())
}

fn app_first_output_timeout_candidates(
    app: &DaemonApp,
    session_id: &str,
) -> Vec<crate::app::ProviderFirstOutputTimeoutCandidate> {
    let prompt_activity = app.prompt_activity.read().clone();
    let active_turns = app.active_turns.snapshot();
    let Ok(session) = app.sessions.get_session(session_id) else {
        return Vec::new();
    };
    crate::app::provider_first_output_timeout_candidates(
        session_id,
        active_turns.into_values(),
        &prompt_activity,
        |turn| {
            app.providers
                .get_run(&turn.provider_run_id)
                .is_ok_and(|run| {
                    run.session_id() == session_id
                        && run.agent_instance_id() == Some(turn.agent_id.as_str())
                        && run.terminal_diagnostic().is_none()
                        && matches!(
                            run.state(),
                            ProviderRunState::Starting
                                | ProviderRunState::Running
                                | ProviderRunState::Parked
                        )
                })
        },
        |turn| {
            app.prompt_state_owner
                .active_prompt_for_agent_snapshot(&session, &turn.agent_id)
                .is_some_and(|prompt| prompt.id() == turn.prompt_id)
        },
    )
}

fn app_inactivity_timeout_candidates(
    app: &DaemonApp,
    session_id: &str,
) -> Vec<crate::app::ProviderInactivityTimeoutCandidate> {
    let prompt_activity = app.prompt_activity.read().clone();
    let active_turns = app.active_turns.snapshot();
    let Ok(session) = app.sessions.get_session(session_id) else {
        return Vec::new();
    };
    crate::app::provider_inactivity_timeout_candidates(
        session_id,
        active_turns.into_values(),
        &prompt_activity,
        |turn| {
            app.providers
                .get_run(&turn.provider_run_id)
                .is_ok_and(|run| {
                    run.session_id() == session_id
                        && run.agent_instance_id() == Some(turn.agent_id.as_str())
                        && run.terminal_diagnostic().is_none()
                        && matches!(
                            run.state(),
                            ProviderRunState::Starting
                                | ProviderRunState::Running
                                | ProviderRunState::Parked
                        )
                })
        },
        |turn| {
            app.prompt_state_owner
                .active_prompt_for_agent_snapshot(&session, &turn.agent_id)
                .is_some_and(|prompt| prompt.id() == turn.prompt_id)
        },
    )
}
