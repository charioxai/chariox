use std::time::{Duration, Instant};

use crate::app::{ActivePromptState, ActiveTurnPhase, ActiveTurnState, DaemonApp};

pub(crate) fn note_prompt_started(app: &mut DaemonApp, provider_run_id: &str) {
    app.prompt_activity.write().insert(
        provider_run_id.to_string(),
        ActivePromptState {
            last_output_at: None,
            saw_response_content: false,
            completion_recorded: false,
            settlement_requested: false,
        },
    );
    let active_turn = app
        .providers()
        .get_run(provider_run_id)
        .ok()
        .and_then(|run| {
            let session_id = run.session_id().to_string();
            let agent_id = run.agent_instance_id()?.to_string();
            let prompt_id = app
                .prompt_owner_active_prompt_for_agent(&session_id, &agent_id)
                .ok()
                .flatten()?
                .id()
                .to_string();
            Some(
                ActiveTurnState::new(session_id, agent_id, prompt_id, provider_run_id.to_string())
                    .with_phase(ActiveTurnPhase::AwaitingFirstOutput),
            )
        });
    if let Some(turn) = active_turn {
        app.active_turns.start(turn);
        app.active_turns.mark_awaiting_first_output(provider_run_id);
    }
}

pub(crate) fn clear_prompt_activity(app: &mut DaemonApp, provider_run_id: &str) {
    let prompt_activity = app.prompt_activity.write().remove(provider_run_id);
    let active_turn = app.active_turns.snapshot().remove(provider_run_id);
    if prompt_activity.is_some() || active_turn.is_some() {
        if let Ok(run) = app.providers().get_run(provider_run_id) {
            crate::runtime::command_latency::log_provider_turn_completed(
                &run,
                active_turn.as_ref(),
                prompt_activity.as_ref(),
            );
        }
    }
    if app.release_prompt_workspace_claim(provider_run_id) {
        crate::app::workflow_runtime::retry_blocked_workflow_claims_from_runtime(app);
    }
}

pub(crate) fn clear_active_turn(app: &mut DaemonApp, provider_run_id: &str) {
    app.active_turns.clear(provider_run_id);
}

pub(crate) fn note_prompt_settlement_requested(app: &mut DaemonApp, provider_run_id: &str) {
    app.active_turns.mark_settling(provider_run_id);
    app.prompt_activity
        .write()
        .entry(provider_run_id.to_string())
        .and_modify(|state| {
            state.last_output_at = Some(Instant::now());
            state.saw_response_content = true;
            state.settlement_requested = true;
        })
        .or_insert(ActivePromptState {
            last_output_at: Some(Instant::now()),
            saw_response_content: true,
            completion_recorded: false,
            settlement_requested: true,
        });
}

pub(crate) fn mark_prompt_completion_recorded(app: &mut DaemonApp, provider_run_id: &str) {
    if let Some(state) = app.prompt_activity.write().get_mut(provider_run_id) {
        state.completion_recorded = true;
    }
}

pub(crate) fn prompt_completion_recorded(app: &DaemonApp, provider_run_id: &str) -> bool {
    app.prompt_activity
        .read()
        .get(provider_run_id)
        .map(|state| state.completion_recorded)
        .unwrap_or(false)
}

pub(crate) fn prompt_completion_settlement_pending(app: &DaemonApp, provider_run_id: &str) -> bool {
    app.prompt_activity
        .read()
        .get(provider_run_id)
        .map(|state| state.completion_recorded && state.settlement_requested)
        .unwrap_or(false)
}

pub(crate) fn prompt_output_quiet_after_response(
    app: &DaemonApp,
    provider_run_id: &str,
    quiet_for: Duration,
) -> bool {
    app.prompt_activity
        .read()
        .get(provider_run_id)
        .is_some_and(|state| {
            state.saw_response_content
                && state
                    .last_output_at
                    .is_some_and(|last_output_at| last_output_at.elapsed() >= quiet_for)
        })
}
