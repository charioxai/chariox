//! An outside agent's grant as a notification subscriber: its "queue" is the grant's own
//! event stream, so delivery is one append that settles the receipt in the same commit.
use super::*;
use crate::local::{
    EventRefs, NotificationSourceKind, OutsideEvent, OutsideEventValue,
    WorkflowNotificationTargetKind::OutsideAgent,
};

/// Sources one grant may follow; a coordinator follows a handful.
pub(crate) const MAX_FOLLOWED_SOURCES: i64 = 64;

/// A follow that would pass the grant's limit, or follow into an ended inbox, fails, and the
/// attach rolls back with it.
pub(super) fn require_follow_room(
    tx: &Transaction<'_>,
    grant: &str,
    sub: &WorkflowNotificationSubscription,
) -> Result<(), DaemonError> {
    if sub.target_kind != OutsideAgent {
        return Ok(());
    }
    crate::durable_state::agent_lifecycle::outside::require_open_inbox(tx, grant)?;
    let followed: i64 = tx.query_row("SELECT count(*) FROM app_automations WHERE source_kind='workflow_completion' AND status='active' AND publication_id=?1 AND json_extract(notification_json,'$.target_kind')='outside_agent'",[grant],|r|r.get(0)).map_err(sql)?;
    if followed > MAX_FOLLOWED_SOURCES {
        return Err(error("event source limit"));
    }
    Ok(())
}

pub(super) fn append_in(
    tx: &Transaction<'_>,
    sub: &WorkflowNotificationSubscription,
    env: &WorkflowNotificationEnvelope,
) -> Result<NotificationOutcome, DaemonError> {
    if sub.target_kind != OutsideAgent {
        return Err(error("not an outside agent subscription"));
    }
    require_attached(tx, sub, &env.source_id)?;
    // Only an accepted receipt appends, so a repeated attempt finds it settled and adds nothing.
    let settled = tx.execute("UPDATE app_outbox SET state='queued' WHERE source_kind='workflow_completion' AND owner_id=?1 AND installation_id=?2 AND automation_id=?3 AND occurrence_id=?4 AND state='accepted'",params![sub.owner_user_id,env.source_id,sub.subscription_id,env.occurrence_id]).map_err(sql)?;
    if settled == 0 {
        return Ok(NotificationOutcome::Queued);
    }
    let source = load_source(tx, &sub.source_id)?;
    let (agent_id, refs, summary) = match source.kind {
        NotificationSourceKind::WorkflowExit => (
            None,
            EventRefs::default(),
            format!("{} finished", env.display_name),
        ),
        NotificationSourceKind::WorkflowRun => {
            (None, EventRefs::default(), env.display_name.clone())
        }
        NotificationSourceKind::OwnerAction => {
            let (agent_id, refs) = owner_action_locator(&env.output);
            (agent_id, refs, env.display_name.clone())
        }
    };
    let event = OutsideEvent {
        cursor: 0,
        kernel: String::new(),
        machine: String::new(),
        session_id: source.session_id,
        agent_id,
        value: OutsideEventValue::Notification {
            source_id: env.source_id.clone(),
            source_kind: source.kind,
            display_name: env.display_name.clone(),
            occurrence_id: env.occurrence_id.clone(),
            output: env.output.clone(),
            deadline_ms: env.deadline_ms,
        },
        summary,
        at_ms: env
            .deadline_ms
            .saturating_sub(crate::local::NOTIFICATION_RETENTION_MS),
        refs,
    };
    crate::durable_state::agent_lifecycle::outside::append_notification(
        tx,
        &sub.endpoint_id,
        &receipt_id(sub, env)?,
        event,
    )?;
    Ok(NotificationOutcome::Queued)
}

/// The blocked task and decision an owner-action alert locates, so the holder can answer the
/// decision it names. The alert carries no task reason or approval value.
fn owner_action_locator(
    output: &crate::session::WorkflowOutputPayload,
) -> (Option<String>, EventRefs) {
    let located: serde_json::Value = serde_json::from_str(output.message()).unwrap_or_default();
    let field = |name: &str| located[name].as_str().map(str::to_owned);
    (
        field("agent_id"),
        EventRefs {
            task_id: field("task_id"),
            interaction_id: field("interaction_id"),
            ..Default::default()
        },
    )
}
