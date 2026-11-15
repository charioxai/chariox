//! Every stage of a remote job's worker against every event that can reach it.
//!
//! | stage    | restart                 | outage / worker loss              | status              | cancel                 |
//! |----------|-------------------------|-----------------------------------|---------------------|------------------------|
//! | Idle     | releases the worker     | prepare fails, worker released    | nothing             | worker released        |
//! | Planned  | stops and reaps it      | open fails, worker reaped         | nothing             | handled once opened    |
//! | Running  | reattaches, waits       | waits, then reaps after patience  | reports, no collect | stops it, files home   |
//! | Finished | keeps worker, collects  | worker kept, retried on status    | collects, releases  | gives up, releases     |
//! | Settled  | releases the worker     | release retried on status         | releases            | releases               |
use super::return_tests::*;
use super::*;
use std::sync::atomic::Ordering::SeqCst;

const STAGES: [fn() -> RemoteStage; 5] = [
    || RemoteStage::Idle,
    || RemoteStage::Planned,
    || RemoteStage::Running {
        room: "room".into(),
        exec_id: "exec".into(),
    },
    || RemoteStage::Finished {
        outcome: JobOutcome::Completed,
    },
    || RemoteStage::Settled,
];

fn events() -> [StageEvent; 5] {
    [
        StageEvent::Plan,
        StageEvent::Open {
            room: "room".into(),
            exec_id: "exec".into(),
        },
        StageEvent::End(JobOutcome::Completed),
        StageEvent::Home,
        StageEvent::GiveUp,
    ]
}

#[test]
fn a_remote_job_moves_only_along_its_lifecycle() {
    // Rows: Idle, Planned, Running, Finished, Settled. Columns: Plan, Open, End, Home, GiveUp.
    let allowed: [[Option<usize>; 5]; 5] = [
        [Some(1), None, None, None, None],
        [None, Some(2), None, None, Some(4)],
        [None, None, Some(3), None, Some(4)],
        [None, None, None, Some(4), Some(4)],
        [None, None, None, None, None],
    ];
    for (row, stage) in STAGES.iter().enumerate() {
        for (column, event) in events().into_iter().enumerate() {
            let next = stage().next(event);
            match allowed[row][column] {
                Some(to) => {
                    let expected = STAGES[to]();
                    // End carries its outcome and Open its identity, so compare by variant.
                    assert_eq!(
                        std::mem::discriminant(&next.unwrap()),
                        std::mem::discriminant(&expected),
                        "stage {row} event {column}"
                    );
                }
                None => assert!(next.is_err(), "stage {row} must refuse event {column}"),
            }
        }
    }
}

#[test]
fn only_a_finished_command_may_have_its_files_collected_and_only_a_settled_worker_released() {
    for (row, stage) in STAGES.iter().enumerate() {
        let stage = stage();
        assert_eq!(stage.holds_worker(), (1..=3).contains(&row));
        assert_eq!(stage.awaits_command(), (1..=2).contains(&row));
    }
}

/// The job's record as `prepared` made it is what the kernel last saved.
pub(super) fn held_of(job: Prepared) -> HeldWorker {
    job.state
        .job_wake_driver
        .as_ref()
        .unwrap()
        .save(&job.record)
        .unwrap();
    HeldWorker {
        state: job.state,
        run: job.run,
        task: job.task,
        workspace: job.workspace,
        _home: job.home,
        outage: job.outage,
        job_id: job.record.job_id,
    }
}

fn save(held: &HeldWorker, change: impl FnOnce(&mut Record)) {
    let driver = held.state.job_wake_driver.as_ref().unwrap();
    let mut record = driver.read(&held.job_id).unwrap();
    change(&mut record);
    driver.save(&record).unwrap();
}

fn record(held: &HeldWorker) -> Record {
    held.state
        .job_wake_driver
        .as_ref()
        .unwrap()
        .read(&held.job_id)
        .unwrap()
}

