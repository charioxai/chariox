//! A local storage failure while a remote command runs loses no files: the job keeps its worker
//! and goes on once the storage is back.
use super::return_tests::{crashed_while_running, job_state_path, prepared, TWO_ARTIFACTS};
use super::*;

#[tokio::test]
async fn offload_job_keeps_its_files_when_its_output_log_cannot_be_written() {
    let mut job = prepared(
        "mkdir -p target; echo building; printf one > target/first.bin",
        &["target/*.bin"],
    )
    .await;
    job.outage
        .cut
        .store(false, std::sync::atomic::Ordering::SeqCst);
    // A full disk: every write to the output log fails with ENOSPC.
    let log = job_state_path(&job.state, &format!("{}/output.log", job.record.job_id));
    std::fs::remove_file(&log).unwrap();
    std::os::unix::fs::symlink("/dev/full", &log).unwrap();
    let release = super::cleanup_tests::service(
        &job.state,
        vec![
            ("job_started", 200, super::cleanup_tests::machine("ready")),
            ("get", 200, super::cleanup_tests::machine("deleted")),
        ],
    )
    .await;
    let result = tokio::time::timeout(Duration::from_secs(20), job.run())
        .await
        .expect("the job never ended")
        .unwrap();
    std::fs::remove_file(&log).unwrap();
    assert_eq!(
        result.outcome,
        JobOutcome::Completed,
        "{}",
        result.output_tail
    );
    assert_eq!(result.files_back, vec!["target/first.bin".to_string()]);
    assert_eq!(
        std::fs::read(job.workspace.path().join("target/first.bin")).unwrap(),
        b"one"
    );
    tokio::time::timeout(Duration::from_secs(10), release)
        .await
        .expect("the worker was not released once its files were home")
        .unwrap();
}

#[tokio::test]
async fn offload_job_after_a_restart_waits_for_its_output_log_instead_of_giving_up_its_files() {
    let crashed = crashed_while_running(
        TWO_ARTIFACTS,
        vec![
            ("job_started", 200, super::cleanup_tests::machine("ready")),
            ("get", 200, super::cleanup_tests::machine("deleted")),
        ],
    )
    .await;
    // The restarted kernel cannot open the output log again: a directory stands in its place.
    let log = job_state_path(
        &crashed.held.state,
        &format!("{}/output.log", crashed.held.job_id),
    );
    std::fs::remove_file(&log).unwrap();
    std::fs::create_dir(&log).unwrap();
    crashed.restart();
    tokio::time::sleep(Duration::from_millis(800)).await;
    let waiting = crashed.held.job_tool("chariox.jobs.status").await;
    assert_ne!(waiting["state"], "finished", "{waiting}");
    assert_eq!(waiting["deleted"], false);
    assert!(
        !crashed.service.is_finished(),
        "the worker was released while its output log could not be opened"
    );
    std::fs::remove_dir(&log).unwrap();
    let status = crashed.finished().await;
    crashed.service.await.unwrap();
    assert_eq!(
        status["result"]["outcome"], "completed",
        "{}",
        status["result"]["output_tail"]
    );
    assert_eq!(
        status["result"]["files_back"],
        serde_json::json!(["target/first.bin", "target/second.bin"])
    );
}
