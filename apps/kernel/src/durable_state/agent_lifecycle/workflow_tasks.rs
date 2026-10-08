//! MP-08 / MP-09 / MP-10 / MP-11: private durable workflow/task lineage.
use super::*;

pub(super) fn initialize(db: &Connection) -> Result<(), DaemonError> {
    db.execute_batch("CREATE TABLE IF NOT EXISTS agent_task_workflows(task_id TEXT PRIMARY KEY,run_id TEXT NOT NULL,node_run_id TEXT NOT NULL);").map_err(sql)
}

pub(super) fn bind(
    tx: &Transaction<'_>,
    task: &str,
    prompt: &str,
    run: &str,
    node: &str,
) -> Result<Outcome, DaemonError> {
    current(&load(tx, task)?, prompt)?;
    tx.execute(
        "INSERT INTO agent_task_workflows VALUES(?1,?2,?3) ON CONFLICT(task_id) DO NOTHING",
        params![task, run, node],
    )
    .map_err(sql)?;
    let existing: (String, String) = tx
        .query_row(
            "SELECT run_id,node_run_id FROM agent_task_workflows WHERE task_id=?1",
            [task],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .map_err(sql)?;
    if existing != (run.into(), node.into()) {
        return Err(error("workflow task lineage changed"));
    }
    Ok(Outcome::Saved)
}

impl DurableKernelStateStore {
    pub(crate) fn agent_task_workflow(
        &self,
        task: &str,
    ) -> Result<Option<(String, String)>, DaemonError> {
        self.lock_connection("agent.lifecycle.workflow")?
            .query_row(
                "SELECT run_id,node_run_id FROM agent_task_workflows WHERE task_id=?1",
                [task],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()
            .map_err(sql)
    }
}
