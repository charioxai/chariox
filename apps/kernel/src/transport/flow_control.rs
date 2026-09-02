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
            active_tool_ids: std::collections::BTreeSet::new(),
        },
    );
    let active_turn = app
        .providers()
        .get_run(provider_run_id)
        .ok()
        .and_then(|run| {
            let session_id = run.session_id().to_string();
            let agent_id = run.agent_instance_id()?.to_string();
            let prompt = app
                .prompt_owner_active_prompt_for_agent(&session_id, &agent_id)
                .ok()
                .flatten()?;
            let prompt_id = prompt.id().to_string();
            Some(
                ActiveTurnState::new(session_id, agent_id, prompt_id, provider_run_id.to_string())
                    .with_prompt_metadata(&prompt)
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
    let active_turn = app.active_turns.get(provider_run_id);
    if prompt_activity.is_some() || active_turn.is_some() {
        if let Ok(run) = app.providers().get_run(provider_run_id) {
            crate::runtime::command_latency::log_provider_turn_completed(
                &run,
                active_turn.as_ref(),
                prompt_activity.as_ref(),
            );
        }
    }
    app.active_turns.clear(provider_run_id);
    if app.release_prompt_workspace_claim(provider_run_id) {
        crate::app::workflow_runtime::retry_blocked_workflow_claims_from_runtime(app);
    }
}

pub(crate) fn note_prompt_settlement_requested(app: &mut DaemonApp, provider_run_id: &str) {
    app.active_turns.mark_settling(provider_run_id);
    app.prompt_activity
        .write()
        .entry(provider_run_id.to_string())
        .and_modify(|state| {
            state.request_settlement();
        })
        .or_insert(ActivePromptState {
            last_output_at: Some(Instant::now()),
            saw_response_content: true,
            completion_recorded: false,
            settlement_requested: true,
            active_tool_ids: std::collections::BTreeSet::new(),
        });
}

pub(crate) fn note_prompt_tool_output(
    app: &mut DaemonApp,
    provider_run_id: &str,
    merge_key: Option<&str>,
    bytes: &[u8],
) {
    if let Some(state) = app.prompt_activity.write().get_mut(provider_run_id) {
        state.observe_provider_tool(merge_key, bytes);
    }
}

pub(crate) fn note_prompt_output(app: &mut DaemonApp, provider_run_id: &str) {
    if let Some(state) = app.prompt_activity.write().get_mut(provider_run_id) {
        state.last_output_at = Some(Instant::now());
    }
    app.active_turns.mark_streaming(provider_run_id);
}

pub(crate) fn note_prompt_response_content(app: &mut DaemonApp, provider_run_id: &str) {
    let first_response_content = {
        let mut prompt_activity = app.prompt_activity.write();
        if let Some(state) = prompt_activity.get_mut(provider_run_id) {
            let first_response_content = !state.saw_response_content;
            state.last_output_at = Some(Instant::now());
            state.saw_response_content = true;
            first_response_content
        } else {
            false
        }
    };
    if first_response_content {
        app.active_turns.mark_streaming(provider_run_id);
        if let Ok(run) = app.providers().get_run(provider_run_id) {
            let active_turn = app.active_turns.get(provider_run_id);
            crate::runtime::command_latency::log_provider_first_response_content(
                &run,
                active_turn.as_ref(),
            );
        }
    }
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
