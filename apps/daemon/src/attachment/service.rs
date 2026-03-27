use std::collections::BTreeMap;

use crate::error::DaemonError;
use crate::session::{PromptDetachEffect, SessionService};

use super::{AttachRequest, AttachmentEvent, RuntimeAttachment};

#[derive(Debug, Clone, Default)]
pub struct AttachmentService {
    attachments: BTreeMap<String, RuntimeAttachment>,
    events: Vec<AttachmentEvent>,
    next_attachment_number: u64,
}

impl AttachmentService {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn attach(
        &mut self,
        sessions: &mut SessionService,
        request: AttachRequest,
    ) -> Result<RuntimeAttachment, DaemonError> {
        let attachment_id = self.next_attachment_id();
        let attachment = RuntimeAttachment::new(
            attachment_id.clone(),
            request.session_id.clone(),
            request.client_id,
            request.capability_level,
        );

        sessions.add_attachment_to_session(&request.session_id, &attachment_id)?;
        self.attachments
            .insert(attachment_id.clone(), attachment.clone());
        self.events.push(AttachmentEvent::Joined {
            session_id: request.session_id,
            attachment_id,
        });

        Ok(attachment)
    }

    pub fn detach(
        &mut self,
        sessions: &mut SessionService,
        attachment_id: &str,
    ) -> Result<RuntimeAttachment, DaemonError> {
        self.detach_with_effect(sessions, attachment_id)
            .map(|(attachment, _)| attachment)
    }

    pub fn detach_with_effect(
        &mut self,
        sessions: &mut SessionService,
        attachment_id: &str,
    ) -> Result<(RuntimeAttachment, PromptDetachEffect), DaemonError> {
        let attachment = self.attachments.remove(attachment_id).ok_or_else(|| {
            DaemonError::AttachmentNotFound {
                attachment_id: attachment_id.to_string(),
            }
        })?;

        let (_, effect) =
            sessions.remove_attachment_from_session(attachment.session_id(), attachment_id)?;
        self.events.push(AttachmentEvent::Left {
            session_id: attachment.session_id().to_string(),
            attachment_id: attachment.id().to_string(),
        });

        Ok((attachment, effect))
    }

    pub fn get_attachment(&self, attachment_id: &str) -> Result<RuntimeAttachment, DaemonError> {
        self.attachments.get(attachment_id).cloned().ok_or_else(|| {
            DaemonError::AttachmentNotFound {
                attachment_id: attachment_id.to_string(),
            }
        })
    }

    pub fn list_events(&self) -> &[AttachmentEvent] {
        &self.events
    }

    pub fn list_session_attachment_ids(&self, session_id: &str) -> Vec<String> {
        self.attachments
            .values()
            .filter(|attachment| attachment.session_id() == session_id)
            .map(|attachment| attachment.id().to_string())
            .collect()
    }

    pub fn list_client_attachments(&self, client_id: &str) -> Vec<RuntimeAttachment> {
        self.attachments
            .values()
            .filter(|attachment| attachment.client_id() == client_id)
            .cloned()
            .collect()
    }

    pub fn remove_session_attachments(&mut self, session_id: &str) -> Vec<RuntimeAttachment> {
        let attachment_ids = self.list_session_attachment_ids(session_id);

        if attachment_ids.is_empty() {
            return Vec::new();
        }

        let removed_attachments: Vec<RuntimeAttachment> = attachment_ids
            .iter()
            .filter_map(|attachment_id| self.attachments.remove(attachment_id))
            .collect();

        for attachment in &removed_attachments {
            self.events.push(AttachmentEvent::Left {
                session_id: attachment.session_id().to_string(),
                attachment_id: attachment.id().to_string(),
            });
        }

        removed_attachments
    }

    fn next_attachment_id(&mut self) -> String {
        self.next_attachment_number += 1;
        format!("attachment-{}", self.next_attachment_number)
    }
}

#[cfg(test)]
mod tests {
    use crate::attachment::ClientCapabilityLevel;
    use crate::config::DaemonConfig;
    use crate::session::{CreateSessionRequest, SessionService};

    use super::{AttachRequest, AttachmentEvent, AttachmentService};

    fn session_service() -> SessionService {
        SessionService::new(&DaemonConfig::for_tests())
    }

    fn create_session(sessions: &mut SessionService) -> String {
        sessions
            .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
            .expect("session should be created")
            .id()
            .to_string()
    }

    #[test]
    fn supports_multiple_attachments_on_one_session() {
        let mut sessions = session_service();
        let session_id = create_session(&mut sessions);
        let mut attachments = AttachmentService::new();

        let first = attachments
            .attach(
                &mut sessions,
                AttachRequest::new(&session_id, "client-a", ClientCapabilityLevel::FullTerminal),
            )
            .expect("first attachment should attach");
        let second = attachments
            .attach(
                &mut sessions,
                AttachRequest::new(
                    &session_id,
                    "client-b",
                    ClientCapabilityLevel::InteractiveStructured,
                ),
            )
            .expect("second attachment should attach");

        let session = sessions
            .get_session(&session_id)
            .expect("session should still exist");

        assert_eq!(session.attachment_ids().len(), 2);
        assert!(session.has_attachment(first.id()));
        assert!(session.has_attachment(second.id()));
        assert_eq!(attachments.list_events().len(), 2);
    }

    #[test]
    fn detaching_attachment_removes_only_that_attachment() {
        let mut sessions = session_service();
        let session_id = create_session(&mut sessions);
        let mut attachments = AttachmentService::new();

        let first = attachments
            .attach(
                &mut sessions,
                AttachRequest::new(&session_id, "client-a", ClientCapabilityLevel::FullTerminal),
            )
            .expect("first attachment should attach");
        let second = attachments
            .attach(
                &mut sessions,
                AttachRequest::new(
                    &session_id,
                    "client-b",
                    ClientCapabilityLevel::InteractiveStructured,
                ),
            )
            .expect("second attachment should attach");

        let detached = attachments
            .detach(&mut sessions, first.id())
            .expect("attachment should detach");

        let session = sessions
            .get_session(&session_id)
            .expect("session should still exist");
        assert_eq!(detached.id(), first.id());
        assert!(!session.has_attachment(first.id()));
        assert!(session.has_attachment(second.id()));
        assert!(attachments.list_events().iter().any(|event| matches!(
            event,
            AttachmentEvent::Left { attachment_id, .. } if attachment_id == first.id()
        )));
    }
}
