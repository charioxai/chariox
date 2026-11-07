//! Runs on the writer inside the normal terminal completion transaction.
use super::run_finished;
use super::*;
use crate::durable_state::DurableWorkflowRunWrite;
use crate::session::WorkflowRun;

pub(crate) fn capture_in(
    tx: &Transaction<'_>,
    kernel: &str,
    current_owner: &str,
    runs: &[DurableWorkflowRunWrite],
) -> rusqlite::Result<()> {
    for encoded in runs {
        if !matches!(&*encoded.status, "Completed" | "Failed") {
            continue;
        }
        let previous:Option<String>=tx.query_row("SELECT status FROM durable_workflow_runs WHERE owner_id=?1 AND session_id=?2 AND run_id=?3",params![kernel,encoded.session_id,encoded.run_id],|r|r.get(0)).optional()?;
        if matches!(previous.as_deref(), Some("Completed" | "Failed")) {
            continue;
        }
        let run: WorkflowRun = parse(&encoded.payload_json)?;
        run_finished::emit(tx, kernel, current_owner, encoded, &run)?;
        // A run that fails before reaching an exit emits nothing through an exit.
        if encoded.status != "Completed" {
            continue;
        }
        let Some(exit_node_id) = run
            .completed_by_node_run_id()
            .and_then(|id| run.node_runs().iter().find(|node_run| node_run.id() == id))
            .map(|node_run| node_run.node_id().to_owned())
        else {
            continue;
        };
        let source_json: Option<String> = tx.query_row("SELECT payload_json FROM workflow_notification_exit_sources WHERE kernel_id=?1 AND session_id=?2 AND workflow_id=?3 AND exit_node_id=?4 AND owner_id=?5 AND enabled=1",params![kernel,encoded.session_id,encoded.workflow_id,exit_key(crate::local::NotificationSourceKind::WorkflowExit,&exit_node_id),current_owner],|r|r.get(0)).optional()?;
        let Some(source_json) = source_json else {
            continue;
        };
        let source: WorkflowNotificationSource = parse(&source_json)?;
        let Some(output) = run
            .final_output()
            .filter(|_| run.final_output_valid() == Some(true))
            .cloned()
        else {
            continue;
        };
        deliver(tx, kernel, encoded, &run, &source, output)?;
    }
    Ok(())
}
/// One run's occurrence of a source: recorded once, dropped on a loop or a limit, otherwise
/// published to every subscriber with the run's ancestry.
pub(super) fn deliver(
    tx: &Transaction<'_>,
    kernel: &str,
    encoded: &DurableWorkflowRunWrite,
    run: &WorkflowRun,
    source: &WorkflowNotificationSource,
    output: crate::session::WorkflowOutputPayload,
) -> rusqlite::Result<()> {
    let inserted = tx.execute(
        "INSERT OR IGNORE INTO workflow_notification_occurrences VALUES (?1,?2,NULL)",
        params![source.source_id, run.id()],
    )?;
    prune_occurrences(tx)?;
    if inserted == 0 {
        return Ok(());
    }
    let mut ancestry: Vec<String> = if let Some(queue_id) = run.queue_item_id() {
        let lineage:Option<String>=tx.query_row("SELECT json_extract(invocation_json,'$.ancestry') FROM app_outbox WHERE source_kind='workflow_completion' AND queued_session_id=?1 AND queued_prompt_id=?2",params![encoded.session_id,queue_id],|r|r.get(0)).optional()?;
        lineage
            .as_deref()
            .map(parse)
            .transpose()?
            .unwrap_or_else(|| {
                run.publication_invocation()
                    .filter(|i| {
                        matches!(
                            i.transport.as_str(),
                            "workflow_notification" | "agent_notification"
                        )
                    })
                    .and_then(|i| i.input.get("ancestry"))
                    .and_then(|v| serde_json::from_value(v.clone()).ok())
                    .unwrap_or_default()
            })
    } else {
        run.publication_invocation()
            .filter(|i| {
                matches!(
                    i.transport.as_str(),
                    "workflow_notification" | "agent_notification"
                )
            })
            .and_then(|i| i.input.get("ancestry"))
            .and_then(|v| serde_json::from_value(v.clone()).ok())
            .unwrap_or_default()
    };
    let identity = workflow_identity(kernel, &encoded.session_id, run.workflow_id());
    let diagnostic = if ancestry.contains(&identity) {
        Some("workflow_notification_loop_dropped")
    } else if ancestry.len() >= 256 {
        Some("workflow_notification_ancestry_limit")
    } else if output.artifacts().len() > 32
        || !crate::durable_state::notification_target::encode(&output)
            .is_ok_and(|bytes| bytes.len() <= MAX_OUTPUT_BYTES)
    {
        Some("workflow_notification_output_limit")
    } else {
        None
    };
    if let Some(code) = diagnostic {
        tx.execute("UPDATE workflow_notification_occurrences SET diagnostic=?3 WHERE source_id=?1 AND occurrence_id=?2",params![source.source_id,run.id(),code])?;
        return Ok(());
    }
    ancestry.push(identity);
    publish_in(
        tx,
        source,
        run.id(),
        output,
        ancestry,
        run.completed_at_ms()
            .unwrap_or(encoded.completed_at_ms.unwrap_or(0)),
    )
    .map_err(|error| rusqlite::Error::ToSqlConversionFailure(Box::new(error)))
}
fn serialization(e: serde_json::Error) -> rusqlite::Error {
    rusqlite::Error::ToSqlConversionFailure(Box::new(e))
}
fn parse<T: serde::de::DeserializeOwned>(s: &str) -> rusqlite::Result<T> {
    serde_json::from_str(s).map_err(serialization)
}

