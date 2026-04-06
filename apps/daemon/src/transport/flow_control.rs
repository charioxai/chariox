use std::time::Instant;

use crate::app::{ActivePromptState, DaemonApp};
use crate::error::DaemonError;
use crate::session::PromptStatus;

pub(crate) fn note_prompt_started(app: &mut DaemonApp, session_id: &str) {
    app.prompt_activity.insert(
        session_id.to_string(),
        ActivePromptState {
            last_output_at: None,
            saw_response_content: false,
            completion_recorded: false,
        },
    );
}

pub(crate) fn note_prompt_output(app: &mut DaemonApp, session_id: &str) {
    if let Some(state) = app.prompt_activity.get_mut(session_id) {
        state.last_output_at = Some(Instant::now());
    }
}

pub(crate) fn note_prompt_response_content(app: &mut DaemonApp, session_id: &str) {
    if let Some(state) = app.prompt_activity.get_mut(session_id) {
        state.last_output_at = Some(Instant::now());
        state.saw_response_content = true;
    }
}

pub(crate) fn clear_prompt_activity(app: &mut DaemonApp, session_id: &str) {
    app.prompt_activity.remove(session_id);
}

pub(crate) fn note_prompt_settlement_requested(app: &mut DaemonApp, session_id: &str) {
    app.prompt_activity
        .entry(session_id.to_string())
        .and_modify(|state| {
            state.last_output_at = Some(Instant::now());
            state.saw_response_content = true;
        })
        .or_insert(ActivePromptState {
            last_output_at: Some(Instant::now()),
            saw_response_content: true,
            completion_recorded: false,
        });
}

pub(crate) fn mark_prompt_completion_recorded(app: &mut DaemonApp, session_id: &str) {
    if let Some(state) = app.prompt_activity.get_mut(session_id) {
        state.completion_recorded = true;
    }
}

pub(crate) fn prompt_completion_recorded(app: &DaemonApp, session_id: &str) -> bool {
    app.prompt_activity
        .get(session_id)
        .map(|state| state.completion_recorded)
        .unwrap_or(false)
}

pub(crate) fn maybe_complete_active_prompt(
    app: &mut DaemonApp,
    session_id: &str,
) -> Result<(), DaemonError> {
    let should_complete = app
        .prompt_activity
        .get(session_id)
        .map(|state| {
            state.saw_response_content
                && state
                    .last_output_at
                    .map(|last_output_at| last_output_at.elapsed() >= app.prompt_idle_timeout)
                    .unwrap_or(false)
        })
        .unwrap_or(false);

    if !should_complete {
        return Ok(());
    }

    if app
        .sessions()
        .get_session(session_id)?
        .active_prompt()
        .is_none()
    {
        clear_prompt_activity(app, session_id);
        return Ok(());
    }

    if app
        .sessions()
        .get_session(session_id)?
        .active_prompt()
        .map(|prompt| prompt.status())
        == Some(PromptStatus::Cancelling)
    {
        let _ = app.finalize_active_prompt_cancellation(session_id)?;
    } else {
        let _ = app.complete_active_prompt(session_id)?;
    }
    Ok(())
}
