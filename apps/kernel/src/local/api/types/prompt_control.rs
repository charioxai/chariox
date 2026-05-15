use super::*;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SubmitPromptRequest {
    pub session_id: String,
    pub attachment_id: String,
    pub target_agent_id: Option<String>,
    pub prompt: String,
    #[serde(default)]
    pub attachments: Vec<PromptAttachment>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CompletePromptRequest {
    pub session_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CancelActivePromptRequest {
    pub session_id: String,
    pub attachment_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UpdateSessionConfigRequest {
    pub session_id: String,
    pub attachment_id: String,
    pub values: BTreeMap<String, String>,
    pub requires_idle: bool,
}
