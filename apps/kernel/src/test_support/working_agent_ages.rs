//! A working agent whose prompt, turn and provider output sit at chosen ages,
//! to read what Health says about its silence.
use super::*;

pub(crate) const MINUTE_MS: u64 = 60 * 1000;

pub(crate) struct WorkingAgentAges {
    pub prompt_queued_ms_ago: u64,
    pub turn_started_ms_ago: u64,
    pub last_output_ms_ago: u64,
}

/// Gives `run` (already in the provider store) the stated output age, publishes
/// that stale copy to the run projection, and makes the agent's prompt and
/// admitted turn the stated ages.
pub(crate) fn age_working_agent(
    app: &mut DaemonApp,
    run: crate::provider::RuntimeProviderRun,
    ages: WorkingAgentAges,
) {
    let now = crate::session::unix_epoch_ms();
    let session_id = run.session_id().to_string();
    let agent_id = run
        .agent_instance_id()
        .expect("the run belongs to an agent")
        .to_string();
    let run = run.with_last_output_at_ms_for_test(now - ages.last_output_ms_ago);
    app.providers_mut().insert_run_for_test(run.clone());
    app.update_provider_run_projection(run.clone());

    let mut prompt = serde_json::to_value(crate::session::PromptQueueItem::new(
        "aged-prompt",
        "attachment",
        &agent_id,
        "work",
        crate::session::PromptStatus::Running,
    ))
    .unwrap();
    prompt["created_at_ms"] = (now - ages.prompt_queued_ms_ago).into();
    let prompt = serde_json::from_value::<crate::session::PromptQueueItem>(prompt).unwrap();
    app.agent_runtime_projection_store()
        .update_agent_prompt_state(&session_id, &agent_id, Some(prompt), None, 0);

    let mut turn = crate::app::ActiveTurnState::new(
        session_id,
        agent_id,
        "aged-prompt".to_string(),
        run.id().to_string(),
    );
    turn.started_at_ms = now - ages.turn_started_ms_ago;
    app.active_turns.start(turn);
}
