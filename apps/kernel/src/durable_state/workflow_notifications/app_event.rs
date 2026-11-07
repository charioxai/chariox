//! An App's incoming event as a notification source: the inbox accepts an occurrence on a route
//! and, in the same commit, every grant that follows the route's source receives it. An event
//! nobody follows is recorded by the inbox for the App and queued for no one else.
use super::*;
use crate::session::WorkflowOutputPayload;
use chariox_app_runtime::app_inbox::InboxRoute;

/// Longest event text a notification carries; the App's own copy keeps the whole payload.
const TEXT_BYTES: usize = 32 * 1024;

/// A route's source is identified by its installation (the source's `session_id`), the route (its
/// `workflow_id`) and the incoming event the route accepts (its `exit_node_id`).
pub(crate) fn emit_in(
    tx: &Transaction<'_>,
    route: &InboxRoute,
    occurrence_id: &str,
    payload: &serde_json::Value,
    now_ms: u64,
) -> Result<(), DaemonError> {
    let source_json: Option<String> = tx.query_row("SELECT payload_json FROM workflow_notification_exit_sources WHERE owner_id=?1 AND session_id=?2 AND workflow_id=?3 AND exit_node_id=?4 AND enabled=1",params![route.owner_id,route.installation_id,route.route_id,exit_key(crate::local::NotificationSourceKind::AppEvent,&route.event_name)],|r|r.get(0)).optional().map_err(sql)?;
    let Some(source_json) = source_json else {
        return Ok(());
    };
    let source: WorkflowNotificationSource = decode(&source_json)?;
    let inserted = tx
        .execute(
            "INSERT OR IGNORE INTO workflow_notification_occurrences VALUES (?1,?2,NULL)",
            params![source.source_id, occurrence_id],
        )
        .map_err(sql)?;
    prune_occurrences(tx).map_err(sql)?;
    if inserted == 0 {
        return Ok(());
    }
    completion::publish_in(
        tx,
        &source,
        occurrence_id,
        WorkflowOutputPayload::new(text_of(payload), Vec::new()),
        Vec::new(),
        now_ms,
    )
}

/// The event's own text, cut at a character boundary.
fn text_of(payload: &serde_json::Value) -> String {
    let text = payload["text"].as_str().unwrap_or_default();
    let mut end = text.len().min(TEXT_BYTES);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    text[..end].to_owned()
}
