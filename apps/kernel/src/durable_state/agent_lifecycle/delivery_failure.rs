//! A delivery refused before any provider I/O ended nowhere: the sweep settles it as failed.
//! Leaving it blocked as "unconfirmed" would fence every later wake and message of the recipient.
use super::*;

/// Ends a refused, never submitted delivery as failed and tells whoever waits for it:
/// the sender of a message and the wake receipt. The caller asks the owner.
pub(super) fn fail_undelivered(
    tx: &Transaction<'_>,
    e: &mut InboxEvent,
) -> Result<(), DaemonError> {
    e.state = "failed".into();
    save_event(tx, e)?;
    wakes::record_delivery(tx, e)?;
    if e.kind != "message" {
        return Ok(());
    }
    if e.reply_requested {
        replies::recipient_start_failed(tx, e)
    } else {
        tell_sender(tx, e)
    }
}

fn tell_sender(tx: &Transaction<'_>, e: &InboxEvent) -> Result<(), DaemonError> {
    // A sender without a task here is not an agent of this room to tell.
    let known: bool = tx
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM agent_tasks WHERE room_id=?1 AND agent_id=?2)",
            params![e.room_id, e.source_id],
            |r| r.get(0),
        )
        .map_err(sql)?;
    if !known {
        return Ok(());
    }
    let source = format!("agent-event-{}-{}", e.agent_id, e.sequence);
    let id = format!("undelivered-{}", e.sequence);
    event(
        tx,
        occurrence(
            &e.room_id,
            &e.source_id,
            &source,
            &id,
            "source_lost",
            serde_json::json!({
                "source_id": source,
                "public_history_ref": source,
                "occurrence_id": id,
                "success": false,
                "public_answer": DelegateGoneReason::FailedStart.answer(&e.agent_id),
            }),
        ),
    )?;
    Ok(())
}

/// The owner's Resume of an unconfirmed delivery: it goes out again as a fresh attempt.
pub(super) fn send_again(tx: &Transaction<'_>, e: &mut InboxEvent) -> Result<(), DaemonError> {
    e.state = "pending".into();
    e.prompt_id = None;
    e.target_prompt_id = None;
    e.provider_run_id = None;
    e.submit_epoch = None;
    e.attempted_at_ms = None;
    tx.execute(
        "DELETE FROM agent_inbox_refusals WHERE sequence=?1",
        [sql_integer(e.sequence)?],
    )
    .map_err(sql)?;
    save_event(tx, e)?;
    wakes::record_delivery(tx, e)
}
