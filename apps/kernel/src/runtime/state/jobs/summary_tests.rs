use super::*;

async fn completed() -> (tests::Fixture, Record) {
    let fixture = tests::fixture().await;
    let (state, run, task, _) = &fixture;
    let value = state
        .dispatch_job_tool(
            Some(run),
            "chariox.jobs.run",
            &serde_json::json!({"command":["/bin/true"],"wait_s":5}),
            task,
        )
        .await
        .unwrap();
    let record = state
        .job_wake_driver
        .as_ref()
        .unwrap()
        .read(value["job_id"].as_str().unwrap())
        .unwrap();
    (fixture, record)
}

#[tokio::test]
async fn job_summary_lists_counts_without_returning_retained_output_or_paths() {
    let ((state, run, task, _workspace), mut record) = completed().await;
    let driver = state.job_wake_driver.as_ref().unwrap();
    let result = record.result.as_mut().unwrap();
    result.output_tail = "large retained output".repeat(4000);
    result.files_back = vec!["some/private/artifact".into(); 2000];
    result.conflicts = vec!["some/private/conflict".into(); 1000];
    driver.save(&record).unwrap();
    let rows = serde_json::to_value(state.session_jobs(&task.room_id)).unwrap();
    assert_eq!(rows[0]["result"]["files_back_count"], 2000);
    assert_eq!(rows[0]["result"]["conflicts_count"], 1000);
    assert!(rows[0]["result"].get("output_tail").is_none());
    assert!(serde_json::to_vec(&rows).unwrap().len() < 4096);
    let opened = state
        .dispatch_job_tool(
            Some(&run),
            "chariox.jobs.status",
            &serde_json::json!({"job_id":record.job_id}),
            &task,
        )
        .await
        .unwrap();
    assert_eq!(
        opened["result"]["output_tail"],
        record.result.as_ref().unwrap().output_tail
    );
    assert_eq!(
        opened["result"]["files_back"].as_array().unwrap().len(),
        2000
    );
}

#[tokio::test]
async fn job_summary_estimates_are_room_scoped_and_do_not_open_full_results() {
    let ((state, _run, task, _workspace), mut record) = completed().await;
    let driver = state.job_wake_driver.as_ref().unwrap();
    record.result.as_mut().unwrap().duration_ms = 1000;
    driver.save(&record).unwrap();
    let mut second = record.clone();
    second.job_id = "job-summary-second".into();
    second.started_at_ms += 1;
    second.result.as_mut().unwrap().duration_ms = 1200;
    driver.save(&second).unwrap();
    let mut other = second.clone();
    other.job_id = "job-summary-other-room".into();
    other.room_id = "another-room".into();
    other.started_at_ms += 1;
    other.result.as_mut().unwrap().duration_ms = 80000;
    driver.save(&other).unwrap();
    let mut next = record.clone();
    next.result = None;
    assert_eq!(state.job_duration_estimate(&next), Some(1100));
    // A result is deliberately unavailable: listing and admission use only its header.
    std::fs::remove_file(driver.path(&record.job_id).unwrap()).unwrap();
    assert_eq!(state.job_duration_estimate(&next), Some(1100));
    assert_eq!(state.session_jobs(&task.room_id).len(), 2);
}

#[tokio::test]
async fn job_summary_reports_unreadable_and_missing_results_without_losing_the_row() {
    let ((state, run, task, _workspace), record) = completed().await;
    let path = state
        .job_wake_driver
        .as_ref()
        .unwrap()
        .path(&record.job_id)
        .unwrap();
    assert_eq!(state.session_jobs(&task.room_id).len(), 1);
    std::fs::write(&path, b"corrupt result").unwrap();
    assert!(state
        .dispatch_job_tool(
            Some(&run),
            "chariox.jobs.status",
            &serde_json::json!({"job_id":record.job_id}),
            &task
        )
        .await
        .is_err());
    let rows = serde_json::to_value(state.session_jobs(&task.room_id)).unwrap();
    assert_eq!(rows.as_array().unwrap().len(), 1);
    assert!(rows[0]["read_error"]
        .as_str()
        .unwrap()
        .contains("unreadable"));
    std::fs::remove_file(&path).unwrap();
    assert!(state
        .dispatch_job_tool(
            Some(&run),
            "chariox.jobs.status",
            &serde_json::json!({"job_id":record.job_id}),
            &task
        )
        .await
        .is_err());
    let rows = serde_json::to_value(state.session_jobs(&task.room_id)).unwrap();
    assert!(rows[0]["read_error"].as_str().unwrap().contains("missing"));
}