pub(super) fn publish_in(
    tx: &Transaction<'_>,
    source: &WorkflowNotificationSource,
    occurrence_id: &str,
    output: crate::session::WorkflowOutputPayload,
    ancestry: Vec<String>,
    at: u64,
) -> Result<(), DaemonError> {
    publish_sql(tx, source, occurrence_id, output, ancestry, at).map_err(sql)
}
fn publish_sql(
    tx: &Transaction<'_>,
    source: &WorkflowNotificationSource,
    occurrence_id: &str,
    output: crate::session::WorkflowOutputPayload,
    ancestry: Vec<String>,
    at: u64,
) -> rusqlite::Result<()> {
    let display_name = naming::display_name_in(tx, source)?;
    let mut q=tx.prepare("SELECT notification_json FROM app_automations WHERE source_kind='workflow_completion' AND installation_id=?1 AND owner_id=?2 AND status='active' ORDER BY automation_id")?;
    let subscriptions = q
        .query_map(params![source.source_id, source.owner_user_id], |r| {
            r.get::<_, String>(0)
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    for value in subscriptions {
        let sub: WorkflowNotificationSubscription = parse(&value)?;
        let env = WorkflowNotificationEnvelope {
            source_id: source.source_id.clone(),
            display_name: display_name.clone(),
            occurrence_id: occurrence_id.into(),
            output: output.clone(),
            ancestry: ancestry.clone(),
            deadline_ms: at.saturating_add(crate::local::NOTIFICATION_RETENTION_MS),
        };
        let env = protect_envelope(env)
            .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
        // Source-owned receipt is committed with completion, before transport.
        if let Err(e) = insert_receipt(tx, &sub, &env, "retryable", at) {
            if let DaemonError::LocalTransport { ref message, .. } = e {
                let diagnostic = match message.as_str() {
                    "notification outbox full" | "notification payload limit" => {
                        Some("workflow_notification_delivery_limit")
                    }
                    "notification prompt limit" => Some("workflow_notification_prompt_limit"),
                    _ => None,
                };
                if let Some(code) = diagnostic {
                    tx.execute("UPDATE workflow_notification_occurrences SET diagnostic=?3 WHERE source_id=?1 AND occurrence_id=?2",params![source.source_id,occurrence_id,code])?;
                    continue;
                }
            }
            return Err(rusqlite::Error::ToSqlConversionFailure(Box::new(e)));
        }
    }
    Ok(())
}
