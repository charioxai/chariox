//! The end of any workflow run, completed or failed, as a notification source. An exit's
//! source (`completion.rs`) says what the run produced; this one says that the run ended and
//! how, so a follower learns about a failed run too.
use super::*;
use crate::durable_state::DurableWorkflowRunWrite;
use crate::session::{WorkflowOutputPayload, WorkflowRun};

/// The `exit_node_id` of a workflow's "run finished" source. It is no node: the source
/// follows the run, not an exit.
pub(crate) const RUN_FINISHED_NODE: &str = "run_finished";
/// Longest invocation prompt or failure message an occurrence carries.
const TEXT_CHARS: usize = 2_000;

/// What a follower is told about one run that ended or holds on a named thing: how it ended,
/// which run (its number and workflow), what started it and, for a failed run, the first
/// failure the run recorded and its typed outcome.
pub(crate) fn output_of(run: &WorkflowRun, workflow_name: &str) -> WorkflowOutputPayload {
    let text = |value: &str| value.chars().take(TEXT_CHARS).collect::<String>();
    let mut body = serde_json::json!({
        "status": format!("{:?}", run.status()),
        "run_id": run.id(),
        "number": run.number(),
        "workflow_id": run.workflow_id(),
        "workflow_name": workflow_name,
        "prompt": run.invocation_prompt().map(text),
    });
    if let Some(failure) = run.failure_events().first() {
        body["failure"] = text(failure.message()).into();
    }
    if let Some(outcome) = run.outcome() {
        body["outcome"] = serde_json::to_value(outcome).expect("outcome serializes");
    }
    WorkflowOutputPayload::new(body.to_string(), Vec::new())
}

/// Publishes a run's occurrence to its workflow's enabled "run finished" source: the end of a
/// run, or a run that enters waiting on a named thing (one occurrence per run, node and thing).
pub(super) fn emit(
    tx: &Transaction<'_>,
    kernel: &str,
    current_owner: &str,
    encoded: &DurableWorkflowRunWrite,
    run: &WorkflowRun,
    occurrence_id: &str,
) -> rusqlite::Result<()> {
    let source_json: Option<String> = tx.query_row("SELECT payload_json FROM workflow_notification_exit_sources WHERE kernel_id=?1 AND session_id=?2 AND workflow_id=?3 AND exit_node_id=?4 AND owner_id=?5 AND enabled=1",params![kernel,encoded.session_id,encoded.workflow_id,exit_key(crate::local::NotificationSourceKind::WorkflowRun,RUN_FINISHED_NODE),current_owner],|r|r.get(0)).optional()?;
    let Some(source_json) = source_json else {
        return Ok(());
    };
    let source: WorkflowNotificationSource = serde_json::from_str(&source_json)
        .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
    super::completion::deliver(
        tx,
        kernel,
        encoded,
        run,
        occurrence_id,
        &source,
        output_of(run, &source.name),
    )
}
