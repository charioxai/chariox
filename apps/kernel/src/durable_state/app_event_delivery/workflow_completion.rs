//! Source-neutral ordinary workflow prompt preparation and atomic inbox handoff.
use crate::durable_state::notification_target::WorkflowNotificationTarget;
use crate::durable_state::workflow_notifications::*;
use crate::durable_state::{
    workflow_runtime::{encode_workflow_session, hot_state_matches},
    DurableWorkflowSessionWrite,
};
use crate::error::DaemonError;
use crate::local::{WorkflowNotificationEnvelope, WorkflowNotificationSubscription};
use crate::session::{
    RuntimeSession, SessionService, WorkflowEventDeliveryReceipt,
    WorkflowPublicationInvocationEnvelope, WorkflowQueuedPromptSource, WorkflowQueuedPromptStatus,
};
use rusqlite::{params, OptionalExtension, Transaction};

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
            || (sub.delivery_mode == crate::local::NotificationDeliveryMode::Queue
                && session
                    .workflow_runs()
                    .iter()
                    .filter(|r| r.workflow_id() == sub.workflow_id && !r.status().is_terminal())
                    .count()
                    >= workflow.max_concurrent() as usize)
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
        let output = encode_notification_envelope(&env)?;
        let prompt = format!("{NOTIFICATION_PROMPT_HEADER}{output}");
        let invocation = WorkflowPublicationInvocationEnvelope {
            publication_id: sub.publication_id.clone(),
            hook_id: Some(sub.subscription_id.clone()),
            invocation_id: receipt_id(&sub, &env)?,
            transport: "workflow_notification".into(),
            endpoint_id: sub.endpoint_id.clone(),
            queue_ref: Some(sub.queue_id.clone()),
            input: serde_json::json!({"source_id":env.source_id,"occurrence_id":env.occurrence_id,"output":env.output,"status":env.status,"subject":env.subject,"payload":env.fields,"ancestry":env.ancestry,"deadline_ms":env.deadline_ms}),
            artifacts: vec![],
            mode: None,
            caller: serde_json::json!({"kind":"workflow_completion","owner_id":sub.owner_user_id,"installation_id":env.source_id,"delivery_mode":sub.delivery_mode}),
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
        after.record_workflow_event_delivery_receipt(WorkflowEventDeliveryReceipt {
            delivery_id: receipt_id(&sub, &env)?,
            binding_id: sub.subscription_id.clone(),
            occurrence_id: env.occurrence_id.clone(),
            queued_prompt_id: queued.id().into(),
            accepted_at_ms: crate::session::unix_epoch_ms(),
            expires_at_ms: env.deadline_ms,
        });
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
pub(crate) fn commit_in(tx: &Transaction<'_>, p: &PreparedNotification) -> Result<(), DaemonError> {
    p.target
        .require_current(tx, &p.subscription.owner_user_id)
        .map_err(|e| error(e.to_string()))?;
    if !hot_state_matches(tx, p.after.host_daemon_id(), &p.before).map_err(sql)? {
        return Err(error("notification queue conflict"));
    }
    let now = crate::session::unix_epoch_ms();
    let value:Option<String>=tx.query_row("SELECT payload_json FROM app_outbox WHERE source_kind='workflow_completion' AND automation_id=?1 AND installation_id=?2 AND occurrence_id=?3 AND state='accepted' AND expires_at_ms>?4",params![p.subscription.subscription_id,p.envelope.source_id,p.envelope.occurrence_id,now as i64],|r|r.get(0)).optional().map_err(sql)?;
    if value.as_deref() != Some(encode(&p.envelope)?.as_str()) {
        return Err(error("notification inbox changed or expired"));
    }
    super::write_queue_state_in(
        tx,
        &p.after,
        &p.encoded,
        "workflow_notification_queued",
        &p.subscription.subscription_id,
    )?;
    tx.execute("UPDATE app_outbox SET state='queued',queued_prompt_id=?4,queued_session_id=?5,invocation_json=?6 WHERE source_kind='workflow_completion' AND automation_id=?1 AND installation_id=?2 AND occurrence_id=?3",params![p.subscription.subscription_id,p.envelope.source_id,p.envelope.occurrence_id,p.queued_id,p.subscription.session_id,encode(&serde_json::json!({"ancestry":p.envelope.ancestry}))?]).map_err(sql)?;
    Ok(())
}
