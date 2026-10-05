//! Runs on the writer inside the normal successful-completion transaction.
use super::*;
use crate::durable_state::DurableWorkflowRunWrite;
use crate::session::{WorkflowRun, WorkflowRunStatus};

pub(crate) fn capture_in(
    tx: &Transaction<'_>,
    kernel: &str,
    runs: &[DurableWorkflowRunWrite],
) -> rusqlite::Result<()> {
    for encoded in runs {
        if encoded.status != "Completed" {
            continue;
        }
        let previous:Option<String>=tx.query_row("SELECT status FROM durable_workflow_runs WHERE owner_id=?1 AND session_id=?2 AND run_id=?3",params![kernel,encoded.session_id,encoded.run_id],|r|r.get(0)).optional()?;
        if previous.as_deref() == Some("Completed") {
            continue;
        }
        let source_json: Option<String> = tx.query_row("SELECT payload_json FROM workflow_notification_sources WHERE kernel_id=?1 AND session_id=?2 AND workflow_id=?3 AND enabled=1",params![kernel,encoded.session_id,encoded.workflow_id],|r|r.get(0)).optional()?;
        let Some(source_json) = source_json else {
            continue;
        };
        let source: WorkflowNotificationSource = parse(&source_json)?;
        let run: WorkflowRun = parse(&encoded.payload_json)?;
        if run.status() != WorkflowRunStatus::Completed || run.final_output_valid() != Some(true) {
            continue;
        }
        let Some(output) = run.final_output() else {
            continue;
        };
        let inserted = tx.execute(
            "INSERT OR IGNORE INTO workflow_notification_occurrences VALUES (?1,?2,NULL)",
            params![source.source_id, run.id()],
        )?;
        if inserted == 0 {
            continue;
        }
        let mut ancestry: Vec<String> = if let Some(queue_id) = run.queue_item_id() {
            let lineage:Option<String>=tx.query_row("SELECT ancestry_json FROM workflow_notification_inbox WHERE session_id=?1 AND queued_prompt_id=?2",params![encoded.session_id,queue_id],|r|r.get(0)).optional()?;
            lineage
                .as_deref()
                .map(parse)
                .transpose()?
                .unwrap_or_default()
        } else {
            Vec::new()
        };
        let identity = workflow_identity(kernel, &encoded.session_id, run.workflow_id());
        let diagnostic = if ancestry.contains(&identity) {
            Some("workflow_notification_loop_dropped")
        } else if ancestry.len() >= 256 {
            Some("workflow_notification_ancestry_limit")
        } else if output.artifacts().len() > 32
            || !crate::durable_state::notification_target::encode(output)
                .is_ok_and(|bytes| bytes.len() <= MAX_OUTPUT_BYTES)
        {
            Some("workflow_notification_output_limit")
        } else {
            None
        };
        if let Some(code) = diagnostic {
            tx.execute("UPDATE workflow_notification_occurrences SET diagnostic=?3 WHERE source_id=?1 AND occurrence_id=?2",params![source.source_id,run.id(),code])?;
            continue;
        }
        ancestry.push(identity);
        let mut q=tx.prepare("SELECT payload_json FROM workflow_notification_subscriptions WHERE source_id=?1 AND owner_id=?2 ORDER BY subscription_id")?;
        let subscriptions = q
            .query_map(params![source.source_id, source.owner_user_id], |r| {
                r.get::<_, String>(0)
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        for value in subscriptions {
            let sub: WorkflowNotificationSubscription = parse(&value)?;
            let env = WorkflowNotificationEnvelope {
                source_id: source.source_id.clone(),
                occurrence_id: run.id().into(),
                output: output.clone(),
                ancestry: ancestry.clone(),
                deadline_ms: run
                    .completed_at_ms()
                    .unwrap_or(encoded.completed_at_ms.unwrap_or(0))
                    .saturating_add(u64::from(sub.ttl_days) * 86_400_000),
            };
            // No transport here: pending bytes are durable before the router runs.
            tx.execute(
                "INSERT INTO workflow_notification_outbox VALUES (?1,?2,?3,?4,'pending',?5,0)",
                params![
                    sub.subscription_id,
                    source.source_id,
                    run.id(),
                    env.deadline_ms as i64,
                    serde_json::to_string(&env).map_err(serialization)?
                ],
            )?;
        }
    }
    Ok(())
}
fn serialization(e: serde_json::Error) -> rusqlite::Error {
    rusqlite::Error::ToSqlConversionFailure(Box::new(e))
}
fn parse<T: serde::de::DeserializeOwned>(s: &str) -> rusqlite::Result<T> {
    serde_json::from_str(s).map_err(serialization)
}
