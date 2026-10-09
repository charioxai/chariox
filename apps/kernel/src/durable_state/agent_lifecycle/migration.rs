//! MP-08 / MP-10 / MP-11 A02: preserve PR1 intent identity; never guess outcomes.
use super::*;
pub(super) fn migrate(db: &mut Connection) -> Result<(), DaemonError> {
    let tx = db
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(sql)?;
    let migrated: Option<String> = tx
        .query_row(
            "SELECT metadata_value FROM durable_state_metadata WHERE owner_id='agent-lifecycle' AND metadata_key='pr1-migrated'",
            [],
            |r| r.get(0),
        )
        .optional()
        .map_err(sql)?;
    if migrated.is_some() {
        return Ok(());
    }
    let mut q=tx.prepare("SELECT payload_json FROM durable_state_events WHERE kind='room.obligation.registered' ORDER BY sequence").map_err(sql)?;
    let rows = q
        .query_map([], |r| r.get::<_, String>(0))
        .map_err(sql)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(sql)?;
    drop(q);
    for row in rows {
        let value: serde_json::Value = decode(&row)?;
        let room = value["room_id"]
            .as_str()
            .ok_or_else(|| error("legacy obligation lacks room; recovery required"))?;
        let agent = value["creating_agent_id"]
            .as_str()
            .ok_or_else(|| error("legacy obligation lacks agent; recovery required"))?;
        let id = value["id"]
            .as_str()
            .ok_or_else(|| error("legacy obligation lacks identity; recovery required"))?;
        let prompt = value["creating_prompt_id"]
            .as_str()
            .map(str::to_owned)
            .unwrap_or_else(|| format!("legacy-{agent}"));
        let mut task = for_turn(&tx, room, agent, &prompt)?.unwrap_or_else(|| {
            new_task(
                room.into(),
                agent.into(),
                prompt.clone(),
                value["creating_provider_run_id"]
                    .as_str()
                    .map(str::to_owned),
                value["created_at_ms"].as_u64().unwrap_or(0),
            )
        });
        if task.obligations.iter().any(|o| o.id == id) {
            continue;
        }
        let receipt:Option<String>=tx.query_row("SELECT payload_json FROM durable_state_events WHERE kind='room.obligation.dispatch_receipt' AND subject_id=?1 ORDER BY sequence DESC LIMIT 1",[id],|r|r.get(0)).optional().map_err(sql)?;
        let receipt = receipt
            .map(|r| decode::<serde_json::Value>(&r))
            .transpose()?;
        task.obligations.push(AgentObligation {
            id: id.into(),
            kind: value["kind"].as_str().unwrap_or("unknown").into(),
            completion_task_id: None,
            resource_id: receipt
                .as_ref()
                .and_then(|r| r["resource_id"].as_str())
                .or(value["resource_ref"].as_str())
                .map(str::to_owned),
            status: "open".into(),
            dispatch_state: receipt
                .as_ref()
                .and_then(|r| r["dispatch_state"].as_str())
                .unwrap_or("uncertain")
                .into(),
        });
        task.state = ExecutionState::Blocked;
        task.blocked_revision = task.revision + 1;
        task.reason="Recovered legacy dispatch obligation; owner must reconcile its exact resource and resume or cancel".into();
        task.revision += 1;
        save(&tx, &task)?;
    }
    tx.execute(
        "INSERT INTO durable_state_metadata(owner_id,metadata_key,metadata_value,updated_at_ms) VALUES('agent-lifecycle','pr1-migrated','1',0)",
        [],
    )
    .map_err(sql)?;
    tx.commit().map_err(sql)
}
pub(super) fn audit(
    tx: &Transaction<'_>,
    kind: &str,
    id: &str,
    payload: serde_json::Value,
) -> Result<(), DaemonError> {
    tx.execute("INSERT INTO durable_state_events(event_id,kind,subject_id,timestamp_ms,payload_json) VALUES(?1,?2,?3,?4,?5)",params![format!("agent-ledger-{:032x}",rand::random::<u128>()),kind,id,sql_integer(crate::session::unix_epoch_ms())?,encode(&payload)?]).map_err(sql)?;
    Ok(())
}
