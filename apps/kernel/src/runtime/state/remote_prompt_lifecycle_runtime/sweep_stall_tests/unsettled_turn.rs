//! MP-08 / MP-09 / MP-10 / MP-11: one task whose settlement does not end holds only itself.
use super::*;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_task_whose_settlement_does_not_end_does_not_hold_the_sweep_of_unrelated_agents() {
    let f = Stalled::new(true).await;
    f.bystander();
    let fault = std::env::temp_dir().join(format!("sweep-task-stall-{}", f.child));
    std::fs::write(&fault, &f.child).unwrap();
    f.runtime
        .owned
        .agent_wakes
        .set_task_stall_fault(fault.clone());
    let swept = f
        .sweep_returns_and_supervises_the_bystander("a task whose settlement does not end")
        .await;
    std::fs::remove_file(&fault).unwrap();
    swept
}
