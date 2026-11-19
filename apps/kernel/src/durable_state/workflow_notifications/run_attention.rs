//! A run that needs someone: the "run finished" source tells a responsible agent, and the
//! person, when responsible, gets one alert on their phone.
use super::*;
use crate::durable_state::DurableWorkflowRunWrite;
use crate::session::{WorkflowOutputPayload, WorkflowRun, WorkflowRunOutcome};

/// A cheap test before parsing a run that has not ended: only a node held on a named thing
/// has a waiting reason or sits blocked on a claim.
pub(super) fn may_hold(payload_json: &str) -> bool {
    payload_json.contains("\"waiting_reason\"") || payload_json.contains("BlockedOnWorkspaceClaim")
}

/// Publishes what a run that holds on a named thing needs, and alerts the person.
pub(super) fn capture(
    tx: &Transaction<'_>,
    kernel: &str,
    current_owner: &str,
    encoded: &DurableWorkflowRunWrite,
    run: &WorkflowRun,
) -> rusqlite::Result<()> {
    let Some(outcome) = run.outcome() else {
        return Ok(());
    };
    if let WorkflowRunOutcome::Waiting { .. } = &outcome {
        let occurrence = format!("{}:{}", run.id(), outcome.key());
        run_finished::emit(tx, kernel, current_owner, encoded, run, &occurrence)?;
    }
    if run.attention() && person_is_responsible(tx, kernel, encoded)? {
        alert_owner(tx, kernel, current_owner, encoded, run, &outcome)?;
    }
    Ok(())
}

fn person_is_responsible(
    tx: &Transaction<'_>,
    kernel: &str,
    encoded: &DurableWorkflowRunWrite,
) -> rusqlite::Result<bool> {
    let workflow: Option<String> = tx
        .query_row(
            "SELECT payload_json FROM durable_workflow_hot_entities WHERE owner_id=?1 AND session_id=?2 AND entity_kind='workflow' AND entity_id=?3",
            params![kernel, encoded.session_id, encoded.workflow_id],
            |row| row.get(0),
        )
        .optional()?;
    Ok(workflow
        .and_then(|json| serde_json::from_str::<serde_json::Value>(&json).ok())
        .is_none_or(|workflow| {
            workflow
                .pointer("/responsible/kind")
                .and_then(serde_json::Value::as_str)
                .is_none_or(|kind| kind == "person")
        }))
}

/// One occurrence of the Room's "needs your action" source per run and need. It carries ids
/// only: nothing a run wrote reaches an outbound alert.
fn alert_owner(
    tx: &Transaction<'_>,
    kernel: &str,
    current_owner: &str,
    encoded: &DurableWorkflowRunWrite,
    run: &WorkflowRun,
    outcome: &WorkflowRunOutcome,
) -> rusqlite::Result<()> {
    let source_json: Option<String> = tx.query_row("SELECT payload_json FROM workflow_notification_exit_sources WHERE kernel_id=?1 AND session_id=?2 AND exit_node_id=?3 AND owner_id=?4 AND enabled=1 AND json_extract(payload_json,'$.kind')='owner_action'",params![kernel,encoded.session_id,exit_key(crate::local::NotificationSourceKind::OwnerAction,"blocked"),current_owner],|r|r.get(0)).optional()?;
    let Some(source_json) = source_json else {
        return Ok(());
    };
    let source: WorkflowNotificationSource = serde_json::from_str(&source_json)
        .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
    let occurrence = format!("run-{}-{}", run.id(), outcome.key());
    let inserted = tx.execute(
        "INSERT OR IGNORE INTO workflow_notification_occurrences VALUES (?1,?2,NULL)",
        params![source.source_id, occurrence],
    )?;
    if inserted == 0 {
        return Ok(());
    }
    let output = WorkflowOutputPayload::new(
        serde_json::json!({
            "title": "Workflow run needs you",
            "room_id": encoded.session_id,
            "workflow_id": encoded.workflow_id,
            "run_id": run.id(),
            "number": run.number(),
        })
        .to_string(),
        Vec::new(),
    );
    super::completion::publish_in(
        tx,
        &source,
        &occurrence,
        output,
        Vec::new(),
        crate::session::unix_epoch_ms(),
    )
    .map_err(|error| rusqlite::Error::ToSqlConversionFailure(Box::new(error)))?;
    prune_occurrences(tx)
}
