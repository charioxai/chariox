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
pub struct SubmitPromptsRequest {
    pub session_id: String,
    pub attachment_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_concurrency: Option<usize>,
    pub prompts: Vec<SubmitPromptsRequestItem>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SubmitPromptsRequestItem {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attachment_id: Option<String>,
    pub target_agent_id: String,
    pub prompt: String,
    #[serde(default)]
    pub attachments: Vec<PromptAttachment>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PromptBatchSubmissionResult {
    pub index: usize,
    pub agent_id: String,
    pub outcome: PromptSubmissionOutcome,
}

impl SubmitPromptsRequestItem {
    pub fn into_submit_prompt_request(
        self,
        session_id: String,
        attachment_id: String,
    ) -> SubmitPromptRequest {
        SubmitPromptRequest {
            session_id: self.session_id.unwrap_or(session_id),
            attachment_id: self.attachment_id.unwrap_or(attachment_id),
            target_agent_id: Some(self.target_agent_id),
            prompt: self.prompt,
            attachments: self.attachments,
        }
    }

    pub fn effective_session_id<'a>(&'a self, default_session_id: &'a str) -> &'a str {
        self.session_id.as_deref().unwrap_or(default_session_id)
    }

    pub fn effective_attachment_id<'a>(&'a self, default_attachment_id: &'a str) -> &'a str {
        self.attachment_id
            .as_deref()
            .unwrap_or(default_attachment_id)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CompletePromptRequest {
    pub session_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CancelActivePromptRequest {
    pub session_id: String,
    pub attachment_id: String,
    pub target_agent_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SteerQueuedPromptRequest {
    pub session_id: String,
    pub attachment_id: String,
    pub target_agent_id: String,
    pub prompt_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CancelQueuedPromptRequest {
    pub session_id: String,
    pub attachment_id: String,
    pub target_agent_id: String,
    pub prompt_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UpdateQueuedPromptRequest {
    pub session_id: String,
    pub attachment_id: String,
    pub target_agent_id: String,
    pub prompt_id: String,
    pub prompt: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UpdateSessionConfigRequest {
    pub session_id: String,
    pub attachment_id: String,
    pub values: BTreeMap<String, String>,
    pub requires_idle: bool,
}
