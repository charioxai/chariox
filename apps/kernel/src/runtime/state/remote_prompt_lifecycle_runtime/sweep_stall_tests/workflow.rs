//! MP-08 / MP-09 / MP-10 / MP-11: a failed workflow run whose task sits on a silent worker.
use super::*;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_failed_workflow_with_a_task_on_a_silent_worker_does_not_hold_the_sweep() {
    let f = Stalled::new(true).await;
    f.fail_workflow_run_of_child_task();
    f.bystander();
    f.sweep_returns_and_supervises_the_bystander("the worker of a failed workflow's task")
        .await;
    until(|| f.child_task_state() == ExecutionState::Cancelled)
        .await
        .expect("the failed run's task is cancelled while its worker is still stopping");
}
