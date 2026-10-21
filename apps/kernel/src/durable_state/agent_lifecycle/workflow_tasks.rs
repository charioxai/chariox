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
    let existing: Option<(String, String)> = tx
        .query_row(
            "SELECT run_id,node_run_id FROM agent_task_workflows WHERE task_id=?1",
            [task],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()
        .map_err(sql)?;
    match existing {
        // Continuations of a bound task, including its completion check, keep
        // the lineage they already have; rebinding grants nothing.
        Some(existing) if existing == (run.into(), node.into()) => Ok(Outcome::Saved),
        Some(_) => Err(error("workflow task lineage changed")),
        None => {
            current(&load(tx, task)?, prompt)?;
            tx.execute(
                "INSERT INTO agent_task_workflows VALUES(?1,?2,?3)",
                params![task, run, node],
            )
            .map_err(sql)?;
            Ok(Outcome::Saved)
        }
    }
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

// MP-08 / MP-10 / MP-11: startup can fail before a node acquires a task.
// Settle only the parent's accepted invocation, never an agent-wide source.
pub(super) fn settle_unstarted_delegates(
    tx: &Transaction<'_>,
    task: &mut AgentTaskExecution,
    owner_id: &str,
    source: &str,
) -> Result<bool, DaemonError> {
    if !task.obligations.iter().any(|o| {
        o.tracks_workflow_run()
            && o.dispatch_state == "accepted"
            && o.resource_id.as_deref() == Some(source)
    }) {
        return Ok(false);
    }
    let Some(run) = load_run(tx, owner_id, &task.room_id, source)? else {
        return Ok(false);
    };
    if run.created_by_agent_id() != Some(task.agent_id.as_str()) || !run_failed(&run) {
        return Ok(false);
    }
    // A delegate that another accepted, still-running invocation of this task
    // uses must stay bound to the parent for that invocation.
    let mut kept = std::collections::HashSet::new();
    for other in task.obligations.iter().filter(|o| {
        o.tracks_workflow_run()
            && o.status == "open"
            && o.dispatch_state == "accepted"
            && o.resource_id.as_deref() != Some(source)
    }) {
        let Some(id) = other.resource_id.as_deref() else {
            continue;
        };
        let Some(run) = load_run(tx, owner_id, &task.room_id, id)? else {
            continue;
        };
        if !run_terminated(&run) {
            kept.extend(unfinished_agents(&run));
        }
    }
    let mut changed = false;
    for obligation in &mut task.obligations {
        if obligation.kind != "delegate"
            || obligation.status != "open"
            || obligation.dispatch_state != "accepted"
            || obligation.completion_task_id.is_some()
            || obligation
                .resource_id
                .as_deref()
                .is_some_and(|a| kept.contains(a))
        {
            continue;
        }
        let Some(node) = run.node_runs().iter().find(|n| {
            Some(n.agent_id()) == obligation.resource_id.as_deref()
                && n.status() != crate::session::WorkflowNodeRunStatus::Completed
        }) else {
            continue;
        };
        let admitted: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM agent_task_workflows WHERE run_id=?1 AND node_run_id=?2)",
            params![source, node.id()], |r| r.get(0),
        ).map_err(sql)?;
        if admitted {
            continue;
        }
        obligation.status = "cancelled".into();
        for mut reg in registrations(tx, &task.task_id)?
            .into_iter()
            .filter(|r| r.obligation_id.as_deref() == Some(obligation.id.as_str()) && r.live)
        {
            reg.live = false;
            tx.execute(
                "UPDATE agent_registrations SET payload=?2 WHERE id=?1",
                params![reg.id, encode(&reg)?],
            )
            .map_err(sql)?;
        }
        let mut reason = serde_json::json!({"task_id": task.task_id, "workflow_run_id": source,
            "obligation_id": obligation.id, "source_id": obligation.resource_id, "success": false,
            "reason": "Workflow terminated before this delegate's provider task was admitted",
            "failure": run.failure_events().last().map(|e| e.message().chars().take(2048).collect::<String>()),
            "recovery": "Correct the reported workflow/provider configuration and invoke a new workflow run; this delegate is cancelled"});
        crate::secret_redaction::redact_json_secrets(&mut reason);
        event(
            tx,
            occurrence(
                &task.room_id,
                &task.agent_id,
                source,
                &format!("workflow-unstarted-{source}-{}", obligation.id),
                "source_lost",
                reason,
            ),
        )?;
        changed = true;
    }
    Ok(changed)
}

fn load_run(
    tx: &Transaction<'_>,
    owner_id: &str,
    room: &str,
    run: &str,
) -> Result<Option<crate::session::WorkflowRun>, DaemonError> {
    let raw: Option<String> = tx
        .query_row(
            "SELECT payload_json FROM durable_workflow_runs WHERE owner_id=?1 AND session_id=?2 AND run_id=?3",
            params![owner_id, room, run],
            |r| r.get(0),
        )
        .optional()
        .map_err(sql)?;
    raw.map(|raw| decode(&raw)).transpose()
}

fn run_failed(run: &crate::session::WorkflowRun) -> bool {
    matches!(
        run.status(),
        crate::session::WorkflowRunStatus::Failed | crate::session::WorkflowRunStatus::Stopped
    )
}

fn run_terminated(run: &crate::session::WorkflowRun) -> bool {
    run_failed(run) || run.status() == crate::session::WorkflowRunStatus::Completed
}

/// Agents a surviving run still has to run: started and unfinished nodes plus
/// the nodes the invocation has not reached yet (its runtime agent mapping).
fn unfinished_agents(run: &crate::session::WorkflowRun) -> Vec<String> {
    let done: std::collections::HashSet<&str> = run
        .node_runs()
        .iter()
        .filter(|n| n.status() == crate::session::WorkflowNodeRunStatus::Completed)
        .map(|n| n.agent_id())
        .collect();
    run.node_runs()
        .iter()
        .map(|n| n.agent_id())
        .chain(run.runtime_agent_ids_by_node().values().map(String::as_str))
        .filter(|agent| !done.contains(agent))
        .map(str::to_owned)
        .collect()
}
