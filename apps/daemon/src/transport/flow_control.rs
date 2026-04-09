use std::time::Instant;

use crate::app::{ActivePromptState, DaemonApp};
use crate::error::DaemonError;
use crate::session::PromptStatus;

enum PromptSettlementAction {
    Complete,
    FinalizeCancellation,
    ClearActivityOnly,
}

pub(crate) fn note_prompt_started(app: &mut DaemonApp, provider_run_id: &str) {
    app.prompt_activity.insert(
        provider_run_id.to_string(),
        ActivePromptState {
            last_output_at: None,
            saw_response_content: false,
            completion_recorded: false,
        },
    );
}

pub(crate) fn note_prompt_output(app: &mut DaemonApp, provider_run_id: &str) {
    if let Some(state) = app.prompt_activity.get_mut(provider_run_id) {
        state.last_output_at = Some(Instant::now());
    }
}

pub(crate) fn note_prompt_response_content(app: &mut DaemonApp, provider_run_id: &str) {
    if let Some(state) = app.prompt_activity.get_mut(provider_run_id) {
        state.last_output_at = Some(Instant::now());
        state.saw_response_content = true;
    }
}

pub(crate) fn clear_prompt_activity(app: &mut DaemonApp, provider_run_id: &str) {
    app.prompt_activity.remove(provider_run_id);
}

pub(crate) fn note_prompt_settlement_requested(app: &mut DaemonApp, provider_run_id: &str) {
    app.prompt_activity
        .entry(provider_run_id.to_string())
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

pub(crate) fn mark_prompt_completion_recorded(app: &mut DaemonApp, provider_run_id: &str) {
    if let Some(state) = app.prompt_activity.get_mut(provider_run_id) {
        state.completion_recorded = true;
    }
}

pub(crate) fn prompt_completion_recorded(app: &DaemonApp, provider_run_id: &str) -> bool {
    app.prompt_activity
        .get(provider_run_id)
        .map(|state| state.completion_recorded)
        .unwrap_or(false)
}

pub(crate) fn maybe_complete_active_prompt(
    app: &mut DaemonApp,
    session_id: &str,
    provider_run_id: &str,
) -> Result<(), DaemonError> {
    let should_complete = app
        .prompt_activity
        .get(provider_run_id)
        .map(|state| {
            (state.saw_response_content || state.completion_recorded)
                && state
                    .last_output_at
                    .map(|last_output_at| last_output_at.elapsed() >= app.prompt_idle_timeout)
                    .unwrap_or(false)
        })
        .unwrap_or(false);

    if !should_complete {
        return Ok(());
    }

    match prompt_settlement_action(app, session_id, provider_run_id)? {
        PromptSettlementAction::ClearActivityOnly => clear_prompt_activity(app, provider_run_id),
        PromptSettlementAction::FinalizeCancellation => {
            let agent_id = provider_run_agent_id(app, provider_run_id)?;
            let _ = app.finalize_active_prompt_cancellation(
                session_id,
                &agent_id,
                Some(provider_run_id),
            )?;
        }
        PromptSettlementAction::Complete => {
            let agent_id = provider_run_agent_id(app, provider_run_id)?;
            let _ = app.complete_active_prompt(session_id, &agent_id, Some(provider_run_id))?;
        }
    }
    Ok(())
}

fn prompt_settlement_action(
    app: &DaemonApp,
    session_id: &str,
    provider_run_id: &str,
) -> Result<PromptSettlementAction, DaemonError> {
    let agent_id = provider_run_agent_id(app, provider_run_id)?;
    let session = app.sessions().get_session(session_id)?;
    let Some(prompt) = session.active_prompt_for_agent(&agent_id) else {
        return Ok(PromptSettlementAction::ClearActivityOnly);
    };
    if prompt.status() == PromptStatus::Cancelling {
        return Ok(PromptSettlementAction::FinalizeCancellation);
    }
    Ok(PromptSettlementAction::Complete)
}

fn provider_run_agent_id(app: &DaemonApp, provider_run_id: &str) -> Result<String, DaemonError> {
    app.providers()
        .get_run(provider_run_id)?
        .agent_instance_id()
        .map(str::to_string)
        .ok_or_else(|| DaemonError::AgentNotFound {
            agent_id: "provider run has no agent".to_string(),
        })
}
