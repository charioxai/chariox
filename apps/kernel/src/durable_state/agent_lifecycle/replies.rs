//! A turn can supply a requested reply only when the request started it.
use super::*;

pub(super) fn bind(
    tx: &Transaction<'_>,
    event: &InboxEvent,
    target: Option<&str>,
    prompt: &str,
) -> Result<(), DaemonError> {
    if !event.reply_requested {
        return Ok(());
    }
    let sequence = sql_integer(event.sequence)?;
    tx.execute(
        "DELETE FROM agent_reply_turns WHERE sequence=?1",
        [sequence],
    )
    .map_err(sql)?;
    let (task, started_by_request) = if let Some(target) = target {
        let task = for_turn(tx, &event.room_id, &event.agent_id, target)?
            .ok_or_else(|| error("steered reply request requires an admitted receiver task"))?;
        (task.task_id, false)
    } else {
        // The ordinary message task already uses the request as its source.
        // A leased retry starts that task with a different transport prompt.
        if prompt == request_source(event) {
            return Ok(());
        }
        (prompt.to_owned(), true)
    };
    tx.execute(
        "INSERT INTO agent_reply_turns VALUES(?1,?2,?3)",
        params![sequence, task, started_by_request],
    )
    .map_err(sql)?;
    Ok(())
}

fn request_source(event: &InboxEvent) -> String {
    format!("agent-event-{}-{}", event.agent_id, event.sequence)
}

/// A message to another room lives in the recipient's room, so its reply outcome is recorded
/// there; the sender's task waits on the same source in the sender's own room.
pub(super) fn forward_to_sender_room(
    tx: &Transaction<'_>,
    room: &str,
    source: &str,
    occurrence: &str,
    success: bool,
    public_answer: Option<serde_json::Value>,
    now: u64,
) -> Result<(), DaemonError> {
    let sender_room: Option<String> = tx
        .query_row(
            "SELECT json_extract(payload,'$.payload.sender_room_id') FROM agent_inbox WHERE room_id=?1 AND json_extract(payload,'$.kind')='message' AND 'agent-event-'||agent_id||'-'||sequence=?2",
            params![room, source],
            |r| r.get::<_, Option<String>>(0),
        )
        .optional()
        .map_err(sql)?
        .flatten();
    match sender_room {
        Some(sender_room) if sender_room != room => super::supervision::apply(
            tx,
            Operation::SourceOutcome {
                now,
                room: sender_room,
                source: source.into(),
                occurrence: occurrence.into(),
                success,
                public_answer,
            },
        )
        .map(|_| ()),
        _ => Ok(()),
    }
}
/// Remember when a request was read during other work so that turn's end can
/// close the request without exposing the turn's answer.
pub(super) fn read_in_turn(
    tx: &Transaction<'_>,
    room: &str,
    agent: &str,
    sequence: u64,
) -> Result<(), DaemonError> {
    let asks_reply: bool = tx
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM agent_inbox WHERE room_id=?1 AND agent_id=?2 AND sequence=?3 AND json_extract(payload,'$.reply_requested')=1 AND json_extract(payload,'$.kind')='message' AND json_extract(payload,'$.prompt_id') IS NULL)",
            params![room, agent, sql_integer(sequence)?],
            |r| r.get(0),
        )
        .map_err(sql)?;
    if !asks_reply {
        return Ok(());
    }
    let receiver = live_tasks(tx, room, agent)?
        .into_iter()
        .rfind(|task| task.state == ExecutionState::Working);
    if let Some(task) = receiver {
        // A read does not replace a binding established by delivery, including
        // a leased retry whose task id differs from the request source.
        tx.execute(
            "INSERT INTO agent_reply_turns VALUES(?1,?2,0) ON CONFLICT(sequence) DO NOTHING",
            params![sql_integer(sequence)?, task.task_id],
        )
        .map_err(sql)?;
    }
    Ok(())
}

