// MP-08 / MP-10 / MP-11 A10-S04: a turn whose workspace was removed ends failed.
use super::automatic_substitutes::failing_turn;
use crate::durable_state::agent_lifecycle::ExecutionState;

#[tokio::test]
async fn a_turn_whose_workspace_is_removed_ends_failed_and_names_the_directory() {
    // A configured substitute must not take over a turn whose workspace is gone:
    // it would run in the same missing directory.
    let turn = failing_turn(&[("dev-stub", "workspace-substitute", None)]).await;
    turn.run_in_worktree().await;
    let mut config = turn.runtime.owned.config_projection.snapshot();
    config.room_agent_tools = true;
    turn.runtime.owned.config_projection.update(config);
    let git = turn.worktree.join(".git");
    std::fs::create_dir(&git).unwrap();

    turn.runtime.sweep_agent_lifecycle_for_test().await.unwrap();
    assert!(
        turn.active_prompt().is_some(),
        "an intact workspace changes nothing"
    );

    std::fs::remove_dir_all(&git).unwrap();
    turn.runtime.sweep_agent_lifecycle_for_test().await.unwrap();

    assert!(
        turn.active_prompt().is_none(),
        "the turn on a removed workspace kept running"
    );
    assert!(
        turn.rerun_notices().is_empty(),
        "a substitute took over the turn: {:?}",
        turn.rerun_notices()
    );
    let task = turn
        .runtime
        .owned
        .durable_state_store
        .agent_tasks(Some(&turn.session_id), Some(&turn.agent_id))
        .unwrap()
        .into_iter()
        .find(|task| task.prompt_id == turn.prompt_id)
        .expect("the turn has a task");
    assert_eq!(task.state, ExecutionState::Cancelled, "{}", task.reason);
    assert!(
        task.reason.contains(&turn.worktree.display().to_string()),
        "the failure names the directory: {}",
        task.reason
    );
}
