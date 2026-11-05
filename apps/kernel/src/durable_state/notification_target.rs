//! Source-neutral workflow notification admission. App signature/capability
//! admission stays in its adapter; targets use one kernel ownership fence.
use crate::{
    error::DaemonError,
    session::{SessionService, WORKFLOW_PUBLICATION_KIND_EVENT_BASED},
};
use chariox_app_runtime::app_outbox::{AutomationTarget, OutboxError};
use rusqlite::{OptionalExtension, Transaction};
#[derive(Debug, thiserror::Error)]
pub(crate) enum NotificationTargetError {
    #[error(transparent)]
    Stopped(#[from] crate::runtime::app_operation_budget::AppOperationStopped),
    #[error(transparent)]
    Storage(#[from] DaemonError),
    #[error(transparent)]
    Outbox(#[from] OutboxError),
    #[error("app_automation_target_changed")]
    TargetChanged,
    #[error("app_automation_target_invalid")]
    InvalidTarget,
    #[error("app_automation_not_owner")]
    NotOwner,
    #[error("app_automation_unauthorized")]
    Unauthorized,
}
impl From<rusqlite::Error> for NotificationTargetError {
    fn from(error: rusqlite::Error) -> Self {
        Self::Outbox(OutboxError::Database(error))
    }
}
/// Built only by resolving existing workflow assets for the authenticated owner.
/// The runtime keeps its SessionService read guard and workflow transition mutex
/// until the writer replies; the writer also checks the exact durable entities.
#[derive(Debug)]
pub(crate) struct WorkflowNotificationTarget {
    owner: String,
    durable_owner: String,
    target: AutomationTarget,
    entities: Vec<(&'static str, String, String)>,
}
impl WorkflowNotificationTarget {
    pub(crate) fn resolve(
        sessions: &SessionService,
        trusted_owner: &str,
        session_id: &str,
        publication_ref: &str,
        queue_ref: Option<&str>,
    ) -> Result<Self, NotificationTargetError> {
        let session = sessions.get_session(session_id)?;
        let publication = sessions.resolve_workflow_publication_ref(session_id, publication_ref)?;
        if session.owner_user_id() != trusted_owner
            || !session.has_member(publication.created_by_user_id())
        {
            return Err(NotificationTargetError::NotOwner);
        }
        if !publication.enabled() || publication.kind() != WORKFLOW_PUBLICATION_KIND_EVENT_BASED {
            return Err(NotificationTargetError::InvalidTarget);
        }
        let workflow = sessions.resolve_workflow_ref(session_id, publication.workflow_id())?;
        let endpoint = sessions.resolve_workflow_endpoint_ref(
            session_id,
            workflow.id(),
            publication.endpoint_id(),
        )?;
        if endpoint.owner_user_id() != publication.created_by_user_id()
            || !session.has_member(endpoint.owner_user_id())
        {
            return Err(NotificationTargetError::NotOwner);
        }
        sessions.validate_workflow_runnable(session_id, &workflow, &endpoint)?;
        let queue_id = sessions.resolve_workflow_prompt_queue_ref(
            session_id,
            workflow.id(),
            queue_ref.or(publication.queue_ref()).unwrap_or("default"),
        )?;
        let queue = session
            .workflow_prompt_queues()
            .iter()
            .find(|queue| queue.id() == queue_id)
            .ok_or(NotificationTargetError::InvalidTarget)?;
        let entities = vec![
            (
                "publication",
                publication.id().to_owned(),
                encode(&publication.without_runtime_state())?,
            ),
            ("workflow", workflow.id().to_owned(), encode(&workflow)?),
            ("queue", queue.id().to_owned(), encode(queue)?),
        ];
        Ok(Self {
            owner: trusted_owner.into(),
            durable_owner: session.host_daemon_id().into(),
            target: AutomationTarget {
                kind: chariox_app_runtime::app_outbox::AutomationTargetKind::Workflow,
                session_id: session.id().into(),
                publication_id: publication.id().into(),
                endpoint_id: endpoint.id().into(),
                queue_id,
            },
            entities,
        })
    }
    pub(crate) fn resolve_agent(
        sessions: &SessionService,
        owner: &str,
        session_id: &str,
        agent_id: &str,
        trigger_id: &str,
    ) -> Result<Self, NotificationTargetError> {
        let session = sessions.get_session(session_id)?;
        if session.owner_user_id() != owner {
            return Err(NotificationTargetError::NotOwner);
        }
        if !session.agent_triggers().iter().any(|trigger| {
            trigger.id() == trigger_id
                && trigger.agent_id() == agent_id
                && matches!(
                    trigger.source(),
                    crate::session::AgentTriggerSource::Notification {
                        source: crate::session::AgentTriggerNotificationSource::App { .. },
                        ..
                    }
                )
        }) {
            return Err(NotificationTargetError::InvalidTarget);
        }
        Ok(Self {
            owner: owner.into(),
            durable_owner: session.host_daemon_id().into(),
            target: AutomationTarget {
                kind: chariox_app_runtime::app_outbox::AutomationTargetKind::Agent,
                session_id: session_id.into(),
                publication_id: trigger_id.into(),
                endpoint_id: agent_id.into(),
                queue_id: "agent".into(),
            },
            entities: Vec::new(),
        })
    }
    /// Authorization of an existing binding does not require its publication to
    /// remain runnable. Its exact publication ownership must still match on commit.
    pub(crate) fn authorize_existing(
        sessions: &SessionService,
        owner: &str,
        caller: &str,
        target: AutomationTarget,
    ) -> Result<Self, NotificationTargetError> {
        if target.kind != chariox_app_runtime::app_outbox::AutomationTargetKind::Workflow {
            return Err(NotificationTargetError::Unauthorized);
        }
        let session = sessions.get_session(&target.session_id)?;
        if session.owner_user_id() != owner
            || !session.has_member(caller)
            || session.status() == crate::session::SessionStatus::Ended
        {
            return Err(NotificationTargetError::Unauthorized);
        }
        let publication = sessions
            .resolve_workflow_publication_ref(&target.session_id, &target.publication_id)
            .map_err(|_| NotificationTargetError::Unauthorized)?;
        if publication.created_by_user_id() != caller {
            return Err(NotificationTargetError::Unauthorized);
        }
        Ok(Self {
            owner: owner.into(),
            durable_owner: session.host_daemon_id().into(),
            entities: vec![(
                "publication",
                publication.id().into(),
                encode(&publication.without_runtime_state())?,
            )],
            target,
        })
    }
    pub(crate) fn target(&self) -> &AutomationTarget {
        &self.target
    }
    pub(crate) fn require_current(
        &self,
        tx: &Transaction<'_>,
        owner: &str,
    ) -> Result<(), NotificationTargetError> {
        if self.owner != owner {
            return Err(NotificationTargetError::NotOwner);
        }
        for (kind, id, payload) in &self.entities {
            let current:Option<String>=tx.query_row("SELECT payload_json FROM durable_workflow_hot_entities WHERE owner_id=?1 AND session_id=?2 AND entity_kind=?3 AND entity_id=?4",
                rusqlite::params![self.durable_owner,self.target.session_id,kind,id],|row|row.get(0)).optional()?;
            // A deployed publication's runtime state changes in memory without
            // a durable write and is not part of what the automation targets.
            let current = match (*kind, current) {
                ("publication", Some(current)) => Some(encode(
                    &serde_json::from_str::<crate::session::WorkflowPublicationDefinition>(
                        &current,
                    )
                    .map_err(|_| NotificationTargetError::TargetChanged)?
                    .without_runtime_state(),
                )?),
                (_, current) => current,
            };
            if current.as_deref() != Some(payload) {
                return Err(NotificationTargetError::TargetChanged);
            }
        }
        Ok(())
    }
}
pub(crate) fn encode(value: &impl serde::Serialize) -> Result<String, NotificationTargetError> {
    let mut output = TargetEncoding {
        bytes: Vec::new(),
        limited: false,
    };
    if serde_json::to_writer(&mut output, value).is_err() {
        return Err(if output.limited {
            OutboxError::Limit.into()
        } else {
            NotificationTargetError::InvalidTarget
        });
    }
    String::from_utf8(output.bytes).map_err(|_| NotificationTargetError::InvalidTarget)
}
struct TargetEncoding {
    bytes: Vec<u8>,
    limited: bool,
}
impl std::io::Write for TargetEncoding {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if self
            .bytes
            .len()
            .checked_add(bytes.len())
            .is_none_or(|length| length > 1024 * 1024)
        {
            self.limited = true;
            return Err(std::io::Error::other(
                "App automation target exceeds encoded limit",
            ));
        }
        self.bytes.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
