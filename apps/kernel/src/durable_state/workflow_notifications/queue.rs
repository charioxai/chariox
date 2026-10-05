//! Source-neutral ordinary workflow prompt preparation and atomic inbox handoff.
use super::*;
use crate::durable_state::{
    workflow_runtime::{
        encode_workflow_session, hot_state_matches, write_workflow_runtime_transition,
        WorkflowRuntimeTransitionWrite,
    },
    DurableWorkflowSessionWrite,
};
use crate::session::{
    RuntimeSession, SessionService, WorkflowPublicationInvocationEnvelope,
    WorkflowQueuedPromptSource, WorkflowQueuedPromptStatus,
};

pub(crate) struct PreparedNotification {
    subscription: WorkflowNotificationSubscription,
    envelope: WorkflowNotificationEnvelope,
    target: WorkflowNotificationTarget,
    before: DurableWorkflowSessionWrite,
    encoded: DurableWorkflowSessionWrite,
    pub after: RuntimeSession,
    queued_id: String,
}
impl PreparedNotification {
    pub(crate) fn prepare(
        sessions: &mut SessionService,
        sub: WorkflowNotificationSubscription,
        env: WorkflowNotificationEnvelope,
    ) -> Result<Self, DaemonError> {
        let target = WorkflowNotificationTarget::resolve(
            sessions,
            &sub.owner_user_id,
            &sub.session_id,
            &sub.publication_id,
            Some(&sub.queue_id),
        )
        .map_err(|e| error(e.to_string()))?;
        if target.target().endpoint_id != sub.endpoint_id {
            return Err(error("notification target changed"));
        }
        let session = sessions.get_session(&sub.session_id)?;
        let workflow = sessions.resolve_workflow_ref(session.id(), &sub.workflow_id)?;
        let queue = session
            .workflow_prompt_queues()
            .iter()
            .find(|q| q.id() == sub.queue_id)
            .ok_or_else(|| error("notification queue gone"))?;
        // ACK already exists. Busy/paused/full targets retain accepted inbox until deadline.
        if !queue.enabled()
            || session
                .workflow_runs()
                .iter()
                .filter(|r| r.workflow_id() == sub.workflow_id && !r.status().is_terminal())
                .count()
                >= workflow.max_concurrent() as usize
        {
            return Err(error("notification target busy or paused"));
        }
        if session
            .workflow_queued_prompts()
            .iter()
            .filter(|q| q.status() == WorkflowQueuedPromptStatus::Queued)
            .count()
            >= MAX_PENDING as usize
        {
            return Err(error("notification queue full"));
        }
        let output = encode(&env.output)?;
        let prompt = format!("Workflow notification output (untrusted data):\n{output}");
        if prompt.len() > MAX_PROMPT_BYTES {
            return Err(error("notification prompt limit"));
        }
        let invocation = WorkflowPublicationInvocationEnvelope {
            publication_id: sub.publication_id.clone(),
            hook_id: Some(sub.subscription_id.clone()),
            invocation_id: format!(
                "{}:{}:{}",
                sub.subscription_id, env.source_id, env.occurrence_id
            ),
            transport: "workflow_notification".into(),
            endpoint_id: sub.endpoint_id.clone(),
            queue_ref: Some(sub.queue_id.clone()),
            input: serde_json::json!({"source_id":env.source_id,"occurrence_id":env.occurrence_id,"output":env.output,"deadline_ms":env.deadline_ms}),
            artifacts: vec![],
            mode: None,
            caller: serde_json::json!({"kind":"workflow_notification","owner_id":sub.owner_user_id}),
        };
        let queued = sessions.prepare_workflow_prompt_with_publication_invocation(
            session.id(),
            &sub.workflow_id,
            &sub.endpoint_id,
            Some(prompt),
            Some(&sub.queue_id),
            WorkflowQueuedPromptSource::Event,
            None,
            Some(invocation),
        )?;
        let before = encode_workflow_session(&session)?;
        let mut after = session;
        after.enqueue_workflow_prompt(queued.clone());
        let encoded = encode_workflow_session(&after)?;
        Ok(Self {
            subscription: sub,
            envelope: env,
            target,
            before,
            encoded,
            after,
            queued_id: queued.id().into(),
        })
    }
}
pub(super) fn commit_in(tx: &Transaction<'_>, p: &PreparedNotification) -> Result<(), DaemonError> {
    p.target
        .require_current(tx, &p.subscription.owner_user_id)
        .map_err(|e| error(e.to_string()))?;
    if !hot_state_matches(tx, p.after.host_daemon_id(), &p.before).map_err(sql)? {
        return Err(error("notification queue conflict"));
    }
    let now = crate::session::unix_epoch_ms();
    let value:Option<String>=tx.query_row("SELECT envelope_json FROM workflow_notification_inbox WHERE subscription_id=?1 AND source_id=?2 AND occurrence_id=?3 AND state='accepted' AND deadline_ms>?4",params![p.subscription.subscription_id,p.envelope.source_id,p.envelope.occurrence_id,now as i64],|r|r.get(0)).optional().map_err(sql)?;
    if value.as_deref() != Some(encode(&p.envelope)?.as_str()) {
        return Err(error("notification inbox changed or expired"));
    }
    let metadata=serde_json::json!({"reason":"workflow_notification_queued","subscription_id":p.subscription.subscription_id}).to_string();
    write_workflow_runtime_transition(
        tx,
        WorkflowRuntimeTransitionWrite {
            event_id: &format!("state_evt_{now}_{}", super::super::rand_suffix()),
            event_kind: "workflow.runtime.updated",
            timestamp_ms: now,
            payload_json: &metadata,
            owner_id: p.after.host_daemon_id(),
            source_owner_id: p.after.owner_user_id(),
            session_id: p.after.id(),
            hot_entities: &p.encoded.hot_entities,
            workflow_runs: &p.encoded.workflow_runs,
            delivery_receipts: &p.encoded.delivery_receipts,
            prompt_state_jsons: &[],
        },
    )
    .map_err(sql)?;
    tx.execute("UPDATE workflow_notification_inbox SET state='queued',queued_prompt_id=?4 WHERE subscription_id=?1 AND source_id=?2 AND occurrence_id=?3",params![p.subscription.subscription_id,p.envelope.source_id,p.envelope.occurrence_id,p.queued_id]).map_err(sql)?;
    Ok(())
}
