use std::time::Instant;

use crate::app::{ActivePromptState, DaemonApp};
use crate::error::DaemonError;
use crate::session::{PromptQueueItem, PromptStatus};

#[derive(Debug, PartialEq, Eq)]
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
    if app.release_prompt_workspace_claim(provider_run_id) {
        crate::scheduler::runtime::retry_blocked_workflow_claims(app);
    }
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
    let Some(prompt) = active_prompt_for_settlement(app, session_id, &agent_id)? else {
        return Ok(PromptSettlementAction::ClearActivityOnly);
    };
    if prompt.status() == PromptStatus::Cancelling {
        return Ok(PromptSettlementAction::FinalizeCancellation);
    }
    Ok(PromptSettlementAction::Complete)
}

fn active_prompt_for_settlement(
    app: &DaemonApp,
    session_id: &str,
    agent_id: &str,
) -> Result<Option<PromptQueueItem>, DaemonError> {
    if let Some(prompt) = app
        .agent_runtime_projection_store()
        .get(agent_id)
        .filter(|projection| projection.session_id == session_id)
        .and_then(|projection| projection.active_prompt)
    {
        return Ok(Some(prompt));
    }
    Ok(app
        .sessions()
        .get_session(session_id)?
        .active_prompt_for_agent(agent_id)
        .cloned())
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::attachment::ClientCapabilityLevel;
    use crate::config::DaemonConfig;
    use crate::local::{
        LaunchProviderRunRequest, LocalDaemonRequest, LocalDaemonResponse, SubmitPromptRequest,
    };
    use crate::session::CreateSessionRequest;

    #[test]
    fn prompt_settlement_action_uses_agent_runtime_projection_before_session_state() {
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
        let (session, agent) = app
            .create_session(CreateSessionRequest::new("workspace", "worktree"))
            .expect("session should be created");
        let session_id = session.id().to_string();
        let agent_id = agent.id().to_string();
        let attachment = app
            .attach(crate::attachment::AttachRequest::new(
                &session_id,
                "cli-flow-control-projection",
                ClientCapabilityLevel::FullTerminal,
            ))
            .expect("attachment should attach");
        app.handle_local_request(LocalDaemonRequest::LaunchProviderRun(
            LaunchProviderRunRequest {
                session_id: session_id.clone(),
                agent_id: Some(agent_id.clone()),
                adapter_key: "dev-stub".to_string(),
                provider: "claude-code".to_string(),
                account_profile: "default".to_string(),
                model: "sonnet".to_string(),
                variant: None,
            },
        ))
        .expect("provider run should launch");
        let provider_run_id = app
            .providers()
            .get_run_for_agent(&session_id, &agent_id)
            .expect("provider run should be registered")
            .id()
            .to_string();
        match app
            .handle_local_request(LocalDaemonRequest::SubmitPrompt(SubmitPromptRequest {
                session_id: session_id.clone(),
                attachment_id: attachment.id().to_string(),
                target_agent_id: Some(agent_id.clone()),
                prompt: "projected settlement".to_string(),
                attachments: Vec::new(),
            }))
            .expect("prompt should submit")
        {
            LocalDaemonResponse::PromptSubmitted { session, .. } => {
                app.update_session_projection(session);
            }
            _ => panic!("unexpected prompt response"),
        }
        app.sessions_mut()
            .complete_active_prompt_only(&session_id, &agent_id)
            .expect("session active prompt should be removed without refreshing projection");

        assert_eq!(
            prompt_settlement_action(&app, &session_id, &provider_run_id)
                .expect("settlement action should resolve"),
            PromptSettlementAction::Complete
        );
    }
}
