use std::fmt;

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum ClientCapabilityLevel {
    FullTerminal,
    InteractiveStructured,
    MessageTransport,
    AutomationOnly,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum AttachmentMode {
    Observer,
    Controller,
}

impl fmt::Display for AttachmentMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let value = match self {
            Self::Observer => "observer",
            Self::Controller => "controller",
        };

        write!(f, "{value}")
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttachRequest {
    pub session_id: String,
    pub client_id: String,
    pub capability_level: ClientCapabilityLevel,
    pub mode: AttachmentMode,
}

impl AttachRequest {
    pub fn new(
        session_id: impl Into<String>,
        client_id: impl Into<String>,
        capability_level: ClientCapabilityLevel,
        mode: AttachmentMode,
    ) -> Self {
        Self {
            session_id: session_id.into(),
            client_id: client_id.into(),
            capability_level,
            mode,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeAttachment {
    id: String,
    session_id: String,
    client_id: String,
    capability_level: ClientCapabilityLevel,
    mode: AttachmentMode,
}

impl RuntimeAttachment {
    pub fn new(
        id: impl Into<String>,
        session_id: impl Into<String>,
        client_id: impl Into<String>,
        capability_level: ClientCapabilityLevel,
        mode: AttachmentMode,
    ) -> Self {
        Self {
            id: id.into(),
            session_id: session_id.into(),
            client_id: client_id.into(),
            capability_level,
            mode,
        }
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn session_id(&self) -> &str {
        &self.session_id
    }

    pub fn client_id(&self) -> &str {
        &self.client_id
    }

    pub fn capability_level(&self) -> ClientCapabilityLevel {
        self.capability_level
    }

    pub fn mode(&self) -> AttachmentMode {
        self.mode
    }

    pub fn set_mode(&mut self, mode: AttachmentMode) {
        self.mode = mode;
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AttachmentEvent {
    Joined {
        session_id: String,
        attachment_id: String,
        mode: AttachmentMode,
    },
    Left {
        session_id: String,
        attachment_id: String,
    },
    ControllerChanged {
        session_id: String,
        previous_attachment_id: Option<String>,
        current_attachment_id: Option<String>,
    },
}
