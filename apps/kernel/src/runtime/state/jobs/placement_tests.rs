//! Remote placement needs no kernel setting: a job the Room's option places remotely reaches
//! the machine service on a default kernel.
use super::*;

#[tokio::test]
async fn offload_job_on_a_default_kernel_asks_the_machine_service_and_raises_no_decision() {
    let (state, run, task, _worktree) = super::tests::fixture().await;
    state
        .set_remote_job_policy(
            &task.room_id,
            Some(&task.agent_id),
            &task.owner_user_id,
            Some(crate::local::OffloadPolicy::AlwaysHeavy),
        )
        .unwrap();
    let service = super::cleanup_tests::service(
        &state,
        vec![(
            "capabilities",
            200,
            serde_json::json!({"kind":"capabilities","machine_allocation":null}),
        )],
    )
    .await;
    let value = state
        .dispatch_job_tool(
            Some(&run),
            "chariox.jobs.run",
            &serde_json::json!({"command":["/bin/true"],"needs":{"cpus":2},"wait_s":5}),
            &task,
        )
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(5), service)
        .await
        .expect("the job never asked the machine service")
        .unwrap();
    assert_eq!(value["placement"], "remote", "{value}");
    assert!(state
        .owned
        .session_store
        .get_session(&task.room_id)
        .unwrap()
        .active_interactions()
        .is_empty());
}
