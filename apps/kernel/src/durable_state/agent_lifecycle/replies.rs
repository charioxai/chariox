//! MP-08/MP-09/MP-10/MP-11 A02: urgent reply follows its receiving logical task.
use super::*;
pub(super) fn bind(
    tx: &Transaction<'_>,
    event: &InboxEvent,
    target: Option<&str>,
    prompt: &str,
) -> Result<(), DaemonError> {
    tx.execute(
        "DELETE FROM agent_urgent_reply_links WHERE sequence=?1",
        [sql_integer(event.sequence)?],
    )
    .map_err(sql)?;
    if event.reply_requested {
        let task = if let Some(target) = target {
            Some(
                for_turn(tx, &event.room_id, &event.agent_id, target)?
                    .ok_or_else(|| error("urgent reply requires an admitted receiver task"))?
                    .task_id,
            )
        } else {
            // MP-08/MP-10/MP-11 R947-1: a leased retry has a fresh transport
            // prompt, while the sender still awaits the original event source.
            // The idle message task is admitted under this prompt after Attempt.
            let source = format!("agent-event-{}-{}", event.agent_id, event.sequence);
            (prompt != source).then(|| prompt.to_owned())
        };
        if let Some(task) = task {
            tx.execute(
                "INSERT INTO agent_urgent_reply_links VALUES(?1,?2)",
                params![sql_integer(event.sequence)?, task],
            )
            .map_err(sql)?;
        }
    }
    Ok(())
}
pub(super) fn reconcile_source(
    tx: &Transaction<'_>,
    room: &str,
    source: &str,
) -> Result<(), DaemonError> {
    let mut q=tx.prepare("SELECT i.sequence FROM agent_urgent_reply_links l JOIN agent_inbox i ON i.sequence=l.sequence WHERE l.target_task_id=?1 AND i.room_id=?2").map_err(sql)?;
    let sequences = q
        .query_map(params![source, room], |r| r.get::<_, i64>(0))
        .map_err(sql)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(sql)?;
    drop(q);
    for seq in sequences {
        let recipient: String = tx
            .query_row(
                "SELECT agent_id FROM agent_inbox WHERE sequence=?1",
                [seq],
                |r| r.get(0),
            )
            .map_err(sql)?;
        let Ok(event) = get_event(
            tx,
            room,
            &recipient,
            u64::try_from(seq).map_err(|_| error("corrupt reply sequence"))?,
        ) else {
            continue;
        };
        reconcile_event(tx, &event)?;
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
    let proof:Option<(bool,Option<String>)>=tx.query_row("SELECT o.success,o.public_answer FROM agent_urgent_reply_links l JOIN agent_source_occurrences o ON o.source_id=l.target_task_id AND o.room_id=?2 WHERE l.sequence=?1 ORDER BY o.sequence DESC LIMIT 1",params![sql_integer(event.sequence)?,event.room_id],|r|Ok((r.get(0)?,r.get(1)?))).optional().map_err(sql)?;
    if let Some((success, answer)) = proof {
        let source = format!("agent-event-{}-{}", event.agent_id, event.sequence);
        let occurrence = format!("reply-terminal-{}", event.sequence);
        // A reply terminal occurrence is immutable; dedup also bounds recursive fanout.
        let exists:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM agent_source_occurrences WHERE room_id=?1 AND source_id=?2)",params![event.room_id,source],|r|r.get(0)).map_err(sql)?;
        if !exists {
            super::supervision::apply(
                tx,
                Operation::SourceOutcome {
                    now: crate::session::unix_epoch_ms(),
                    room: event.room_id.clone(),
                    source,
                    occurrence,
                    success,
                    public_answer: if success {
                        answer
                            .map(|v| decode::<serde_json::Value>(&v))
                            .transpose()?
                    } else {
                        Some(DelegateGoneReason::SourceLost.answer(&event.agent_id))
                    },
                },
            )?;
        }
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
fn recipient_unavailable(
    tx: &Transaction<'_>,
    event: &InboxEvent,
    now: u64,
    reason: DelegateGoneReason,
) -> Result<(), DaemonError> {
    if !event.reply_requested {
        return Ok(());
    }
    let source = format!("agent-event-{}-{}", event.agent_id, event.sequence);
    let settled: bool = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM agent_source_occurrences WHERE room_id=?1 AND source_id=?2)",
        params![event.room_id, source], |row| row.get(0),
    ).map_err(sql)?;
    if !settled {
        super::supervision::apply(
            tx,
            Operation::SourceOutcome {
                now,
                room: event.room_id.clone(),
                source,
                occurrence: format!("reply-terminal-{}", event.sequence),
                success: false,
                public_answer: Some(reason.answer(&event.agent_id)),
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
        None => DelegateGoneReason::SourceLost,
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
