//! MP-08 / MP-09 / MP-10 / MP-11: a goal whose completion check ran out while its worker is silent.
use super::*;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_expired_goal_check_on_a_silent_worker_does_not_hold_the_sweep_of_unrelated_agents() {
    let f = Stalled::new(true).await;
    f.expire_child_goal_check();
    f.bystander();
    f.sweep_returns_and_supervises_the_bystander("the worker of a goal whose check ran out")
        .await;
    until(|| f.child_task_state() == ExecutionState::Blocked)
        .await
        .expect("the goal whose check ran out ends Blocked while its worker is still stopping");
}
