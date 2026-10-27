use super::*;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExternalProviderSessionCapabilities {
    #[serde(default)]
    pub can_read_history: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExternalProviderSessionRecord {
    #[serde(skip)]
    pub(crate) owner_user_id: String,
    pub external_session_id: String,
    pub provider: String,
    pub provider_session_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title_source: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub first_prompt_preview: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_at_ms: Option<u64>,
    pub last_modified_at_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub worktree_path: Option<String>,
    #[serde(default = "default_external_provider_account_profile")]
    pub account_profile: String,
    #[serde(default)]
    pub capabilities: ExternalProviderSessionCapabilities,
    #[serde(default, skip)]
    pub attached_to_chariox: bool,
    #[serde(default, skip)]
    pub attached_session_ids: Vec<String>,
    #[serde(default, skip)]
    pub attached_agent_ids: Vec<String>,
}

fn default_external_provider_account_profile() -> String {
    "default".to_string()
}

impl ExternalProviderSessionRecord {
    pub fn is_attached_to_chariox(&self) -> bool {
        self.attached_to_chariox
    }

    pub fn is_attachable_to_chariox(&self) -> bool {
        !self.is_attached_to_chariox()
    }

    pub fn mark_attached_to_chariox(&mut self, session_ids: Vec<String>, agent_ids: Vec<String>) {
        self.attached_to_chariox = true;
        self.attached_session_ids = session_ids;
        self.attached_agent_ids = agent_ids;
    }

    pub fn clear_chariox_attachment(&mut self) {
        self.attached_to_chariox = false;
        self.attached_session_ids.clear();
        self.attached_agent_ids.clear();
    }

    pub fn first_attached_session_id(&self) -> Option<&str> {
        self.attached_session_ids.first().map(String::as_str)
    }

    pub fn first_attached_agent_id(&self) -> Option<&str> {
        self.attached_agent_ids.first().map(String::as_str)
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExternalProviderSessionPage {
    pub sessions: Vec<ExternalProviderSessionRecord>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
    #[serde(default)]
    pub has_more: bool,
    pub generated_at_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ListExternalProviderSessionsRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cursor: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RefreshExternalProviderSessionsRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImportExternalProviderSessionRequest {
    pub external_session_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub alias: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effort: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub worktree_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImportExternalProviderAgentRequest {
    pub session_id: String,
    pub external_session_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub alias: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effort: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub focus: Option<bool>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn external_provider_session_record_tracks_chariox_attachment_state() {
        let mut record = ExternalProviderSessionRecord {
            owner_user_id: crate::session::DEFAULT_LOCAL_USER_ID.to_string(),
            external_session_id: "codex:thread-1".to_string(),
            provider: "codex".to_string(),
            provider_session_id: "thread-1".to_string(),
            title: None,
            title_source: None,
            first_prompt_preview: None,
            created_at_ms: None,
            last_modified_at_ms: 42,
            worktree_path: None,
            account_profile: "default".to_string(),
            capabilities: ExternalProviderSessionCapabilities::default(),
            attached_to_chariox: false,
            attached_session_ids: Vec::new(),
            attached_agent_ids: Vec::new(),
        };

        assert!(record.is_attachable_to_chariox());
        assert!(!record.is_attached_to_chariox());

        record.mark_attached_to_chariox(vec!["session-1".to_string()], vec!["agent-1".to_string()]);

        assert!(record.is_attached_to_chariox());
        assert!(!record.is_attachable_to_chariox());
        assert_eq!(record.first_attached_session_id(), Some("session-1"));
        assert_eq!(record.first_attached_agent_id(), Some("agent-1"));

        record.clear_chariox_attachment();

        assert!(record.is_attachable_to_chariox());
        assert_eq!(record.first_attached_session_id(), None);
        assert_eq!(record.first_attached_agent_id(), None);
    }
}
