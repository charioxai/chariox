use super::*;
use crate::durable_state::{
    app_automations::WorkflowAutomationTarget, workflow_runtime::encode_workflow_session,
};
use crate::session::{
    RuntimeSession, SessionService, WorkflowEventDeliveryReceipt,
    WorkflowPublicationInvocationEnvelope, WorkflowQueuedPromptSource, WorkflowQueuedPromptStatus,
};
use chariox_app_runtime::app_outbox::{AutomationStatus, Invocation, Occurrence};

pub(crate) const APP_EVENT_TRANSPORT: &str = "app_event";
const MAX_QUEUED_WORKFLOW_PROMPTS: usize = 1024;

/// Retains an exact candidate and a tentative clone. It is not a published
/// SessionService mutation and cannot be manufactured from terminal parameters.
pub(crate) struct PreparedAppEvent {
    pub(super) candidate: AppEventCandidate,
    pub(super) target: WorkflowAutomationTarget,
    pub(super) before: DurableWorkflowSessionWrite,
    pub(super) encoded: DurableWorkflowSessionWrite,
    pub(super) after: RuntimeSession,
    pub(super) queued_id: String,
    #[cfg(test)]
    pub(super) fault_after_commit: Option<Arc<TestCommitFault>>,
}
impl PreparedAppEvent {
    /// Holds the caller's SessionService write guard through the subsequent
    /// writer acknowledgement. The ID allocator advances; the queue does not.
    pub(crate) fn prepare(
        sessions: &mut SessionService,
        candidate: AppEventCandidate,
    ) -> Result<Self> {
        let configuration = &candidate.configuration;
        let receipt = &candidate.receipt;
        if configuration.status == AutomationStatus::Paused {
            return Err(OutboxError::Inactive.into());
        }
        if configuration.status != AutomationStatus::Active
            || receipt.automation_revision != configuration.revision
        {
            return Err(AppEventDeliveryError::AutomationChanged);
        }
        if !matches!(
            receipt.state,
            ReceiptState::Accepted | ReceiptState::Retryable
        ) {
            return Err(AppEventDeliveryError::Conflict);
        }
        let target = WorkflowAutomationTarget::resolve(
            sessions,
            &candidate.owner,
            &configuration.target.session_id,
            &configuration.target.publication_id,
            Some(&configuration.target.queue_id),
        )?;
        if target.target() != &configuration.target {
            return Err(AppEventDeliveryError::Conflict);
        }
        let session = sessions.get_session(&configuration.target.session_id)?;
        if session
            .workflow_queued_prompts()
            .iter()
            .filter(|queued| queued.status() == WorkflowQueuedPromptStatus::Queued)
            .count()
            >= MAX_QUEUED_WORKFLOW_PROMPTS
        {
            return Err(AppEventDeliveryError::Limit);
        }
        let invocation: Invocation = serde_json::from_str(
            receipt
                .invocation
                .as_deref()
                .ok_or(AppEventDeliveryError::Conflict)?,
        )
        .map_err(|_| OutboxError::Corrupt)?;
        let payload = serde_json::from_str(
            receipt
                .payload
                .as_deref()
                .ok_or(AppEventDeliveryError::Conflict)?,
        )
        .map_err(|_| OutboxError::Corrupt)?;
        AppOutbox::validate_occurrences(&[Occurrence {
            automation_id: receipt.automation_id.clone(),
            occurrence_id: receipt.occurrence_id.clone(),
            event_version: receipt.event_version,
            occurred_at_ms: receipt.occurred_at_ms,
            schedule_revision: receipt.schedule_revision.clone(),
            invocation: invocation.clone(),
            payload,
        }])?;
        let publication = sessions
            .resolve_workflow_publication_ref(session.id(), &configuration.target.publication_id)?;
        let envelope = envelope(&candidate, &invocation)?;
        let queued = sessions.prepare_workflow_prompt_with_publication_invocation(
            session.id(),
            publication.workflow_id(),
            &configuration.target.endpoint_id,
            Some(invocation.prompt),
            Some(&configuration.target.queue_id),
            WorkflowQueuedPromptSource::Event,
            None,
            Some(envelope),
        )?;
        let before = encode_workflow_session(&session)?;
        let mut after = session;
        after.enqueue_workflow_prompt(queued.clone());
        after.record_workflow_event_delivery_receipt(WorkflowEventDeliveryReceipt {
            delivery_id: receipt.receipt_id.clone(),
            binding_id: receipt.automation_id.clone(),
            occurrence_id: receipt.occurrence_id.clone(),
            queued_prompt_id: queued.id().into(),
            accepted_at_ms: receipt.accepted_at_ms,
            expires_at_ms: receipt.expires_at_ms,
        });
        let encoded = encode_workflow_session(&after)?;
        Ok(Self {
            candidate,
            target,
            before,
            encoded,
            after,
            queued_id: queued.id().into(),
            #[cfg(test)]
            fault_after_commit: None,
        })
    }
    pub(crate) fn session(&self) -> &RuntimeSession {
        &self.after
    }
}
fn envelope(
    candidate: &AppEventCandidate,
    invocation: &Invocation,
) -> Result<WorkflowPublicationInvocationEnvelope> {
    let artifacts = invocation
        .artifacts
        .iter()
        .map(|artifact| {
            // Strictly metadata. No URL fetch, host file open, artifact-store lookup,
            // PromptAttachment conversion, or provider attachment registration.
            serde_json::to_value(chariox_event_protocol::EventArtifact {
                name: artifact.name.clone(),
                media_type: artifact.media_type.clone(),
                reference: artifact.reference.clone(),
                size_bytes: artifact.size_bytes,
                digest: artifact.digest.clone(),
            })
            .map_err(|_| OutboxError::Corrupt.into())
        })
        .collect::<Result<Vec<_>>>()?;
    let receipt = &candidate.receipt;
    Ok(WorkflowPublicationInvocationEnvelope {
        publication_id: candidate.configuration.target.publication_id.clone(),
        hook_id: Some(receipt.automation_id.clone()),
        invocation_id: receipt.receipt_id.clone(),
        transport: APP_EVENT_TRANSPORT.into(),
        endpoint_id: candidate.configuration.target.endpoint_id.clone(),
        queue_ref: Some(candidate.configuration.target.queue_id.clone()),
        input: serde_json::json!({"event_type":candidate.configuration.event_name,"event_type_version":receipt.event_version,
            "occurrence_id":receipt.occurrence_id,"occurred_at_ms":receipt.occurred_at_ms,"schedule_revision":receipt.schedule_revision,
            "payload":serde_json::from_str::<serde_json::Value>(receipt.payload.as_deref().ok_or(AppEventDeliveryError::Conflict)?).map_err(|_|OutboxError::Corrupt)?}),
        artifacts,
        mode: None,
        caller: serde_json::json!({"kind":"app_event","installation_id":candidate.catalog.installation_id(),
            "owner_id":candidate.owner,"automation_id":receipt.automation_id,"automation_revision":receipt.automation_revision}),
    })
}

#[cfg(test)]
pub(super) struct TestCommitFault {
    pub(super) entered: std::sync::mpsc::SyncSender<()>,
    pub(super) release: std::sync::Mutex<std::sync::mpsc::Receiver<()>>,
}