async fn recorded_finished(held: &HeldWorker) -> Record {
    tokio::time::timeout(Duration::from_secs(20), async {
        loop {
            let record = record(held);
            if record.state == "finished" {
                return record;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .expect("the job never finished")
}

fn plan_exists(held: &HeldWorker) -> bool {
    job_state_path(&held.state, &format!("{}/return.json", held.job_id)).exists()
}

/// A live job of this stage: its status call is answered without touching the worker or files.
async fn status_leaves_a_live_job_alone(stage: RemoteStage) {
    let job = prepared("true", &[]).await;
    let held = held_of(job);
    save(&held, |record| {
        record.state = "running".into();
        record.remote = stage;
    });
    let service = release_service(&held.state).await;
    let status = held.job_tool("chariox.jobs.status").await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(
        !service.is_finished(),
        "status released a live job's worker"
    );
    service.abort();
    assert_eq!(status["deleted"], false);
    assert_eq!(status["state"], "running");
    assert!(!held.workspace.path().join("target").exists());
}

#[tokio::test]
async fn offload_job_idle_status_leaves_the_worker_alone() {
    status_leaves_a_live_job_alone(RemoteStage::Idle).await;
}

#[tokio::test]
async fn offload_job_planned_status_leaves_the_worker_alone() {
    status_leaves_a_live_job_alone(RemoteStage::Planned).await;
}

#[tokio::test]
async fn offload_job_running_status_leaves_the_worker_and_files_alone() {
    status_leaves_a_live_job_alone(RemoteStage::Running {
        room: "room".into(),
        exec_id: "exec".into(),
    })
    .await;
}

#[tokio::test]
async fn offload_job_idle_restart_releases_the_worker() {
    let held = held_of(prepared("true", &[]).await);
    let service = release_service(&held.state).await;
    recover_job(&held.state, &held.job_id);
    let record = recorded_finished(&held).await;
    service.await.unwrap();
    assert!(record.deleted);
    assert_eq!(record.result.unwrap().outcome, JobOutcome::MachineLost);
}

#[tokio::test]
async fn offload_job_idle_worker_loss_releases_the_worker() {
    let mut job = prepared("true", &[]).await;
    job.outage.down.store(true, SeqCst);
    let service = release_service(&job.state).await;
    let result = job.run().await.unwrap();
    service.await.unwrap();
    assert!(job.record.deleted);
    assert_eq!(result.outcome, JobOutcome::MachineLost);
    assert_eq!(job.record.remote, RemoteStage::Idle);
}

#[tokio::test]
async fn offload_job_planned_restart_stops_and_reaps_the_worker() {
    let held = held_of(prepared("true", &[]).await);
    save(&held, |record| {
        record.remote = RemoteStage::Planned;
    });
    std::fs::create_dir_all(job_state_path(&held.state, &held.job_id)).unwrap();
    std::fs::write(
        job_state_path(&held.state, &format!("{}/return.json", held.job_id)),
        b"{}",
    )
    .unwrap();
    let service = release_service(&held.state).await;
    recover_job(&held.state, &held.job_id);
    let record = recorded_finished(&held).await;
    service.await.unwrap();
    assert!(record.deleted);
    assert_eq!(record.remote, RemoteStage::Settled);
    assert!(!plan_exists(&held));
}

#[tokio::test]
async fn offload_job_planned_open_failure_reaps_the_worker_without_running_the_command() {
    let marker = crate::test_support::TestWorktree::new("planned-open-failure");
    let ran = marker.path().join("ran");
    let mut job = prepared(&format!("printf ran > {ran:?}"), &[]).await;
    job.outage.open_down.store(true, SeqCst);
    let service = release_service(&job.state).await;
    let result = job.run().await.unwrap();
    service.await.unwrap();
    assert!(job.record.deleted);
    assert_eq!(job.record.remote, RemoteStage::Settled);
    assert_eq!(result.outcome, JobOutcome::MachineLost);
    assert!(!ran.exists());
    assert!(!job_state_path(&job.state, &format!("{}/return.json", job.record.job_id)).exists());
}

fn two_gets(state: &str) -> Vec<(&'static str, u16, serde_json::Value)> {
    vec![
        ("job_started", 200, super::cleanup_tests::machine("ready")),
        ("get", 200, super::cleanup_tests::machine(state)),
    ]
}

#[tokio::test]
async fn offload_job_running_cancel_after_a_restart_stops_the_command_and_brings_its_files_home() {
    let crashed = crashed_while_running(TWO_ARTIFACTS, two_gets("deleted")).await;
    crashed.restart();
    tokio::time::sleep(Duration::from_millis(700)).await;
    let driver = crashed.held.state.job_wake_driver.as_ref().unwrap();
    let wake_id = record(&crashed.held).wake_id;
    driver.live.lock().unwrap()[&wake_id].send_replace(true);
    let record = recorded_finished(&crashed.held).await;
    crashed.service.await.unwrap();
    let result = record.result.unwrap();
    assert_eq!(result.outcome, JobOutcome::Cancelled);
    assert_eq!(result.files_back, ["target/first.bin"]);
    assert!(record.deleted);
    assert!(!crashed
        .held
        .workspace
        .path()
        .join("target/second.bin")
        .exists());
}

#[tokio::test]
async fn offload_job_running_short_outage_waits_for_the_command_and_loses_nothing() {
    let crashed = crashed_while_running(TWO_ARTIFACTS, two_gets("deleted")).await;
    crashed.held.outage.down.store(true, SeqCst);
    crashed.restart();
    tokio::time::sleep(Duration::from_secs(1)).await;
    assert!(!crashed.service.is_finished());
    crashed.held.outage.down.store(false, SeqCst);
    let record = recorded_finished(&crashed.held).await;
    crashed.service.await.unwrap();
    assert!(record.deleted);
    assert_eq!(
        record.result.unwrap().files_back,
        ["target/first.bin", "target/second.bin"]
    );
}

#[tokio::test]
async fn offload_job_running_long_outage_reaps_the_worker_and_accepts_no_file() {
    let crashed = crashed_while_running(TWO_ARTIFACTS, two_gets("deleted")).await;
    crashed.held.outage.down.store(true, SeqCst);
    crashed.restart();
    let record = recorded_finished(&crashed.held).await;
    crashed.service.await.unwrap();
    assert!(record.deleted);
    assert_eq!(record.remote, RemoteStage::Settled);
    assert_eq!(record.result.unwrap().outcome, JobOutcome::MachineLost);
    assert!(!crashed.held.workspace.path().join("target").exists());
}

#[tokio::test]
async fn offload_job_running_worker_loss_after_a_restart_reaps_the_worker_and_accepts_no_file() {
    let mut responses = vec![("job_started", 200, super::cleanup_tests::machine("ready"))];
    // The reattachment asks the service three times (now, 2 s and 4 s later) before its patience
    // ends, then the release asks once more.
    responses.extend((0..4).map(|_| ("get", 200, super::cleanup_tests::machine("deleted"))));
    let crashed = crashed_while_running(TWO_ARTIFACTS, responses).await;
    crashed
        .held
        .state
        .job_wake_driver
        .as_ref()
        .unwrap()
        .return_grants
        .lock()
        .unwrap()
        .remove(&crashed.held.job_id);
    crashed.restart();
    let record = recorded_finished(&crashed.held).await;
    tokio::time::timeout(Duration::from_secs(5), crashed.service)
        .await
        .expect("the service was not asked to reattach the worker")
        .unwrap();
    assert!(record.deleted);
    assert_eq!(record.result.unwrap().outcome, JobOutcome::MachineLost);
    assert!(!crashed.held.workspace.path().join("target").exists());
}

#[tokio::test]
async fn offload_job_finished_restart_keeps_the_worker_until_status_collects_its_files() {
    let held = worker_held_after_failed_return().await;
    save(&held, |record| {
        record.state = "returning".into();
        record.result = None;
    });
    let service = release_service(&held.state).await;
    recover_job(&held.state, &held.job_id);
    let record = recorded_finished(&held).await;
    assert!(!record.deleted && record.remote.holds_worker());
    assert_eq!(record.result.unwrap().outcome, JobOutcome::OutsideEnd);
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(!service.is_finished());
    held.outage.cut.store(false, SeqCst);
    let status = held.job_tool("chariox.jobs.status").await;
    service.await.unwrap();
    assert_eq!(status["deleted"], true);
    assert_eq!(status["result"]["outcome"], "completed");
    assert_eq!(
        status["result"]["files_back"],
        serde_json::json!(["target/first.bin", "target/second.bin"])
    );
}

async fn settled_worker() -> HeldWorker {
    let held = held_of(prepared("true", &[]).await);
    save(&held, |record| {
        record.remote = RemoteStage::Settled;
        record.state = "finished".into();
        record.result = Some(JobResult {
            exit_code: Some(0),
            output_tail: String::new(),
            log: String::new(),
            files_back: vec![],
            conflicts: vec![],
            ran_on: JobRanOn {
                machine: "offload-test".into(),
                ip: None,
            },
            duration_ms: 1,
            outcome: JobOutcome::Completed,
        });
    });
    held
}

#[tokio::test]
async fn offload_job_settled_restart_releases_the_worker() {
    let held = held_of(prepared("true", &[]).await);
    save(&held, |record| record.remote = RemoteStage::Settled);
    let service = release_service(&held.state).await;
    recover_job(&held.state, &held.job_id);
    let record = recorded_finished(&held).await;
    service.await.unwrap();
    assert!(record.deleted);
}

#[tokio::test]
async fn offload_job_settled_status_and_cancel_release_the_worker() {
    for name in ["chariox.jobs.status", "chariox.jobs.cancel"] {
        let held = settled_worker().await;
        let service = release_service(&held.state).await;
        let status = held.job_tool(name).await;
        service.await.unwrap();
        assert_eq!(status["deleted"], true, "{name}");
    }
}

#[tokio::test]
async fn offload_job_settled_status_stays_answerable_while_the_release_fails() {
    let held = settled_worker().await;
    let service = super::cleanup_tests::service(
        &held.state,
        vec![(
            "get",
            503,
            serde_json::json!({"error":{"code":"dependency_unavailable"}}),
        )],
    )
    .await;
    let status = held.job_tool("chariox.jobs.status").await;
    service.abort();
    assert_eq!(status["deleted"], false);
    assert_eq!(record(&held).remote, RemoteStage::Settled);
}
