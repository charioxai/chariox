//! Which agents something they registered can still wake.
use super::*;
use std::collections::BTreeSet;

impl DurableKernelStateStore {
    /// The (room, agent) pairs that have an armed wake (kernel timer, watched
    /// process, terminal or driver source), a live registration (a subscription
    /// to a delegate, workflow or task completion; a follow of a peer that outlived the turn
    /// that made it is not one), a task waiting on a
    /// deadline, or a task blocked on an owner decision. A fired, cancelled,
    /// lost or expired source leaves no row that matches.
    pub(crate) fn agents_awaiting_wake(&self) -> Result<BTreeSet<(String, String)>, DaemonError> {
        let armed = wakes::ARMED
            .iter()
            .map(|state| format!("'{state}'"))
            .collect::<Vec<_>>()
            .join(",");
        let db = self.lock_connection("agent.lifecycle.awaiting_wake")?;
        let mut q = db
            .prepare_cached(&format!(
                "SELECT room_id,agent_id FROM agent_wakes WHERE json_extract(payload,'$.state') IN ({armed}) \
                 UNION SELECT t.room_id,t.agent_id FROM agent_registrations r JOIN agent_tasks t ON t.task_id=r.task_id \
                 WHERE CASE WHEN json_valid(r.payload) THEN json_extract(r.payload,'$.live') END=1 \
                 AND NOT (CASE WHEN json_valid(r.payload) THEN json_extract(r.payload,'$.peer') END=1 AND t.state IN ('done','cancelled')) \
                 UNION SELECT room_id,agent_id FROM agent_tasks WHERE state='waiting' \
                 OR (state='blocked' AND CASE WHEN json_valid(payload) THEN COALESCE(json_extract(payload,'$.goal.paused'),0) END=0)"
            ))
            .map_err(sql)?;
        let rows = q
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
            .map_err(sql)?;
        rows.collect::<Result<_, _>>().map_err(sql)
    }
}
