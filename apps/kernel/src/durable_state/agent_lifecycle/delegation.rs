//! MP-08/MP-10/MP-11 F2/F4: completion follows a parent-submitted child task.
use super::*;
pub(super) fn bind(
    tx: &Transaction<'_>,
    parent_id: &str,
    child_id: &str,
) -> Result<(), DaemonError> {
    bind_admitted(tx, parent_id, child_id, false)
}

/// Cross-Room binding is used only for an instruction the kernel admitted from the child's parent.
fn bind_admitted(
    tx: &Transaction<'_>,
    parent_id: &str,
    child_id: &str,
    cross_room_child: bool,
) -> Result<(), DaemonError> {
    let mut parent = load(tx, parent_id)?;
    let child = load(tx, child_id)?;
    if !cross_room_child
        && (parent.room_id != child.room_id
            || parent.owner_user_id.is_empty()
            || parent.owner_user_id != child.owner_user_id)
    {
        return Err(error("delegation task binding is foreign or closed"));
    }
    // Cancellation after admission cannot turn an accepted provider prompt into an error. A Done
    // parent still binds the delegate it retained: its work message may be delivered after its turn.
    if !matches!(
        parent.state,
        ExecutionState::Working | ExecutionState::Waiting | ExecutionState::Done
    ) {
        return Ok(());
    }
    if parent
        .obligations
        .iter()
        .any(|o| o.completion_task_id.as_deref() == Some(child_id))
    {
        return Ok(());
    }
    let Some(obligation) = parent.obligations.iter_mut().find(|o| {
        o.kind == "delegate"
            && o.status == "open"
            && o.dispatch_state == "accepted"
            && o.resource_id.as_deref() == Some(child.agent_id.as_str())
            && o.completion_task_id.is_none()
    }) else {
        return Ok(());
    };
    obligation.completion_task_id = Some(child.task_id.clone());
    let obligation_id = obligation.id.clone();
    for mut reg in registrations(tx, &parent.task_id)?
        .into_iter()
        .filter(|r| r.obligation_id.as_deref() == Some(obligation_id.as_str()))
    {
        reg.source_id = child.task_id.clone();
        reg.source_cursor = 0;
        tx.execute(
            "UPDATE agent_registrations SET payload=?2 WHERE id=?1",
            params![reg.id, encode(&reg)?],
        )
        .map_err(sql)?;
    }
    parent.revision += 1;
    save(tx, &parent)?;
    // A fast child may have settled before its admission receipt arrived.
    for reg in registrations(tx, &parent.task_id)?
        .into_iter()
        .filter(|r| r.live && r.obligation_id.as_deref() == Some(obligation_id.as_str()))
    {
        transitions::recover_source(tx, &mut parent, reg)?;
    }
    Ok(())
}
pub(super) fn bind_message(tx: &Transaction<'_>, e: &InboxEvent) -> Result<(), DaemonError> {
    if e.kind != "message"
        || e.target_prompt_id.is_some()
        || !matches!(e.state.as_str(), "accepted" | "acknowledged" | "handled")
    {
        return Ok(());
    }
    let parent: Option<String> = tx
        .query_row(
            "SELECT task_id FROM agent_delegation_messages WHERE sequence=?1",
            [sql_integer(e.sequence)?],
            |r| r.get(0),
        )
        .optional()
        .map_err(sql)?;
    if let (Some(parent), Some(prompt)) = (parent, e.prompt_id.as_deref()) {
        let cross_room_child = e.payload["cross_room_child"] == true;
        if load(tx, &parent)?.room_id == e.room_id || cross_room_child {
            if let Some(child) = for_turn(tx, &e.room_id, &e.agent_id, prompt)? {
                bind_admitted(tx, &parent, &child.task_id, cross_room_child)?;
            }
        }
    }
    Ok(())
}
pub(super) fn unbound(task: &AgentTaskExecution, reg: &Registration) -> bool {
    task.obligations.iter().any(|o| {
        reg.obligation_id.as_deref() == Some(o.id.as_str())
            && o.kind == "delegate"
            && o.dispatch_state == "accepted"
            && o.completion_task_id.is_none()
    })
}
