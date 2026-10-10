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
        let exists:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM agent_source_occurrences WHERE room_id=?1 AND source_id=?2 AND occurrence_id=?3)",params![event.room_id,source,occurrence],|r|r.get(0)).map_err(sql)?;
        if !exists {
            super::supervision::apply(
                tx,
                Operation::SourceOutcome {
                    room: event.room_id.clone(),
                    source,
                    occurrence,
                    success,
                    public_answer: answer
                        .map(|v| decode::<serde_json::Value>(&v))
                        .transpose()?,
                    now: crate::session::unix_epoch_ms(),
                },
            )?;
        }
    }
    Ok(())
}
