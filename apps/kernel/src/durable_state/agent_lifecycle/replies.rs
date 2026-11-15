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
/// A reply_requested question has no message turn of its own once its recipient reads it inside a
/// running task: that task is what answers it, by a message or by the way it ends.
pub(super) fn read_in_turn(
    tx: &Transaction<'_>,
    room: &str,
    agent: &str,
    sequence: u64,
) -> Result<(), DaemonError> {
    let asks_reply: bool = tx
        .query_row(
            "SELECT COALESCE(json_extract(payload,'$.reply_requested'),0)=1 AND json_extract(payload,'$.kind')='message' FROM agent_inbox WHERE room_id=?1 AND agent_id=?2 AND sequence=?3",
            params![room, agent, sql_integer(sequence)?],
            |r| r.get(0),
        )
        .optional()
        .map_err(sql)?
        .unwrap_or(false);
    let reader = live_tasks(tx, room, agent)?
        .into_iter()
        .rfind(|task| task.state == ExecutionState::Working);
    if let (true, Some(task)) = (asks_reply, reader) {
        tx.execute(
            "INSERT OR REPLACE INTO agent_read_questions VALUES(?1,?2)",
            params![sql_integer(sequence)?, task.task_id],
        )
        .map_err(sql)?;
    }
    Ok(())
}
/// A reply_requested question its recipient read inside a running turn (shown by an inbox read, or
/// already acknowledged) has no message turn of its own whose end could answer it: the recipient's
/// next message to the asker is that answer.
pub(super) fn answer_by_message(
    tx: &Transaction<'_>,
    reply: &InboxEvent,
) -> Result<(), DaemonError> {
    // The question lives in the replier's inbox, in the room the reply was sent from.
    let replier_room = reply.payload["sender_room_id"]
        .as_str()
        .unwrap_or(&reply.room_id);
    let question: Option<(i64, String)> = tx
        .query_row(
            "SELECT q.sequence,json_extract(q.payload,'$.state') FROM agent_inbox q WHERE q.room_id=?1 AND q.agent_id=?2 AND q.source_id=?3 \
             AND json_extract(q.payload,'$.kind')='message' AND json_extract(q.payload,'$.reply_requested')=1 \
             AND (json_extract(q.payload,'$.state') IN ('accepted','acknowledged','handled') \
                  OR EXISTS(SELECT 1 FROM agent_inbox_shown s WHERE s.sequence=q.sequence)) \
             AND NOT EXISTS(SELECT 1 FROM agent_source_occurrences o WHERE o.room_id=q.room_id AND o.source_id='agent-event-'||q.agent_id||'-'||q.sequence) \
             ORDER BY q.sequence LIMIT 1",
            params![replier_room, reply.source_id, reply.agent_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()
        .map_err(sql)?;
    let Some((question, state)) = question else {
        return Ok(());
    };
    let text = reply.payload["message"].as_str().unwrap_or_default();
    let excerpt = &text[..text.floor_char_boundary(4_000)];
    super::supervision::apply(
        tx,
        Operation::SourceOutcome {
            now: crate::session::unix_epoch_ms(),
            room: replier_room.into(),
            source: format!("agent-event-{}-{question}", reply.source_id),
            occurrence: format!("reply-terminal-{question}"),
            success: true,
            public_answer: Some(serde_json::json!({
                "agent_id": reply.source_id,
                "reply_sequence": reply.sequence,
                "excerpt": excerpt,
            })),
        },
    )?;
    // Answering the question handles it: it is not delivered again as a turn of its own.
    if !matches!(state.as_str(), "acknowledged" | "handled") {
        super::delivery::apply(
            tx,
            Operation::Ack {
                room: replier_room.into(),
                agent: reply.source_id.clone(),
                sequence: u64::try_from(question)
                    .map_err(|_| error("corrupt question sequence"))?,
                handled: true,
                now: crate::session::unix_epoch_ms(),
            },
        )?;
    }
    Ok(())
}
pub(super) fn reconcile_source(
    tx: &Transaction<'_>,
    room: &str,
    source: &str,
) -> Result<(), DaemonError> {
    let mut q=tx.prepare("SELECT i.sequence FROM (SELECT sequence,target_task_id FROM agent_urgent_reply_links UNION SELECT sequence,task_id FROM agent_read_questions) l JOIN agent_inbox i ON i.sequence=l.sequence WHERE l.target_task_id=?1 AND i.room_id=?2").map_err(sql)?;
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
    let proof:Option<(bool,Option<String>)>=tx.query_row("SELECT o.success,o.public_answer FROM (SELECT sequence,target_task_id FROM agent_urgent_reply_links UNION SELECT sequence,task_id FROM agent_read_questions) l JOIN agent_source_occurrences o ON o.source_id=l.target_task_id AND o.room_id=?2 WHERE l.sequence=?1 ORDER BY o.sequence DESC LIMIT 1",params![sql_integer(event.sequence)?,event.room_id],|r|Ok((r.get(0)?,r.get(1)?))).optional().map_err(sql)?;
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
