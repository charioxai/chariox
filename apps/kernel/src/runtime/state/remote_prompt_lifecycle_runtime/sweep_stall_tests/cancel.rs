//! MP-08 / MP-09 / MP-10 / MP-11: a cancelled task whose leased worker does not answer.
use super::*;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_cancelled_task_on_a_silent_worker_does_not_hold_the_sweep_of_unrelated_agents() {
    let f = Stalled::new(true).await;
    f.cancel_child_task();
    f.bystander();
    f.sweep_returns_and_supervises_the_bystander("the worker of a cancelled task")
        .await;
    assert_eq!(
        f.child_task_state(),
        ExecutionState::Cancelled,
        "the cancellation stays supervised until the worker confirms the stop"
    );
}
