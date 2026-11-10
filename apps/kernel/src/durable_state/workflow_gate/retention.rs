//! Bounded maintenance on the same writer as admission; pending work is never evicted.
use super::*;
const DAY: u64 = 86_400_000;
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DecisionSummary {
    pub evaluation_sha256: String,
    pub report_sha256: String,
    pub decision: String,
    pub target_nodes: Vec<String>,
    pub next_due_at_ms: Option<u64>,
    pub batch_key: Option<String>,
    pub reason: String,
    pub profile: String,
    pub fuel_consumed: u64,
    pub execution_ms: u64,
}
fn compact(record: &mut DecisionRecord) -> Result<()> {
    let Some(evaluation) = record.evaluation.take() else {
        return Ok(());
    };
    let report = record
        .report
        .take()
        .ok_or_else(|| error("gate replay report missing"))?;
    let mut summary = DecisionSummary {
        evaluation_sha256: gate::sha256(encode(&evaluation)?.as_bytes()),
        report_sha256: gate::sha256(encode(&report)?.as_bytes()),
        decision: "failed".into(),
        target_nodes: vec![],
        next_due_at_ms: None,
        batch_key: None,
        reason: String::new(),
        profile: report.profile.clone(),
        fuel_consumed: report.fuel_consumed,
        execution_ms: report.execution_ms,
    };
    match &report.outcome {
        Err(e) => summary.reason = e.to_string(),
        Ok(output) => {
            summary.reason = output.reason.clone();
            match &output.decision {
                gate::Decision::Run { targets } => {
                    summary.decision = "run".into();
                    summary.target_nodes = targets.iter().map(|t| t.node_id.clone()).collect();
                }
                gate::Decision::Drop => summary.decision = "drop".into(),
                gate::Decision::DeferUntil { at_ms } => {
                    summary.decision = "defer_until".into();
                    summary.next_due_at_ms = Some(*at_ms);
                }
                gate::Decision::Batch {
                    key, flush_at_ms, ..
                } => {
                    summary.decision = "batch".into();
                    summary.batch_key = Some(key.clone());
                    summary.next_due_at_ms = Some(*flush_at_ms);
                }
            }
        }
    }
    record.compacted = Some(summary);
    Ok(())
}
pub(super) fn retained_bytes(db: &Connection) -> Result<i64> {
    db.query_row("SELECT coalesce((SELECT sum(length(body_json)+coalesce(length(summary_json),0)) FROM workflow_gate_entries),0)+coalesce((SELECT sum(length(body_json)) FROM workflow_gate_decisions),0)+coalesce((SELECT sum(length(payload_json)) FROM workflow_gate_members),0)",[],|r|r.get(0)).map_err(sql)
}
pub(super) fn decision_reservations(db: &Connection) -> Result<i64> {
    db.query_row("SELECT (SELECT count(*) FROM workflow_gate_decisions)+coalesce((SELECT sum(32-(SELECT count(*) FROM workflow_gate_decisions d WHERE d.entry_id=workflow_gate_entries.entry_id)) FROM workflow_gate_entries WHERE terminal_at_ms IS NULL),0)",[],|r|r.get(0)).map_err(sql)
}
pub(super) fn body_capacity(db: &Connection, a: &Admission, extra: i64) -> Result<bool> {
    // Charge originals, snapshots and transformed/duplicate batch copies together.
    let query="SELECT coalesce(sum(length(e.body_json)+coalesce((SELECT sum(length(d.body_json)) FROM workflow_gate_decisions d WHERE d.entry_id=e.entry_id),0)+coalesce((SELECT sum(length(m.payload_json)) FROM workflow_gate_members m WHERE m.entry_id=e.entry_id),0)),0) FROM workflow_gate_entries e WHERE e.terminal_at_ms IS NULL";
    let kernel: i64 = db.query_row(query, [], |r| r.get(0)).map_err(sql)?;
    let pending: i64 = db
        .query_row(
            "SELECT count(*) FROM workflow_gate_entries WHERE terminal_at_ms IS NULL",
            [],
            |r| r.get(0),
        )
        .map_err(sql)?;
    let local: i64 = db
        .query_row(
            &format!("{query} AND e.owner_id=?1 AND e.session_id=?2 AND e.workflow_id=?3"),
            params![a.owner, a.session_id, a.workflow_id],
            |r| r.get(0),
        )
        .map_err(sql)?;
    Ok(kernel + extra <= 64 * 1024 * 1024
        && local + extra <= 16 * 1024 * 1024
        && retained_bytes(db)? + extra + pending * 4096 + 2 * 1024 * 1024 <= 256 * 1024 * 1024)
}
/// A passed entry is the proof `require_queue_pass_in` loads when its prompt
/// starts, so it stays whole for as long as that prompt waits in a queue.
const NOT_HELD_IN_QUEUE: &str = "NOT EXISTS(SELECT 1 FROM durable_workflow_hot_entities h WHERE h.session_id=e.session_id AND h.entity_kind='queued_prompt' AND h.entity_id=json_extract(coalesce(e.summary_json,json_extract(nullif(e.body_json,''),'$.entry')),'$.queued_prompt_id'))";
pub(super) fn sweep(tx: &Transaction<'_>, now: u64) -> Result<()> {
    for id in ids(tx,&format!("SELECT entry_id FROM workflow_gate_entries e WHERE terminal_at_ms IS NOT NULL AND summary_json IS NULL AND (terminal_at_ms<=?1 OR deadline_ms<=?2) AND {NOT_HELD_IN_QUEUE} ORDER BY sequence LIMIT 16"),params![now.saturating_sub(7*DAY) as i64,now as i64])? {
        let mut e=entry(tx,&id)?.ok_or_else(||error("gate retention entry missing"))?;
        e.replay_compacted_at_ms=Some(now);
        let mut q=tx.prepare("SELECT evaluation_number,body_json FROM workflow_gate_decisions WHERE entry_id=?1 ORDER BY evaluation_number").map_err(sql)?;
        let rows=q.query_map([&id],|r|Ok((r.get::<_,i64>(0)?,r.get::<_,String>(1)?))).map_err(sql)?.collect::<std::result::Result<Vec<_>,_>>().map_err(sql)?;
        for (number,json) in rows {
            let mut record:DecisionRecord=decode(&json)?;compact(&mut record)?;
            tx.execute("UPDATE workflow_gate_decisions SET body_json=?3 WHERE entry_id=?1 AND evaluation_number=?2",params![id,number,encode(&record)?]).map_err(sql)?;
        }
        tx.execute("UPDATE workflow_gate_entries SET body_json='',summary_json=?2 WHERE entry_id=?1",params![id,encode(&e)?]).map_err(sql)?;
        tx.execute("DELETE FROM workflow_gate_members WHERE entry_id=?1",[&id]).map_err(sql)?;
        tx.execute("DELETE FROM workflow_gate_batches WHERE entry_id=?1 AND closed=1",[&id]).map_err(sql)?;
    }
    for id in ids(tx,&format!("SELECT entry_id FROM workflow_gate_entries e WHERE terminal_at_ms<=?1 AND deadline_ms<=?1 AND {NOT_HELD_IN_QUEUE} ORDER BY sequence LIMIT 16"),[now.saturating_sub(30*DAY) as i64])? {
        tx.execute("DELETE FROM workflow_gate_decisions WHERE entry_id=?1",[&id]).map_err(sql)?;
        tx.execute("DELETE FROM workflow_gate_members WHERE entry_id=?1",[&id]).map_err(sql)?;
        tx.execute("DELETE FROM workflow_gate_batches WHERE entry_id=?1 AND closed=1",[&id]).map_err(sql)?;
        tx.execute("DELETE FROM workflow_gate_entries WHERE entry_id=?1",[&id]).map_err(sql)?;
    }
    tx.execute("DELETE FROM workflow_gate_modules WHERE (owner_id,digest) IN (SELECT owner_id,digest FROM workflow_gate_modules m WHERE m.created_at_ms<=?1 AND NOT EXISTS(SELECT 1 FROM workflow_gate_entries e WHERE e.owner_id=m.owner_id AND json_extract(coalesce(e.summary_json,json_extract(nullif(e.body_json,''),'$.entry')),'$.module_sha256')=m.digest) AND NOT EXISTS(SELECT 1 FROM durable_workflow_hot_entities w WHERE w.entity_kind='workflow' AND json_extract(w.payload_json,'$.gate.module_sha256')=m.digest) LIMIT 8)",[now.saturating_sub(30*DAY) as i64]).map_err(sql)?;
    tx.execute("DELETE FROM workflow_gate_state WHERE rowid IN (SELECT s.rowid FROM workflow_gate_state s WHERE NOT EXISTS(SELECT 1 FROM durable_workflow_hot_entities w WHERE w.session_id=s.session_id AND w.entity_kind='workflow' AND w.entity_id=s.workflow_id) AND NOT EXISTS(SELECT 1 FROM workflow_gate_entries e WHERE e.owner_id=s.owner_id AND e.session_id=s.session_id AND e.workflow_id=s.workflow_id) LIMIT 8)",[]).map_err(sql)?;
    Ok(())
}
