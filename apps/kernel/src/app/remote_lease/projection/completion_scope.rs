//! MP-08 / MP-10 / MP-11: exact worker turn outcomes, including native-origin turns.
use crate::{
    app::DaemonApp, execution_lease::LeasedAgent, terminal::AssistantMessageCompletionRecord,
    transport::relay_peer::RelayProjectedCompletion,
};

pub(super) fn project(
    app: &DaemonApp,
    leased: &LeasedAgent,
    run: &str,
    home_prompt: Option<&str>,
    backing_prompt: Option<&str>,
    record: AssistantMessageCompletionRecord,
) -> Option<RelayProjectedCompletion> {
    let prompt = record.prompt_id.as_deref().or(backing_prompt);
    let active_backing_prompt = app
        .prompt_owner_active_prompt_for_agent_snapshot(
            &leased.backing_session_id,
            &leased.backing_agent_id,
        )
        .ok()
        .flatten();
    if leased.active_home_prompt_id.is_some()
        && record.prompt_id.is_some()
        && active_backing_prompt
            .as_ref()
            .is_some_and(|active| prompt != Some(active.id()))
    {
        return None; // An old message cannot settle a successor's managed turn.
    }
    let binding = prompt.and_then(|id| {
        app.leased_workflow_turns.values().find(|binding| {
            binding.leased_agent_id == leased.id
                && binding.provider_run_id == run
                && binding.backing_prompt_id == id
        })
    });
    let mut completion = RelayProjectedCompletion {
        message_id: record.message_id,
        completed_at_ms: record.completed_at_ms,
        home_prompt_id: binding
            .map(|binding| binding.home_prompt_id.clone())
            .or_else(|| {
                if leased.active_home_prompt_id.is_some() {
                    home_prompt.map(str::to_string)
                } else {
                    record
                        .prompt_id
                        .clone()
                        .or_else(|| home_prompt.map(str::to_string))
                }
            }),
        settlement_status: None,
        provider_termination: None,
    };
    if let Some(turn) = prompt
        .and_then(|id| {
            app.completed_git_turn_snapshot_store()
                .projection_for_prompt(&leased.backing_session_id, &leased.backing_agent_id, id)
        })
        .filter(|turn| turn.provider_run_id == run)
    {
        completion.settlement_status = Some(turn.settlement_status);
        completion.provider_termination = turn.provider_termination;
    }
    Some(completion)
}

pub(super) fn refresh(
    app: &DaemonApp,
    leased: &LeasedAgent,
    run: &str,
    completion: &mut RelayProjectedCompletion,
) {
    let Some(home_prompt) = completion.home_prompt_id.as_deref() else {
        return;
    };
    let backing_prompt = app
        .leased_workflow_turns
        .values()
        .find(|binding| {
            binding.leased_agent_id == leased.id
                && binding.provider_run_id == run
                && binding.home_prompt_id == home_prompt
        })
        .map_or(home_prompt, |binding| binding.backing_prompt_id.as_str());
    if let Some(turn) = app
        .completed_git_turn_snapshot_store()
        .projection_for_prompt(
            &leased.backing_session_id,
            &leased.backing_agent_id,
            backing_prompt,
        )
        .filter(|turn| turn.provider_run_id == run)
    {
        completion.settlement_status = Some(turn.settlement_status);
        completion.provider_termination = turn.provider_termination;
    }
}