pub(super) fn reconcile_source(
    tx: &Transaction<'_>,
    room: &str,
    source: &str,
) -> Result<(), DaemonError> {
    let mut query = tx.prepare(
        "SELECT i.sequence,i.agent_id FROM agent_reply_turns l JOIN agent_inbox i ON i.sequence=l.sequence WHERE l.task_id=?1 AND i.room_id=?2",
    ).map_err(sql)?;
    let requests = query
        .query_map(params![source, room], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
        })
        .map_err(sql)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(sql)?;
    drop(query);
    for (sequence, recipient) in requests {
        let sequence = u64::try_from(sequence).map_err(|_| error("corrupt reply sequence"))?;
        reconcile_event(tx, &get_event(tx, room, &recipient, sequence)?)?;
    }
    Ok(())
}
pub(super) fn reconcile_event(tx: &Transaction<'_>, event: &InboxEvent) -> Result<(), DaemonError> {
    if !event.reply_requested
        || !matches!(
            event.state.as_str(),
            "accepted" | "acknowledged" | "handled"
        )
    {
        return Ok(());
    }
    let proof: Option<(bool, bool, Option<String>)> = tx.query_row(
        "SELECT l.started_by_request,o.success,o.public_answer FROM agent_reply_turns l JOIN agent_source_occurrences o ON o.source_id=l.task_id AND o.room_id=?2 WHERE l.sequence=?1 AND o.occurrence_id='task-terminal-'||l.task_id",
        params![sql_integer(event.sequence)?,event.room_id],
        |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?)),
    ).optional().map_err(sql)?;
    if let Some((started_by_request, turn_succeeded, answer)) = proof {
        let answer = match (started_by_request, turn_succeeded) {
            (_, false) => Some(DelegateGoneReason::SourceLost.answer(&event.agent_id)),
            (false, true) => Some(ReplyOutcome::no_answer(&event.agent_id)),
            (true, true) => answer.map(|value| decode(&value)).transpose()?,
        };
        settle_request(
            tx,
            event,
            crate::session::unix_epoch_ms(),
            started_by_request && turn_succeeded,
            answer,
        )?;
    }
    Ok(())
}

// Removal is terminal even when the recipient never admitted a receiving task.
pub(super) fn recipient_removed(
    tx: &Transaction<'_>,
    event: &InboxEvent,
    now: u64,
) -> Result<(), DaemonError> {
    recipient_unavailable(tx, event, now, DelegateGoneReason::Deleted)
}
/// The recipient's start failed before any provider I/O, so the request is answered as such.
pub(super) fn recipient_start_failed(
    tx: &Transaction<'_>,
    event: &InboxEvent,
) -> Result<(), DaemonError> {
    recipient_unavailable(
        tx,
        event,
        crate::session::unix_epoch_ms(),
        DelegateGoneReason::FailedStart,
    )
}

/// The person declined a message before its recipient read it.
pub(super) fn recipient_declined(
    tx: &Transaction<'_>,
    event: &InboxEvent,
    now: u64,
) -> Result<(), DaemonError> {
    recipient_unavailable(tx, event, now, DelegateGoneReason::SourceLost)
}
fn recipient_unavailable(
    tx: &Transaction<'_>,
    event: &InboxEvent,
    now: u64,
    reason: DelegateGoneReason,
) -> Result<(), DaemonError> {
    if !event.reply_requested {
        return Ok(());
    }
    settle_request(tx, event, now, false, Some(reason.answer(&event.agent_id)))
}

fn settle_request(
    tx: &Transaction<'_>,
    event: &InboxEvent,
    now: u64,
    success: bool,
    public_answer: Option<serde_json::Value>,
) -> Result<(), DaemonError> {
    let source = request_source(event);
    // Settlement can race a delivery receipt or be replayed after restart.
    let settled: bool = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM agent_source_occurrences WHERE room_id=?1 AND source_id=?2)",
        params![event.room_id, source], |row| row.get(0),
    ).map_err(sql)?;
    if !settled {
        if event.state == "pending" {
            // A terminal reply and removal from automatic retry must commit together.
            super::delivery::apply(
                tx,
                Operation::Expire {
                    room: event.room_id.clone(),
                    agent: event.agent_id.clone(),
                    sequence: event.sequence,
                },
            )?;
        }
        super::supervision::apply(
            tx,
            Operation::SourceOutcome {
                now,
                room: event.room_id.clone(),
                source,
                occurrence: format!("reply-terminal-{}", event.sequence),
                success,
                public_answer,
            },
        )?;
    }
    Ok(())
}
// Match the real recipient receipt, never parse an agent id out of a synthetic source.
pub(super) fn recipient_lost(
    tx: &Transaction<'_>,
    room: &str,
    recipient: &str,
    now: u64,
    answer: Option<&serde_json::Value>,
) -> Result<(), DaemonError> {
    let reason = match answer
        .cloned()
        .and_then(|a| serde_json::from_value::<ReplyOutcome>(a).ok())
    {
        Some(ReplyOutcome::DelegateGone { reason, .. }) => reason,
        Some(ReplyOutcome::NoAnswer { .. }) | None => DelegateGoneReason::SourceLost,
    };
    let mut query = tx.prepare("SELECT sequence FROM agent_inbox WHERE room_id=?1 AND agent_id=?2 AND json_extract(payload,'$.reply_requested')=1").map_err(sql)?;
    let sequences = query
        .query_map(params![room, recipient], |r| r.get::<_, i64>(0))
        .map_err(sql)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(sql)?;
    drop(query);
    for sequence in sequences {
        let sequence = u64::try_from(sequence).map_err(|_| error("corrupt reply sequence"))?;
        recipient_unavailable(tx, &get_event(tx, room, recipient, sequence)?, now, reason)?;
    }
    Ok(())
}
