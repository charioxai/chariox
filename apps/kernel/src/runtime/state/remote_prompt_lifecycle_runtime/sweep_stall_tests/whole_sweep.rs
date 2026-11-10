//! MP-08 / MP-09 / MP-10 / MP-11: the sweep as a whole, with every await that used to hold it stuck.
use super::*;

/// With nothing to wait for the sweep takes well under a second.
const WHOLE_SWEEP_BOUND: std::time::Duration = std::time::Duration::from_secs(5);

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_sweep_returns_while_a_goal_check_a_workflow_a_cancellation_and_a_peer_follow_are_stuck(
) {
    let f = Stalled::new(true).await;
    // The child's task is at once a goal whose check ran out and a task of a failed workflow run;
    // stopping it, either way, asks the worker, which never answers.
    f.fail_workflow_run_of_child_task();
    f.expire_child_goal_check();
    let _directory = f.follow_a_peer_while_the_cloud_directory_is_silent();
    f.bystander();
    tokio::time::timeout(
        WHOLE_SWEEP_BOUND,
        f.runtime.sweep_agent_lifecycle_for_test(),
    )
    .await
    .expect("the sweep waits on a worker, a peer follow or Cloud")
    .unwrap();
    assert_eq!(f.bystander_status(), "cancelled");
}