#[tokio::test]
async fn job_summary_same_size_corruption_and_damaged_header_keep_the_owned_row() {
    let ((state, _run, task, _workspace), record) = completed().await;
    let path = state
        .job_wake_driver
        .as_ref()
        .unwrap()
        .path(&record.job_id)
        .unwrap();
    let size = std::fs::metadata(&path).unwrap().len() as usize;
    std::fs::write(&path, vec![b'x'; size]).unwrap();
    let rows = serde_json::to_value(state.session_jobs(&task.room_id)).unwrap();
    assert_eq!(rows[0]["job_id"], record.job_id);
    assert!(rows[0]["read_error"]
        .as_str()
        .unwrap()
        .contains("unreadable"));
    use sha2::{Digest, Sha256};
    let header = path
        .parent()
        .unwrap()
        .join("summaries")
        .join(format!("{:x}", Sha256::digest(task.room_id.as_bytes())))
        .join(&task.agent_id)
        .join(format!("{}.json", record.job_id));
    std::fs::write(header, b"broken header").unwrap();
    let rows = serde_json::to_value(state.session_jobs(&task.room_id)).unwrap();
    assert_eq!(rows[0]["agent_id"], task.agent_id);
    assert!(rows[0]["read_error"]
        .as_str()
        .unwrap()
        .contains("unreadable"));
}

#[tokio::test]
async fn job_summary_own_list_needs_no_run_and_excludes_peer_and_other_room_jobs() {
    let ((state, _run, task, _workspace), record) = completed().await;
    let driver = state.job_wake_driver.as_ref().unwrap();
    let mut peer = record.clone();
    peer.job_id = "job-peer-list".into();
    peer.agent_id = "peer-agent".into();
    driver.save(&peer).unwrap();
    let mut other = record.clone();
    other.job_id = "job-other-room-list".into();
    other.room_id = "other-room".into();
    driver.save(&other).unwrap();
    std::fs::remove_file(driver.path(&record.job_id).unwrap()).unwrap();
    let result = state
        .dispatch_job_tool(None, "chariox.jobs.list", &serde_json::json!({}), &task)
        .await
        .unwrap();
    let jobs = result["jobs"].as_array().unwrap();
    assert_eq!(jobs.len(), 1, "{result}");
    assert_eq!(jobs[0]["job_id"], record.job_id);
    assert!(jobs[0]["read_error"].as_str().unwrap().contains("missing"));
    assert!(result["next_action"]
        .as_str()
        .unwrap()
        .contains("chariox.jobs.status"));
}

#[tokio::test]
async fn job_summary_own_list_and_status_explain_memory_pressure_and_next_step() {
    let (state, run, task, workspace) = tests::fixture().await;
    let admission = crate::runtime::process_resources::LaunchAdmission::default();
    let busy = admission
        .reserve_with_memory(
            "other-job",
            &["cat".into()],
            workspace.path(),
            true,
            admission.capacity(),
        )
        .unwrap();
    let job = state
        .dispatch_job_tool(
            Some(&run),
            "chariox.jobs.run",
            &serde_json::json!({"command":["/bin/true"],"needs":{"memory_gib":1}}),
            &task,
        )
        .await
        .unwrap();
    let id = job["job_id"].as_str().unwrap();
    tokio::time::timeout(Duration::from_secs(5), async {
        while state
            .job_wake_driver
            .as_ref()
            .unwrap()
            .read(id)
            .unwrap()
            .state
            != "queued"
        {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    let status = state
        .dispatch_job_tool(
            None,
            "chariox.jobs.status",
            &serde_json::json!({"job_id":id}),
            &task,
        )
        .await
        .unwrap();
    let list = state
        .dispatch_job_tool(None, "chariox.jobs.list", &serde_json::json!({}), &task)
        .await;
    drop(busy);
    state
        .dispatch_job_tool(
            None,
            "chariox.jobs.cancel",
            &serde_json::json!({"job_id":id}),
            &task,
        )
        .await
        .unwrap();
    assert_eq!(status["waiting"]["reason"], "memory", "{status}");
    assert!(
        status["waiting"]["need_bytes"].as_u64().unwrap()
            > status["waiting"]["free_bytes"].as_u64().unwrap()
    );
    assert!(status["next_action"].as_str().unwrap().contains("Wait"));
    let list = list.unwrap();
    assert_eq!(list["waiting"][0]["job_id"], id, "{list}");
    assert_eq!(list["waiting"][0]["pressure"]["reason"], "memory");
}
