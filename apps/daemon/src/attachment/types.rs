use serde::{Deserialize, Serialize};

#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ClientCapabilityLevel {
    FullTerminal,
    InteractiveStructured,
    MessageTransport,
    AutomationOnly,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AttachRequest {
    pub session_id: String,
    pub client_id: String,
    pub capability_level: ClientCapabilityLevel,
}

impl AttachRequest {
    pub fn new(
        session_id: impl Into<String>,
        client_id: impl Into<String>,
        capability_level: ClientCapabilityLevel,
    ) -> Self {
        Self {
            session_id: session_id.into(),
            client_id: client_id.into(),
            capability_level,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeAttachment {
    id: String,
    session_id: String,
    client_id: String,
    capability_level: ClientCapabilityLevel,
}

impl RuntimeAttachment {
    pub fn new(
        id: impl Into<String>,
        session_id: impl Into<String>,
        client_id: impl Into<String>,
        capability_level: ClientCapabilityLevel,
    ) -> Self {
        Self {
            id: id.into(),
            session_id: session_id.into(),
            client_id: client_id.into(),
            capability_level,
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
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum AttachmentEvent {
    Joined {
        session_id: String,
        attachment_id: String,
    },
    Left {
        session_id: String,
        attachment_id: String,
    },
}
