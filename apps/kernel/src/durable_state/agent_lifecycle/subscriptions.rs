//! MP-08/MP-10/MP-11 F04: bind a completion subscription to submitted work.
use super::*;

pub(super) fn bind(
    tx: &Transaction<'_>,
    task: &AgentTaskExecution,
    registration: &mut Registration,
) -> Result<(), DaemonError> {
    let peers = tasks(tx)?;
    let source_for = |obligation: &AgentObligation| -> Option<String> {
        let source = obligation.completion_source()?;
        if source == registration.source_id
            || obligation.resource_id.as_deref() == Some(&registration.source_id)
        {
            return Some(source.into());
        }
        // A normal prompt to an existing child has a delivery obligation,
        // whose resource is the accepted prompt. Wait for that task, never
        // a prior task-terminal occurrence published under the same agent ID.
        (obligation.kind == "message" && obligation.dispatch_state == "accepted")
            .then(|| {
                peers
                    .iter()
                    .find(|peer| {
                        peer.room_id == task.room_id
                            && peer.agent_id == registration.source_id
                            && (peer.task_id == source || peer.prompt_id == source)
                    })
                    .map(|peer| peer.task_id.clone())
            })
            .flatten()
    };
    let candidate = if let Some(id) = registration.obligation_id.as_deref() {
        Some(
            task.obligations
                .iter()
                .find(|o| o.id == id)
                .ok_or_else(|| error("source does not cover this task's obligation"))?,
        )
    } else {
        task.obligations
            .iter()
            .rev()
            // A spawned child's completion and delivered prompt can match
            // the same source. Cover the unresolved completion first.
            .find(|o| o.requires_resolution() && source_for(o).is_some())
            .or_else(|| {
                task.obligations
                    .iter()
                    .rev()
                    .find(|o| source_for(o).is_some())
            })
    };
    if let Some(obligation) = candidate {
        let source = source_for(obligation).ok_or_else(|| {
            error("source does not cover this obligation; use its exact task_id from subscriptions")
        })?;
        if obligation.requires_resolution() {
            registration.obligation_id = Some(obligation.id.clone());
        } else if obligation.kind == "message"
            && obligation.status == "satisfied"
            && obligation.dispatch_state == "accepted"
        {
            // Delivery is already complete. Observing the task's completion
            // creates no reply obligation and does not reopen message delivery.
            registration.obligation_id = None;
        } else {
            return Err(error(
                "obligation already resolved; inspect its receipt and finish",
            ));
        }
        registration.source_id = source;
    }
    Ok(())
}
