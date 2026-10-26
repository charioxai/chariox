//! Fold the pre-release round-1 delivery tables into the existing App receipts.
//! Preserve pending deadlines and queue linkage; never replay completed runs.
use super::*;
pub(super) fn migrate(db: &mut Connection) -> Result<(), DaemonError> {
    let exists:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='workflow_notification_subscriptions')",[],|r|r.get(0)).map_err(sql)?;
    if !exists {
        return Ok(());
    }
    let tx = db
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(sql)?;
    let subscriptions = {
        let mut q = tx
            .prepare("SELECT payload_json FROM workflow_notification_subscriptions")
            .map_err(sql)?;
        let rows = q.query_map([], |r| r.get::<_, String>(0)).map_err(sql)?;
        rows.collect::<rusqlite::Result<Vec<_>>>().map_err(sql)?
    };
    for encoded in subscriptions {
        let mut sub: WorkflowNotificationSubscription = decode(&encoded)?;
        if sub.source_kernel_id.is_empty() {
            sub.source_kernel_id = load_source(&tx, &sub.source_id)?.kernel_id;
        }
        // Round 1 accepted successes only, regardless of round-2 defaults.
        sub.events = crate::local::WorkflowNotificationEvents::Success;
        save_subscription(&tx, &mut sub)?;
        for (table, local) in [
            ("workflow_notification_outbox", false),
            ("workflow_notification_inbox", true),
        ] {
            let deliveries = {
                let mut q=tx.prepare(&format!("SELECT envelope_json,state,{} FROM {table} WHERE subscription_id=?1 AND envelope_json IS NOT NULL",if local {"queued_prompt_id"} else {"NULL"})).map_err(sql)?;
                let rows = q
                    .query_map([&sub.subscription_id], |r| {
                        Ok((
                            r.get::<_, String>(0)?,
                            r.get::<_, String>(1)?,
                            r.get::<_, Option<String>>(2)?,
                        ))
                    })
                    .map_err(sql)?;
                rows.collect::<rusqlite::Result<Vec<_>>>().map_err(sql)?
            };
            for (json, state, queued) in deliveries {
                let mut value: serde_json::Value = decode(&json)?;
                value["status"] = serde_json::json!("success");
                value["subject"] = serde_json::Value::Null;
                value["fields"] = serde_json::json!({"status":"success"});
                let env: WorkflowNotificationEnvelope = serde_json::from_value(value)
                    .map_err(|_| error("legacy notification corrupt"))?;
                let state = if state == "pending" {
                    "retryable"
                } else {
                    state.as_str()
                };
                if !local {
                    insert_receipt_row(
                        &tx,
                        &sub,
                        &env,
                        state,
                        env.deadline_ms
                            .saturating_sub(u64::from(sub.ttl_days) * 86_400_000),
                    )?;
                } else {
                    // Only this kernel's incoming half sets queue/session lineage.
                    let invocation = encode(&serde_json::json!({"ancestry":env.ancestry}))?;
                    let exists:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM app_outbox WHERE source_kind='workflow_completion' AND automation_id=?1 AND installation_id=?2 AND occurrence_id=?3)",params![sub.subscription_id,env.source_id,env.occurrence_id],|r|r.get(0)).map_err(sql)?;
                    if !exists {
                        insert_receipt_row(
                            &tx,
                            &sub,
                            &env,
                            state,
                            env.deadline_ms
                                .saturating_sub(u64::from(sub.ttl_days) * 86_400_000),
                        )?;
                    }
                    tx.execute("UPDATE app_outbox SET state=?4,queued_prompt_id=?5,queued_session_id=?6,invocation_json=?7 WHERE source_kind='workflow_completion' AND automation_id=?1 AND installation_id=?2 AND occurrence_id=?3",params![sub.subscription_id,env.source_id,env.occurrence_id,state,queued,sub.session_id,invocation]).map_err(sql)?;
                    if queued.is_some() {
                        // Keep the invocation id already recorded in legacy hot queues/runs.
                        let legacy = format!(
                            "{}:{}:{}",
                            sub.subscription_id, env.source_id, env.occurrence_id
                        );
                        tx.execute("UPDATE app_outbox SET receipt_id=?4 WHERE source_kind='workflow_completion' AND automation_id=?1 AND installation_id=?2 AND occurrence_id=?3",params![sub.subscription_id,env.source_id,env.occurrence_id,legacy]).map_err(sql)?;
                    }
                }
            }
        }
    }
    tx.execute_batch("DROP TABLE workflow_notification_subscriptions; DROP TABLE workflow_notification_outbox; DROP TABLE workflow_notification_inbox;").map_err(sql)?;
    tx.commit().map_err(sql)
}
