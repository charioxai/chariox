//! Runs on the writer inside the normal terminal completion transaction.
use super::*;
use crate::durable_state::DurableWorkflowRunWrite;
use crate::session::{WorkflowRun, WorkflowRunStatus};

pub(crate) fn capture_in(
    tx: &Transaction<'_>,
    kernel: &str,
    current_owner: &str,
    runs: &[DurableWorkflowRunWrite],
) -> rusqlite::Result<()> {
    for encoded in runs {
        if !matches!(encoded.status.as_str(), "Completed" | "Failed") {
            continue;
        }
        let previous:Option<String>=tx.query_row("SELECT status FROM durable_workflow_runs WHERE owner_id=?1 AND session_id=?2 AND run_id=?3",params![kernel,encoded.session_id,encoded.run_id],|r|r.get(0)).optional()?;
        if matches!(previous.as_deref(), Some("Completed" | "Failed")) {
            continue;
        }
        let source_json: Option<String> = tx.query_row("SELECT payload_json FROM workflow_notification_sources WHERE kernel_id=?1 AND session_id=?2 AND workflow_id=?3 AND owner_id=?4 AND enabled=1",params![kernel,encoded.session_id,encoded.workflow_id,current_owner],|r|r.get(0)).optional()?;
        let Some(source_json) = source_json else {
            continue;
        };
        let source: WorkflowNotificationSource = parse(&source_json)?;
        let run: WorkflowRun = parse(&encoded.payload_json)?;
        let status = if run.status() == WorkflowRunStatus::Failed {
            crate::local::WorkflowNotificationStatus::Failure
        } else {
            if run.final_output_valid() != Some(true) || run.final_output().is_none() {
                continue;
            }
            crate::local::WorkflowNotificationStatus::Success
        };
        let output = if status == crate::local::WorkflowNotificationStatus::Success {
            run.final_output().cloned()
        } else {
            None
        };
        let (subject, fields) = provenance(&run, &source, output.as_ref());
        let inserted = tx.execute(
            "INSERT OR IGNORE INTO workflow_notification_occurrences VALUES (?1,?2,NULL)",
            params![source.source_id, run.id()],
        )?;
        if inserted == 0 {
            continue;
        }
        let mut ancestry: Vec<String> = if let Some(queue_id) = run.queue_item_id() {
            let lineage:Option<String>=tx.query_row("SELECT json_extract(invocation_json,'$.ancestry') FROM app_outbox WHERE source_kind='workflow_completion' AND queued_session_id=?1 AND queued_prompt_id=?2",params![encoded.session_id,queue_id],|r|r.get(0)).optional()?;
            lineage
                .as_deref()
                .map(parse)
                .transpose()?
                .unwrap_or_else(|| {
                    run.publication_invocation()
                        .filter(|i| i.transport == "workflow_notification")
                        .and_then(|i| i.input.get("ancestry"))
                        .and_then(|v| serde_json::from_value(v.clone()).ok())
                        .unwrap_or_default()
                })
        } else {
            run.publication_invocation()
                .filter(|i| i.transport == "workflow_notification")
                .and_then(|i| i.input.get("ancestry"))
                .and_then(|v| serde_json::from_value(v.clone()).ok())
                .unwrap_or_default()
        };
        // A workflow-level injection contributes lineage to that same run,
        // including when its original trigger was an App or manual invocation.
        let injected: Vec<String> = {
            let mut q = tx.prepare("SELECT json_extract(invocation_json,'$.ancestry') FROM app_outbox WHERE source_kind='workflow_completion' AND queued_session_id=?1 AND json_extract(invocation_json,'$.injected_run_id')=?2")?;
            let rows = q.query_map(params![encoded.session_id, run.id()], |r| {
                r.get::<_, String>(0)
            })?;
            rows.collect::<rusqlite::Result<Vec<_>>>()?
        };
        for value in injected {
            for identity in parse::<Vec<String>>(&value)? {
                if !ancestry.contains(&identity) {
                    ancestry.push(identity);
                }
            }
        }
        let identity = workflow_identity(kernel, &encoded.session_id, run.workflow_id());
        let diagnostic = if ancestry.contains(&identity) {
            Some("workflow_notification_loop_dropped")
        } else if ancestry.len() >= 256 {
            Some("workflow_notification_ancestry_limit")
        } else if subject.as_ref().is_some_and(|subject| subject.len() > 512) {
            Some("workflow_notification_subject_limit")
        } else if output.as_ref().is_some_and(|output| {
            output.artifacts().len() > 32
                || !crate::durable_state::notification_target::encode(output)
                    .is_ok_and(|bytes| bytes.len() <= MAX_OUTPUT_BYTES)
        }) {
            Some("workflow_notification_output_limit")
        } else {
            None
        };
        if let Some(code) = diagnostic {
            tx.execute("UPDATE workflow_notification_occurrences SET diagnostic=?3 WHERE source_id=?1 AND occurrence_id=?2",params![source.source_id,run.id(),code])?;
            continue;
        }
        ancestry.push(identity);
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
                occurrence_id: run.id().into(),
                output: output.clone(),
                status,
                subject: subject.clone(),
                fields: fields.clone(),
                ancestry: ancestry.clone(),
                deadline_ms: run
                    .completed_at_ms()
                    .unwrap_or(encoded.completed_at_ms.unwrap_or(0))
                    .saturating_add(u64::from(sub.ttl_days) * 86_400_000),
            };
            if !matches(&sub, &env) {
                continue;
            }
            let env = protect_envelope(env)
                .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
            // Source-owned receipt is committed with completion, before transport.
            if let Err(e) = insert_receipt(
                tx,
                &sub,
                &env,
                "retryable",
                run.completed_at_ms().unwrap_or(0),
            ) {
                if let DaemonError::LocalTransport { ref message, .. } = e {
                    let diagnostic = match message.as_str() {
                        "notification outbox full" | "notification payload limit" => {
                            Some("workflow_notification_delivery_limit")
                        }
                        "notification prompt limit" => Some("workflow_notification_prompt_limit"),
                        _ => None,
                    };
                    if let Some(code) = diagnostic {
                        tx.execute("UPDATE workflow_notification_occurrences SET diagnostic=?3 WHERE source_id=?1 AND occurrence_id=?2",params![source.source_id,run.id(),code])?;
                        continue;
                    }
                }
                return Err(rusqlite::Error::ToSqlConversionFailure(Box::new(e)));
            }
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

/// Only recorded invocation provenance supplies correlation, never final-output text.
fn provenance(
    run: &WorkflowRun,
    source: &WorkflowNotificationSource,
    output: Option<&crate::session::WorkflowOutputPayload>,
) -> (Option<String>, serde_json::Value) {
    use serde_json::{json, Value};
    let mut fields = serde_json::Map::new();
    fields.insert(
        "status".into(),
        json!(if run.status() == WorkflowRunStatus::Failed {
            "failure"
        } else {
            "success"
        }),
    );
    if let Some(inv) = run.publication_invocation().filter(|i| {
        matches!(
            i.transport.as_str(),
            "app_event" | "event" | "workflow_notification"
        )
    }) {
        let payload = inv.input.get("payload").unwrap_or(&inv.input);
        // App/generator metadata is opaque. Workflow completions already carry
        // the projected fields as payload; copy nested values without interpretation.
        let metadata = payload.get("metadata").unwrap_or(payload);
        if let Some(metadata) = metadata.as_object() {
            for (key, value) in metadata {
                fields.entry(key.clone()).or_insert_with(|| value.clone());
            }
        }
    }
    if let Some(output) = output {
        if let Ok(structured) = serde_json::from_str::<Value>(output.message()) {
            for key in &source.output_fields {
                // Provenance/status are reserved and cannot be replaced by agent text.
                if let Some(value) = structured.get(key).filter(|v| small_scalar(v)) {
                    fields.entry(key.clone()).or_insert_with(|| value.clone());
                }
            }
        }
    }
    let subject = fields
        .get("subject")
        .and_then(Value::as_str)
        .map(str::to_owned);
    (subject, Value::Object(fields))
}
fn small_scalar(v: &serde_json::Value) -> bool {
    v.is_boolean()
        || v.is_number()
        || v.as_str()
            .is_some_and(|s| s.len() <= 512 && !s.chars().any(char::is_control))
}
