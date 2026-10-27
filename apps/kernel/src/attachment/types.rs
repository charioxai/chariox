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
    pub owner_user_id: String,
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
            owner_user_id: default_attachment_owner_user_id(),
        }
    }

    pub fn for_user(
        session_id: impl Into<String>,
        client_id: impl Into<String>,
        capability_level: ClientCapabilityLevel,
        owner_user_id: impl Into<String>,
    ) -> Self {
        Self {
            session_id: session_id.into(),
            client_id: client_id.into(),
            capability_level,
            owner_user_id: owner_user_id.into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeAttachment {
    id: String,
    session_id: String,
    client_id: String,
    capability_level: ClientCapabilityLevel,
    #[serde(default = "default_attachment_owner_user_id", skip_serializing)]
    owner_user_id: String,
}

impl RuntimeAttachment {
    pub fn new(
        id: impl Into<String>,
        session_id: impl Into<String>,
        client_id: impl Into<String>,
        capability_level: ClientCapabilityLevel,
        owner_user_id: impl Into<String>,
    ) -> Self {
        Self {
            id: id.into(),
            session_id: session_id.into(),
            client_id: client_id.into(),
            capability_level,
            owner_user_id: owner_user_id.into(),
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

    pub fn owner_user_id(&self) -> &str {
        &self.owner_user_id
    }
}

fn default_attachment_owner_user_id() -> String {
    crate::session::DEFAULT_LOCAL_USER_ID.to_string()
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
