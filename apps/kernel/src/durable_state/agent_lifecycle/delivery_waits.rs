//! MP-08 / MP-09 / MP-10 / MP-11 A12-S02: finite Goal waits behind uncertain delivery.
use super::*;

pub(super) fn block_undeliverable_waits(
    tx: &Transaction<'_>,
    now: u64,
) -> Result<Vec<AgentTaskExecution>, DaemonError> {
    let mut changed = Vec::new();
    for mut task in waiting_tasks(tx)? {
        if task.goal.is_none() || task.state != ExecutionState::Waiting {
            continue;
        }
        // Admission rejects every unresolved recipient attempt, including a
        // later urgent steer. None can be skipped or replayed to deliver a wake.
        let blocker: Option<i64> = tx
            .query_row(
                "SELECT blocked.sequence FROM agent_inbox blocked JOIN agent_inbox wake
             ON blocked.room_id=wake.room_id AND blocked.agent_id=wake.agent_id
             WHERE wake.room_id=?1 AND wake.agent_id=?2
             AND json_extract(blocked.payload,'$.state')='blocked'
             AND json_extract(wake.payload,'$.state')='pending'
             AND json_extract(wake.payload,'$.kind') IN
                 ('deadline_reached','wait_recheck','source_lost','source_completed')
             AND (json_extract(wake.payload,'$.payload.task_id')=?3 OR
                 EXISTS(SELECT 1 FROM json_each(json_extract(wake.payload,'$.payload.task_ids'))
                        WHERE value=?3))
             ORDER BY blocked.sequence LIMIT 1",
                params![task.room_id, task.agent_id, task.task_id],
                |row| row.get(0),
            )
            .optional()
            .map_err(sql)?;
        let Some(sequence) = blocker else { continue };
        goals::clock(&mut task, now);
        goals::blocked(&mut task, &format!(
            "Unconfirmed delivery {sequence} prevents this task's wake; owner must reconcile the original attempt or cancel, then explicitly resume"
        ));
        task.revision += 1;
        save(tx, &task)?;
        changed.push(task);
    }
    Ok(changed)
}
