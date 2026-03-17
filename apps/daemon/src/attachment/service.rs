use std::collections::BTreeMap;

use crate::error::DaemonError;
use crate::session::SessionService;

use super::{AttachRequest, AttachmentEvent, AttachmentMode, RuntimeAttachment};

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
        let mut attachment = RuntimeAttachment::new(
            attachment_id.clone(),
            request.session_id.clone(),
            request.client_id,
            request.capability_level,
            AttachmentMode::Observer,
        );

        sessions.add_attachment_to_session(&request.session_id, &attachment_id)?;

        self.attachments
            .insert(attachment_id.clone(), attachment.clone());

        if request.mode == AttachmentMode::Controller {
            attachment = self.acquire_controller(sessions, &attachment_id)?;
        }

        self.events.push(AttachmentEvent::Joined {
            session_id: request.session_id,
            attachment_id: attachment_id.clone(),
            mode: attachment.mode(),
        });
        Ok(attachment)
    }

    pub fn detach(
        &mut self,
        sessions: &mut SessionService,
        attachment_id: &str,
    ) -> Result<RuntimeAttachment, DaemonError> {
        let attachment = self
            .attachments
            .get(attachment_id)
            .cloned()
            .ok_or_else(|| DaemonError::AttachmentNotFound {
                attachment_id: attachment_id.to_string(),
            })?;

        let (_, removed_was_controller) =
            sessions.remove_attachment_from_session(attachment.session_id(), attachment_id)?;

        if removed_was_controller {
            self.events.push(AttachmentEvent::ControllerChanged {
                session_id: attachment.session_id().to_string(),
                previous_attachment_id: Some(attachment.id().to_string()),
                current_attachment_id: None,
            });
        }

        self.attachments.remove(attachment_id);

        self.events.push(AttachmentEvent::Left {
            session_id: attachment.session_id().to_string(),
            attachment_id: attachment.id().to_string(),
        });

        Ok(attachment)
    }

    pub fn acquire_controller(
        &mut self,
        sessions: &mut SessionService,
        attachment_id: &str,
    ) -> Result<RuntimeAttachment, DaemonError> {
        let attachment = self
            .attachments
            .get(attachment_id)
            .cloned()
            .ok_or_else(|| DaemonError::AttachmentNotFound {
                attachment_id: attachment_id.to_string(),
            })?;

        let (session, previous_controller) =
            sessions.assign_controller(attachment.session_id(), attachment_id)?;

        if let Some(previous_attachment_id) = previous_controller.as_deref() {
            if previous_attachment_id != attachment_id {
                if let Some(previous_attachment) = self.attachments.get_mut(previous_attachment_id)
                {
                    previous_attachment.set_mode(AttachmentMode::Observer);
                }
            }
        }

        let updated_attachment = self
            .attachments
            .get_mut(attachment_id)
            .expect("attachment should exist while controller is acquired");
        updated_attachment.set_mode(AttachmentMode::Controller);

        self.events.push(AttachmentEvent::ControllerChanged {
            session_id: session.id().to_string(),
            previous_attachment_id: previous_controller,
            current_attachment_id: Some(attachment_id.to_string()),
        });

        Ok(updated_attachment.clone())
    }

    pub fn release_controller(
        &mut self,
        sessions: &mut SessionService,
        attachment_id: &str,
    ) -> Result<RuntimeAttachment, DaemonError> {
        let attachment = self.attachments.get_mut(attachment_id).ok_or_else(|| {
            DaemonError::AttachmentNotFound {
                attachment_id: attachment_id.to_string(),
            }
        })?;

        let (_, previous_controller) =
            sessions.release_controller(attachment.session_id(), attachment_id)?;

        attachment.set_mode(AttachmentMode::Observer);

        self.events.push(AttachmentEvent::ControllerChanged {
            session_id: attachment.session_id().to_string(),
            previous_attachment_id: previous_controller,
            current_attachment_id: None,
        });

        Ok(attachment.clone())
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

    pub fn remove_session_attachments(&mut self, session_id: &str) -> Vec<RuntimeAttachment> {
        let attachment_ids: Vec<String> = self
            .attachments
            .values()
            .filter(|attachment| attachment.session_id() == session_id)
            .map(|attachment| attachment.id().to_string())
            .collect();

        if attachment_ids.is_empty() {
            return Vec::new();
        }

        let removed_attachments: Vec<RuntimeAttachment> = attachment_ids
            .iter()
            .filter_map(|attachment_id| self.attachments.remove(attachment_id))
            .collect();

        if let Some(controller) = removed_attachments
            .iter()
            .find(|attachment| attachment.mode() == AttachmentMode::Controller)
        {
            self.events.push(AttachmentEvent::ControllerChanged {
                session_id: session_id.to_string(),
                previous_attachment_id: Some(controller.id().to_string()),
                current_attachment_id: None,
            });
        }

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
    use crate::DaemonError;

    use super::{AttachRequest, AttachmentEvent, AttachmentMode, AttachmentService};

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
    fn supports_multiple_observers_on_one_session() {
        let mut sessions = session_service();
        let session_id = create_session(&mut sessions);
        let mut attachments = AttachmentService::new();

        let first = attachments
            .attach(
                &mut sessions,
                AttachRequest::new(
                    &session_id,
                    "client-a",
                    ClientCapabilityLevel::FullTerminal,
                    AttachmentMode::Observer,
                ),
            )
            .expect("first observer should attach");
        let second = attachments
            .attach(
                &mut sessions,
                AttachRequest::new(
                    &session_id,
                    "client-b",
                    ClientCapabilityLevel::InteractiveStructured,
                    AttachmentMode::Observer,
                ),
            )
            .expect("second observer should attach");

        let session = sessions
            .get_session(&session_id)
            .expect("session should still exist");

        assert_eq!(first.mode(), AttachmentMode::Observer);
        assert_eq!(second.mode(), AttachmentMode::Observer);
        assert_eq!(session.attachment_ids().len(), 2);
        assert!(session.controller_attachment_id().is_none());
        assert_eq!(attachments.list_events().len(), 2);
    }

    #[test]
    fn reassigns_controller_when_another_attachment_takes_control() {
        let mut sessions = session_service();
        let session_id = create_session(&mut sessions);
        let mut attachments = AttachmentService::new();

        let first = attachments
            .attach(
                &mut sessions,
                AttachRequest::new(
                    &session_id,
                    "client-a",
                    ClientCapabilityLevel::FullTerminal,
                    AttachmentMode::Observer,
                ),
            )
            .expect("first attachment should attach");
        let second = attachments
            .attach(
                &mut sessions,
                AttachRequest::new(
                    &session_id,
                    "client-b",
                    ClientCapabilityLevel::FullTerminal,
                    AttachmentMode::Observer,
                ),
            )
            .expect("second attachment should attach");

        attachments
            .acquire_controller(&mut sessions, first.id())
            .expect("first attachment should become controller");
        let second = attachments
            .acquire_controller(&mut sessions, second.id())
            .expect("second attachment should take control");

        let session = sessions
            .get_session(&session_id)
            .expect("session should still exist");
        let first = attachments
            .get_attachment(first.id())
            .expect("first attachment should still exist");

        assert_eq!(second.mode(), AttachmentMode::Controller);
        assert_eq!(first.mode(), AttachmentMode::Observer);
        assert_eq!(session.controller_attachment_id(), Some(second.id()));
        assert!(attachments.list_events().iter().any(|event| matches!(
            event,
            AttachmentEvent::ControllerChanged {
                previous_attachment_id: Some(previous),
                current_attachment_id: Some(current),
                ..
            } if previous == first.id() && current == second.id()
        )));
    }

    #[test]
    fn detaching_current_controller_clears_controller_lease() {
        let mut sessions = session_service();
        let session_id = create_session(&mut sessions);
        let mut attachments = AttachmentService::new();

        let controller = attachments
            .attach(
                &mut sessions,
                AttachRequest::new(
                    &session_id,
                    "client-a",
                    ClientCapabilityLevel::FullTerminal,
                    AttachmentMode::Observer,
                ),
            )
            .expect("controller candidate should attach");
        let observer = attachments
            .attach(
                &mut sessions,
                AttachRequest::new(
                    &session_id,
                    "client-b",
                    ClientCapabilityLevel::MessageTransport,
                    AttachmentMode::Observer,
                ),
            )
            .expect("observer should attach");

        attachments
            .acquire_controller(&mut sessions, controller.id())
            .expect("controller should be acquired");
        attachments
            .detach(&mut sessions, controller.id())
            .expect("controller should detach cleanly");

        let session = sessions
            .get_session(&session_id)
            .expect("session should still exist");
        let observer = attachments
            .get_attachment(observer.id())
            .expect("observer should remain attached");

        assert_eq!(session.attachment_ids().len(), 1);
        assert_eq!(session.controller_attachment_id(), None);
        assert_eq!(observer.mode(), AttachmentMode::Observer);
        assert!(attachments.list_events().iter().any(|event| matches!(
            event,
            AttachmentEvent::ControllerChanged {
                previous_attachment_id: Some(previous),
                current_attachment_id: None,
                ..
            } if previous == controller.id()
        )));
    }

    #[test]
    fn detaching_observer_keeps_existing_controller_lease() {
        let mut sessions = session_service();
        let session_id = create_session(&mut sessions);
        let mut attachments = AttachmentService::new();

        let controller = attachments
            .attach(
                &mut sessions,
                AttachRequest::new(
                    &session_id,
                    "client-a",
                    ClientCapabilityLevel::FullTerminal,
                    AttachmentMode::Observer,
                ),
            )
            .expect("controller candidate should attach");
        let observer = attachments
            .attach(
                &mut sessions,
                AttachRequest::new(
                    &session_id,
                    "client-b",
                    ClientCapabilityLevel::MessageTransport,
                    AttachmentMode::Observer,
                ),
            )
            .expect("observer should attach");

        attachments
            .acquire_controller(&mut sessions, controller.id())
            .expect("controller should be acquired");
        attachments
            .detach(&mut sessions, observer.id())
            .expect("observer should detach cleanly");

        let session = sessions
            .get_session(&session_id)
            .expect("session should still exist");

        assert_eq!(session.attachment_ids().len(), 1);
        assert_eq!(session.controller_attachment_id(), Some(controller.id()));
        assert!(!attachments.list_events().iter().any(|event| matches!(
            event,
            AttachmentEvent::ControllerChanged {
                previous_attachment_id: Some(previous),
                current_attachment_id: None,
                ..
            } if previous == controller.id()
        )));
    }

    #[test]
    fn rejects_controller_release_from_non_controller_attachment() {
        let mut sessions = session_service();
        let session_id = create_session(&mut sessions);
        let mut attachments = AttachmentService::new();

        let controller = attachments
            .attach(
                &mut sessions,
                AttachRequest::new(
                    &session_id,
                    "client-a",
                    ClientCapabilityLevel::FullTerminal,
                    AttachmentMode::Controller,
                ),
            )
            .expect("controller should attach");
        let observer = attachments
            .attach(
                &mut sessions,
                AttachRequest::new(
                    &session_id,
                    "client-b",
                    ClientCapabilityLevel::InteractiveStructured,
                    AttachmentMode::Observer,
                ),
            )
            .expect("observer should attach");

        let error = attachments
            .release_controller(&mut sessions, observer.id())
            .expect_err("observer must not be able to release someone else's lease");

        match error {
            DaemonError::AttachmentIsNotController {
                session_id: errored_session_id,
                attachment_id,
            } => {
                assert_eq!(errored_session_id, session_id);
                assert_eq!(attachment_id, observer.id());
            }
            other => panic!("unexpected error: {other}"),
        }

        let session = sessions
            .get_session(&session_id)
            .expect("session should still exist");
        let persisted_controller = attachments
            .get_attachment(controller.id())
            .expect("controller should still exist");

        assert_eq!(session.controller_attachment_id(), Some(controller.id()));
        assert_eq!(persisted_controller.mode(), AttachmentMode::Controller);
    }
}
