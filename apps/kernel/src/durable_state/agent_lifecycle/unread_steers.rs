//! MP-08 / MP-09 / MP-10 / MP-11: a steered row its turn never read goes back to pending.
use super::*;

/// A row nobody reads is given back at most this many times, so it cannot loop.
const MAX_UNREAD_REVERTS: i64 = 2;

/// A turn that completes without calling `events_inbox` leaves the rows steered into it `accepted` and
/// unread: nothing would wake the agent for them. They become ordinary pending rows, delivered by the next
/// idle sweep as one wake. A row the agent was shown, or that is already settled, is never touched.
pub(super) fn revert_for_prompt(
    tx: &Transaction<'_>,
    room: &str,
    agent: &str,
    prompt: &str,
) -> Result<(), DaemonError> {
    let mut q = tx
        .prepare("SELECT sequence FROM agent_inbox WHERE room_id=?1 AND agent_id=?2 AND json_extract(payload,'$.state')='accepted' AND json_extract(payload,'$.target_prompt_id')=?3 AND NOT EXISTS(SELECT 1 FROM agent_inbox_shown s WHERE s.sequence=agent_inbox.sequence) ORDER BY sequence")
        .map_err(sql)?;
    let sequences = q
        .query_map(params![room, agent, prompt], |r| r.get::<_, i64>(0))
        .map_err(sql)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(sql)?;
    #[cfg(test)]
    INBOX_LOOKUP_STEPS.with(|n| n.set(n.get() + q.get_status(rusqlite::StatementStatus::VmStep)));
    drop(q);
    for sequence in sequences {
        let sequence = u64::try_from(sequence).map_err(|_| error("corrupt inbox sequence"))?;
        let mut event = get_event(tx, room, agent, sequence)?;
        revert(tx, &mut event)?;
    }
    Ok(())
}

/// The provider accepted a steer after its turn had already settled: the row was never going to be read.
pub(super) fn revert_late_receipt(
    tx: &Transaction<'_>,
    event: &mut InboxEvent,
) -> Result<(), DaemonError> {
    let Some(target) = event.target_prompt_id.as_deref() else {
        return Ok(());
    };
    // A Blocked task is held by a delivery decision, not finished: its turn may still read the row.
    let settled = for_turn(tx, &event.room_id, &event.agent_id, target)?.is_some_and(|task| {
        matches!(
            task.state,
            ExecutionState::Done | ExecutionState::Cancelled | ExecutionState::Waiting
        )
    });
    if settled && !shown(tx, event.sequence)? {
        revert(tx, event)?;
    }
    Ok(())
}

fn shown(tx: &Transaction<'_>, sequence: u64) -> Result<bool, DaemonError> {
    tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM agent_inbox_shown WHERE sequence=?1)",
        [sql_integer(sequence)?],
        |r| r.get(0),
    )
    .map_err(sql)
}

fn revert(tx: &Transaction<'_>, event: &mut InboxEvent) -> Result<(), DaemonError> {
    let sequence = sql_integer(event.sequence)?;
    let count: i64 = tx
        .query_row(
            "SELECT count FROM agent_inbox_unread_reverts WHERE sequence=?1",
            [sequence],
            |r| r.get(0),
        )
        .optional()
        .map_err(sql)?
        .unwrap_or(0);
    if count >= MAX_UNREAD_REVERTS {
        return Ok(());
    }
    tx.execute(
        "INSERT INTO agent_inbox_unread_reverts(sequence,count) VALUES(?1,1) ON CONFLICT(sequence) DO UPDATE SET count=count+1",
        [sequence],
    )
    .map_err(sql)?;
    // The fields a refused steer clears; `attempted_at_ms` stays so the row is not steered again.
    event.state = "pending".into();
    event.prompt_id = None;
    event.target_prompt_id = None;
    event.provider_run_id = None;
    event.submit_epoch = None;
    save_event(tx, event)?;
    super::wakes::record_delivery(tx, event)?;
    super::delegation::bind_message(tx, event)?;
    replies::reconcile_event(tx, event)
}
