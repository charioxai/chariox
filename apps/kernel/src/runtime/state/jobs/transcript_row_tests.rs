use super::tests::fixture;
use super::transcript_row::{is_job_launch_snapshot, job_row};
use super::*;
use crate::terminal::TerminalOutputKind;

fn row(
    state: &KernelRuntimeState,
    task: &AgentTaskExecution,
    job_id: &str,
) -> (String, serde_json::Value) {
    let (key, run, row) = state
        .owned
        .latest_kernel_tool_row(&task.room_id, &task.agent_id, &format!("job:{job_id}"))
        .expect("the job has a transcript row");
    assert_eq!(key, format!("job:{job_id}"));
    (run, row)
}

async fn finished(state: &KernelRuntimeState, job_id: &str) {
    for _ in 0..200 {
        if state.job_driver().unwrap().read(job_id).unwrap().state == "finished" {
            return;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("the job did not finish");
}

#[tokio::test]
async fn a_job_is_one_tool_row_of_its_turn_that_opens_to_its_command_and_output() {
    let (state, run, task, _worktree) = fixture().await;
    let result = state
        .dispatch_job_tool(
            Some(&run),
            "chariox.jobs.run",
            &serde_json::json!({"command":"printf 'built\\n'","wait_s":5}),
            &task,
        )
        .await
        .unwrap();
    let id = result["job_id"].as_str().unwrap();
    finished(&state, id).await;
    let (row_run, row) = row(&state, &task, id);
    assert_eq!(
        row_run,
        run.id(),
        "written in the turn that started the job"
    );
    assert_eq!(row["tool"], "chariox.jobs.run");
    assert_eq!(row["status"], "completed");
    assert_eq!(
        row["input"]["command"], "printf 'built\\n'",
        "the command as the agent gave it"
    );
    assert!(row["output"].as_str().unwrap().contains("built"), "{row}");
    assert!(
        row.get("background").is_none(),
        "a job ends with the job, not with a provider process"
    );
    assert!(row["finished_at_ms"].as_u64().unwrap() >= row["started_at_ms"].as_u64().unwrap());
    assert_eq!(row["text"], "exit 0");
}

#[tokio::test]
async fn a_running_job_row_runs_until_the_job_ends_and_a_cancelled_one_says_so() {
    let (state, run, task, _worktree) = fixture().await;
    let result = state
        .dispatch_job_tool(
            Some(&run),
            "chariox.jobs.run",
            &serde_json::json!({"command":"printf 'started\\n'; sleep 30"}),
            &task,
        )
        .await
        .unwrap();
    let id = result["job_id"].as_str().unwrap();
    let (_, running) = row(&state, &task, id);
    assert_eq!(
        running["status"], "running",
        "the call returned; the job still runs"
    );
    assert!(running["finished_at_ms"].is_null());
    state
        .dispatch_job_tool(
            Some(&run),
            "chariox.jobs.cancel",
            &serde_json::json!({"job_id": id}),
            &task,
        )
        .await
        .unwrap();
    finished(&state, id).await;
    let (_, cancelled) = row(&state, &task, id);
    assert_eq!(cancelled["status"], "cancelled", "{cancelled}");
    assert!(cancelled["finished_at_ms"].is_u64());
}

#[tokio::test]
async fn a_failed_job_row_is_failed_and_opens_to_its_error_output() {
    let (state, run, task, _worktree) = fixture().await;
    let result = state
        .dispatch_job_tool(
            Some(&run),
            "chariox.jobs.run",
            &serde_json::json!({"command":"echo broke >&2; exit 3","wait_s":5}),
            &task,
        )
        .await
        .unwrap();
    let id = result["job_id"].as_str().unwrap();
    finished(&state, id).await;
    let (_, row) = row(&state, &task, id);
    assert_eq!(row["status"], "error");
    assert!(row["error"].as_str().unwrap().contains("broke"), "{row}");
    assert_eq!(row["text"], "exit 3");
}

#[tokio::test]
async fn a_remote_job_row_names_its_machine_exit_and_files_back_inside_the_row() {
    let (state, run, task, _worktree) = fixture().await;
    let result = state
        .dispatch_job_tool(
            Some(&run),
            "chariox.jobs.run",
            &serde_json::json!({"command":"true","wait_s":5}),
            &task,
        )
        .await
        .unwrap();
    let id = result["job_id"].as_str().unwrap();
    finished(&state, id).await;
    let mut record = state.job_driver().unwrap().read(id).unwrap();
    record.placement = JobPlacement::Remote;
    record.machine_label = Some("nelpx-6wek3".into());
    record.result.as_mut().unwrap().files_back = vec!["out.txt".into()];
    let row = job_row(&record);
    assert_eq!(row["text"], "On nelpx-6wek3 · exit 0 · 1 file back");
    assert!(!row.to_string().contains("Ran on"));
    record.state = "running".into();
    assert_eq!(job_row(&record)["status"], "running");
}

#[test]
fn the_providers_row_of_a_call_that_started_a_job_is_left_out() {
    let snapshot = |value: serde_json::Value| {
        is_job_launch_snapshot(
            &TerminalOutputKind::ProviderTool,
            value.to_string().as_bytes(),
        )
    };
    let started = format!(
        "{{\"job_id\":\"job-{}\",\"state\":\"running\"}}",
        "0a".repeat(16)
    );
    assert!(snapshot(
        serde_json::json!({"tool":"mcp__chariox__chariox_jobs_run","status":"running","input":{"command":"make"}})
    ));
    assert!(snapshot(
        serde_json::json!({"tool":"mcp__chariox__chariox_jobs_run","status":"completed","output":started})
    ));
    assert!(
        snapshot(serde_json::json!({"tool":"chariox_jobs_run","status":"inProgress"})),
        "Codex names it with underscores"
    );
    assert!(snapshot(
        serde_json::json!({"tool":"chariox.jobs.run","status":"completed","output":{"content":[{"type":"text","text":started}]}})
    ));
    assert!(
        !snapshot(
            serde_json::json!({"tool":"mcp__chariox__chariox_jobs_run","status":"error","error":"Invalid job request"})
        ),
        "a refused call keeps its row"
    );
    assert!(!snapshot(
        serde_json::json!({"tool":"mcp__chariox__chariox_jobs_status","status":"running"})
    ));
    assert!(!snapshot(
        serde_json::json!({"tool":"Bash","status":"running","input":{"command":"make"}})
    ));
    assert!(!is_job_launch_snapshot(
        &TerminalOutputKind::ProviderOutput,
        b"{\"tool\":\"chariox.jobs.run\"}"
    ));
}
