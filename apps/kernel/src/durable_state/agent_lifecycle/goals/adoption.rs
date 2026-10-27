//! MP-08 / MP-09 / MP-10 / MP-11 A12: shared goal adoption policy.
use super::*;
/// An owner-confirmed migration tracks existing resources; it never dispatches
/// a template node. The immutable run-derived task id is the atomic fence.
#[allow(clippy::too_many_arguments)]
pub(in crate::durable_state::agent_lifecycle) fn adopt(
    tx: &Transaction<'_>,
    owner: String,
    room: String,
    agent: String,
    run: String,
    nodes: Vec<String>,
    objective: String,
    now: u64,
) -> Result<Outcome, DaemonError> {
    let id = format!("goal-adopt-{run}");
    if let Some(t) = tasks(tx)?.into_iter().find(|t| t.task_id == id) {
        if t.owner_user_id == owner
            && t.room_id == room
            && t.agent_id == agent
            && t.goal.as_ref().is_some_and(|g| {
                g.legacy_run_id.as_deref() == Some(run.as_str()) && g.objective == objective
            })
        {
            return Ok(Outcome::Task(t));
        }
        return Err(error("conflicting legacy goal adoption"));
    }
    if owner.is_empty() || room.is_empty() || agent.is_empty() || run.is_empty() || nodes.is_empty()
    {
        return Err(error(
            "legacy adoption requires authoritative owner, room, entry agent and nodes",
        ));
    }
    let mut t = new_admitted_task(tx, room, agent, id.clone(), None, now)?;
    t.owner_user_id = owner.clone();
    save(tx, &t)?;
    start(tx, id.clone(), owner, objective, 10, now)?;
    t = load(tx, &id)?;
    t.goal.as_mut().unwrap().legacy_run_id = Some(run.clone());
    let mut ids = vec![format!("legacy-run-{run}")];
    ids.extend(
        nodes
            .into_iter()
            .map(|node| format!("legacy-node-{run}-{node}")),
    );
    ids.sort();
    ids.dedup();
    let mut registration_ids = vec![];
    for obligation in ids {
        t.obligations.push(AgentObligation {
            id: obligation.clone(),
            kind: "workflow_run".into(),
            resource_id: Some(run.clone()),
            completion_task_id: None,
            status: "open".into(),
            dispatch_state: "accepted".into(),
        });
        let reg = Registration {
            id: format!("completion-{obligation}"),
            task_id: id.clone(),
            source_id: run.clone(),
            obligation_id: Some(obligation),
            source_cursor: 0,
            live: true,
        };
        tx.execute(
            "INSERT INTO agent_registrations VALUES(?1,?2,?3)",
            params![reg.id, reg.task_id, encode(&reg)?],
        )
        .map_err(sql)?;
        registration_ids.push(reg.id);
    }
    let due = now.saturating_add(600_000);
    t.state = ExecutionState::Waiting;
    t.reason = "Owner adopted legacy invocation; existing run/nodes drain without relaunch".into();
    t.wait = Some(AgentWait {
        registration_ids,
        deadline_ms: due,
        started_at_ms: now,
        inbox_cursor: 0,
        long_wait_notified: false,
        last_checked_at_ms: now,
    });
    t.goal.as_mut().unwrap().next_check_in_ms = Some(due);
    t.revision += 1;
    migration::audit(
        tx,
        "goal.legacy_adopted",
        &run,
        serde_json::json!({"goal_id":t.goal.as_ref().unwrap().id,
        "task_id":id,"room_id":t.room_id,"owner_user_id":t.owner_user_id,"agent_id":t.agent_id,"legacy_run_id":run,
        "original_invocation_ref":format!("workflow-run:{run}"),"obligations":t.obligations}),
    )?;
    save(tx, &t)?;
    Ok(Outcome::Task(t))
}
