//! A delivery refused before any provider I/O ended nowhere: the sweep settles it as failed.
//! Leaving it blocked as "unconfirmed" would fence every later wake and message of the recipient.
use super::*;

/// Ends a refused, never submitted delivery as failed and tells whoever waits for it:
/// the sender of a message and the wake receipt.
pub(super) fn fail_undelivered(
    tx: &Transaction<'_>,
    e: &mut InboxEvent,
    reason: DelegateGoneReason,
    now: u64,
) -> Result<(), DaemonError> {
    e.state = "failed".into();
    save_event(tx, e)?;
    wakes::record_delivery(tx, e)?;
    if e.kind != "message" {
        return Ok(());
    }
    if e.reply_requested {
        replies::recipient_unavailable(tx, e, now, reason)?;
        if reason != DelegateGoneReason::AgentDeadlineReached {
            return Ok(());
        }
    }
    tell_sender(tx, e, reason)
}

/// The deadline receipt and its message outcomes commit together. A queued message keeps its
/// original payload for Inbox reads, even after its delivery finishes.
pub(super) fn expire_deadline_messages(
    tx: &Transaction<'_>,
    room: &str,
    agent: &str,
    now: u64,
) -> Result<(), DaemonError> {
    let mut query = tx
        .prepare("SELECT payload FROM agent_inbox WHERE room_id=?1 AND agent_id=?2
            AND json_extract(payload,'$.kind')='message' AND json_extract(payload,'$.state')='pending'
            AND NOT EXISTS(SELECT 1 FROM agent_inbox_shown WHERE sequence=agent_inbox.sequence)
            ORDER BY sequence")
        .map_err(sql)?;
    let pending = query
        .query_map(params![room, agent], |row| row.get::<_, String>(0))
        .map_err(sql)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(sql)?;
    for payload in pending {
        let mut event = decode(&payload)?;
        fail_undelivered(
            tx,
            &mut event,
            DelegateGoneReason::AgentDeadlineReached,
            now,
        )?;
    }
    Ok(())
}

fn tell_sender(
    tx: &Transaction<'_>,
    e: &InboxEvent,
    reason: DelegateGoneReason,
) -> Result<(), DaemonError> {
    // A message to another room lives in the recipient's room and names the sender's room.
    let sender_room = e.payload["sender_room_id"].as_str().unwrap_or(&e.room_id);
    // A sender without a task there is not an agent of this kernel to tell.
    let known: bool = tx
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM agent_tasks WHERE room_id=?1 AND agent_id=?2)",
            params![sender_room, e.source_id],
            |r| r.get(0),
        )
        .map_err(sql)?;
    if !known {
        return Ok(());
    }
    let source = format!("agent-event-{}-{}", e.agent_id, e.sequence);
    let id = if e.reply_requested {
        format!("reply-terminal-{}", e.sequence)
    } else {
        format!("undelivered-{}", e.sequence)
    };
    // If the sender still had a reply registration, its terminal outcome already told it.
    let told: bool = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM agent_inbox WHERE room_id=?1 AND agent_id=?2 AND source_id=?3 AND occurrence_id=?4)",
        params![sender_room, e.source_id, source, id],
        |row| row.get(0),
    ).map_err(sql)?;
    if told {
        return Ok(());
    }
    event(
        tx,
        occurrence(
            sender_room,
            &e.source_id,
            &source,
            &id,
            "source_lost",
            serde_json::json!({
                "source_id": source,
                "public_history_ref": source,
                "occurrence_id": id,
                "success": false,
                "public_answer": reason.answer(&e.agent_id),
            }),
        ),
    )?;
    Ok(())
}
