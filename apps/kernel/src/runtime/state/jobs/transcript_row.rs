//! A job is one tool row of its agent's transcript, like any other call: the kernel writes it
//! in the turn that started the job and rewrites that same row as the job runs and ends, so
//! every client shows the job in place from history. The provider's own row of the launching
//! `chariox.jobs.run` call is left out: it would be a second row for the same command.
use super::*;
use crate::local::JobCommand;
use crate::terminal::TerminalOutputKind;

const TOOL: &str = "chariox.jobs.run";
/// What a row opens to: the end of the command's output, as much as a row keeps.
const OUTPUT_CHARS: usize = 16 * 1024;

/// Writing a row and rewriting it are one decision about the job at a time: a change that lands
/// while the row is first written is found by the writer, or finds the row.
static JOB_ROWS: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn job_rows_guard() -> std::sync::MutexGuard<'static, ()> {
    JOB_ROWS
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn row_key(job_id: &str) -> String {
    format!("{}{job_id}", crate::app::JOB_ROW_KEY_PREFIX)
}

impl KernelRuntimeState {
    /// The row of a job whose wake is armed, written in `run`, the turn that started it.
    pub(super) fn write_job_row(&self, job_id: &str, run: Option<&str>) {
        let Some(run) = run else {
            return;
        };
        let _rows = job_rows_guard();
        let Ok(record) = self.job_driver().and_then(|driver| driver.read(job_id)) else {
            return;
        };
        self.owned.write_kernel_tool_row(
            &record.room_id,
            &record.agent_id,
            run,
            &row_key(job_id),
            job_row(&record),
        );
    }
}

/// Rewrites the job's row in place when what it shows changed. A job without a row (its wake is
/// not armed yet, or it runs for an agent of another kernel) gets none here.
pub(super) fn update_job_row(state: &KernelRuntimeState, record: &Record) {
    let _rows = job_rows_guard();
    let Some((key, run, current)) = state.owned.latest_kernel_tool_row(
        &record.room_id,
        &record.agent_id,
        &row_key(&record.job_id),
    ) else {
        return;
    };
    let next = job_row(record);
    if current != next {
        state
            .owned
            .write_kernel_tool_row(&record.room_id, &record.agent_id, &run, &key, next);
    }
}

/// The row: the command as the agent gave it, running until the job ends; then its end, where it
/// ran, its exit and the files it brought back, and the end of its output.
pub(super) fn job_row(record: &Record) -> serde_json::Value {
    let command = match &record.request.command {
        JobCommand::Shell(command) => command.clone(),
        JobCommand::Argv(argv) => crate::runtime::process_resources::command_line(argv),
    };
    let mut row = serde_json::json!({
        "tool": TOOL, "id": row_key(&record.job_id), "status": "running",
        "input": {"command": command},
        "started_at_ms": record.started_at_ms, "finished_at_ms": null,
    });
    if let Some(machine) = &record.machine_label {
        row["text"] = format!("On {machine}").into();
    }
    if record.state != "finished" {
        return row;
    }
    let Some(result) = &record.result else {
        row["status"] = "error".into();
        row["finished_at_ms"] = record.started_at_ms.into();
        return row;
    };
    let status = match result.outcome {
        JobOutcome::Cancelled => "cancelled",
        JobOutcome::Completed if result.exit_code == Some(0) => "completed",
        _ => "error",
    };
    row["status"] = status.into();
    row["finished_at_ms"] = record
        .started_at_ms
        .saturating_add(result.duration_ms)
        .into();
    let mut detail = Vec::new();
    if let Some(machine) = &record.machine_label {
        detail.push(format!("On {machine}"));
    }
    detail.push(match result.exit_code {
        Some(code) if result.outcome == JobOutcome::Completed => format!("exit {code}"),
        _ => outcome_words(result.outcome).into(),
    });
    if record.placement == JobPlacement::Remote && record.machine_label.is_some() {
        let files = result.files_back.len();
        detail.push(format!(
            "{files} {} back",
            if files == 1 { "file" } else { "files" }
        ));
        if !result.conflicts.is_empty() {
            let conflicts = result.conflicts.len();
            detail.push(format!(
                "{conflicts} {}",
                if conflicts == 1 {
                    "conflict"
                } else {
                    "conflicts"
                }
            ));
        }
    }
    row["text"] = detail.join(" · ").into();
    let output = output_end(&result.output_tail);
    if !output.trim().is_empty() {
        row[if status == "error" { "error" } else { "output" }] = output.into();
    }
    row
}

fn outcome_words(outcome: JobOutcome) -> &'static str {
    match outcome {
        JobOutcome::Completed => "completed",
        JobOutcome::TimeLimit => "stopped at its time limit",
        JobOutcome::MemoryLimit => "stopped at its memory limit",
        JobOutcome::OutsideEnd => "ended outside Chariox",
        JobOutcome::MachineLost => "machine lost",
        JobOutcome::CapacityUnavailable => "no machine free",
        JobOutcome::Cancelled => "cancelled",
    }
}

fn output_end(output: &str) -> &str {
    match output.char_indices().rev().nth(OUTPUT_CHARS - 1) {
        Some((start, _)) => &output[start..],
        None => output,
    }
}

/// A provider's own snapshot of a `chariox.jobs.run` call that started (or is still starting) a
/// job: the job's row is the call's row. A refused call keeps its own row, with why.
pub(in crate::runtime::state) fn is_job_launch_snapshot(
    kind: &TerminalOutputKind,
    bytes: &[u8],
) -> bool {
    if *kind != TerminalOutputKind::ProviderTool {
        return false;
    }
    let Ok(value) = serde_json::from_slice::<serde_json::Value>(bytes) else {
        return false;
    };
    let tool = value.get("tool").and_then(serde_json::Value::as_str);
    if tool.and_then(crate::transport::runtime_tools::canonical_agent_event_tool_name) != Some(TOOL)
    {
        return false;
    }
    let status = value
        .get("status")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("running");
    let failed = value.get("error").is_some_and(|error| {
        !error.is_null() && error.as_str() != Some("") && error.as_bool() != Some(false)
    });
    match status {
        "completed" | "success" if !failed => names_a_job(&value["output"]),
        "running" | "pending" | "in_progress" | "inProgress" | "started" => true,
        _ => false,
    }
}

/// Whether the call's result names the job it started (`job-` and 32 hex digits).
fn names_a_job(output: &serde_json::Value) -> bool {
    let text = match output {
        serde_json::Value::String(text) => text.clone(),
        other => other.to_string(),
    };
    text.match_indices("job-").any(|(at, _)| {
        let id = &text.as_bytes()[at + 4..];
        id.len() >= 32 && id[..32].iter().all(u8::is_ascii_hexdigit)
    })
}
