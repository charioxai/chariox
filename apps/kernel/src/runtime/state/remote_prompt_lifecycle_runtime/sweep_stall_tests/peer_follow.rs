//! MP-08 / MP-09 / MP-10 / MP-11: follows of peers on other kernels while Cloud does not answer.
use super::*;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_silent_kernel_directory_does_not_hold_the_sweep_of_unrelated_agents() {
    let f = Stalled::new(true).await;
    let _directory = f.follow_a_peer_while_the_cloud_directory_is_silent();
    f.bystander();
    f.sweep_returns_and_supervises_the_bystander("Cloud's kernel directory")
        .await;
}
