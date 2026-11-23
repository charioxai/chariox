//! Remote jobs are off unless the kernel starts with them on: a job runs on the local queue and
//! nothing reaches a worker machine or the machine service.
use super::*;

/// The machine service answers its first call with a response this test never sends: the
/// handle ends only if the kernel called it.
async fn unreachable_service(state: &KernelRuntimeState) -> tokio::task::JoinHandle<()> {
    super::cleanup_tests::service(state, vec![("acquire", 200, serde_json::Value::Null)]).await
}

#[tokio::test]
async fn offload_job_with_remote_jobs_off_runs_a_heavy_job_locally_and_never_calls_the_machine_service(
) {
    let (state, run, task, _worktree) = super::tests::fixture().await;
    state
        .set_remote_job_policy(
            &task.room_id,
            Some(&task.agent_id),
            false,
            &task.owner_user_id,
            Some(crate::local::OffloadPolicy::AlwaysHeavy),
        )
        .unwrap();
    let service = unreachable_service(&state).await;
    let value = state
        .dispatch_job_tool(
            Some(&run),
            "chariox.jobs.run",
            &serde_json::json!({"command":["/bin/true"],"needs":{"cpus":2},"wait_s":5}),
            &task,
        )
        .await
        .unwrap();
    assert_eq!(value["placement"], "local", "{value}");
    assert_eq!(value["result"]["outcome"], "completed");
    assert_eq!(value["result"]["exit_code"], 0);
    assert!(
        !service.is_finished(),
        "a local job called the machine service"
    );
    service.abort();
}

#[tokio::test]
async fn offload_job_with_remote_jobs_off_refuses_to_move_a_queued_job_to_a_machine() {
    let (state, _run, task, _worktree) = super::tests::fixture().await;
    let owner = crate::runtime::command::KernelCaller {
        connection_class: Some(crate::local::KernelConnectionClass::Terminal),
        user_id: Some(task.owner_user_id.clone()),
        ..crate::runtime::command::KernelCaller::default()
    };
    let error = state
        .run_session_process_remotely(&task.room_id, &owner, "job-queued", None)
        .await
        .unwrap_err();
    assert!(error.to_string().contains("switched off"), "{error}");
}

#[tokio::test]
async fn offload_job_left_remote_by_an_earlier_run_ends_with_its_reason_when_remote_jobs_are_off() {
    let crashed = super::return_tests::crashed_while_running(
        super::return_tests::TWO_ARTIFACTS,
        vec![
            ("job_started", 200, super::cleanup_tests::machine("ready")),
            ("get", 200, super::cleanup_tests::machine("deleted")),
        ],
    )
    .await;
    let state = &crashed.held.state;
    let mut config = state.owned.config_projection.snapshot();
    config.remote_jobs = false;
    state.owned.config_projection.update(config);
    crashed.restart();
    let status = crashed.finished().await;
    assert_eq!(status["result"]["outcome"], "machine_lost");
    assert!(status["result"]["output_tail"]
        .as_str()
        .unwrap()
        .contains("remote jobs are switched off"));
    // The saved worker stays as the earlier run left it, through later status calls too.
    let again = crashed.held.job_tool("chariox.jobs.status").await;
    assert_eq!(again["deleted"], false);
    assert_eq!(again["return_pending"], true);
    tokio::time::sleep(Duration::from_secs(4)).await;
    assert!(
        !crashed
            .held
            .workspace
            .path()
            .join("target/first.bin")
            .exists(),
        "a file was collected from a worker with remote jobs off"
    );
    assert!(
        !crashed.service.is_finished(),
        "the worker or its allocation was touched with remote jobs off"
    );
    crashed.service.abort();
}
