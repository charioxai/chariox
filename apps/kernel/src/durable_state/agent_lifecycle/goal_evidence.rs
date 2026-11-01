//! MP-08 / MP-09 / MP-10 / MP-11: revision-bound evidence from the existing ledger journal.
use super::*;

pub(crate) struct GoalEvidenceEvent {
    pub sequence: u64,
    pub kind: String,
    pub payload: serde_json::Value,
}

impl DurableKernelStateStore {
    pub(crate) fn agent_goal_evidence_events(
        &self,
        task: &AgentTaskExecution,
    ) -> Result<Vec<GoalEvidenceEvent>, DaemonError> {
        let Some(goal) = &task.goal else {
            return Ok(Vec::new());
        };
        let db = self.lock_connection("agent_goal_evidence")?;
        // Sudo receipts have no objective revision. Admit them only through a
        // journalled owner resume belonging to this exact goal revision.
        let mut query = db.prepare("SELECT sequence,kind,payload_json FROM durable_state_events e
            WHERE (kind IN ('goal.evaluation','goal.owner_resume','goal.blocked')
                AND json_extract(payload_json,'$.task_id')=?1
                AND json_extract(payload_json,'$.goal_id')=?2
                AND COALESCE(json_extract(payload_json,'$.evaluation.objective_revision'),json_extract(payload_json,'$.objective_revision'))=?3)
            OR (kind='kernel_access.sudo'
                AND json_extract(payload_json,'$.turn.task_id')=?1
                AND json_extract(payload_json,'$.outcome')='started'
                AND EXISTS(SELECT 1 FROM durable_state_events r WHERE r.kind='goal.owner_resume'
                    AND json_extract(r.payload_json,'$.task_id')=?1
                    AND json_extract(r.payload_json,'$.goal_id')=?2
                    AND json_extract(r.payload_json,'$.objective_revision')=?3
                    AND json_extract(r.payload_json,'$.prompt_id')=json_extract(e.payload_json,'$.turn.prompt_id')))
            ORDER BY sequence DESC LIMIT 129").map_err(sql)?;
        let rows = query
            .query_map(
                params![task.task_id, goal.id, sql_integer(goal.objective_revision)?],
                |r| {
                    Ok((
                        r.get::<_, i64>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, String>(2)?,
                    ))
                },
            )
            .map_err(sql)?;
        let mut events = Vec::new();
        for row in rows {
            let (sequence, kind, payload) = row.map_err(sql)?;
            events.push(GoalEvidenceEvent {
                sequence: u64::try_from(sequence)
                    .map_err(|_| error("negative goal evidence sequence; inspect the ledger"))?,
                kind,
                payload: decode(&payload)?,
            });
        }
        events.reverse();
        Ok(events)
    }
}
