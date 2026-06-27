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
            session_id,
            attachment_id,
            target_agent_id: Some(self.target_agent_id),
            prompt: self.prompt,
            attachments: self.attachments,
        }
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
pub struct UpdateSessionConfigRequest {
    pub session_id: String,
    pub attachment_id: String,
    pub values: BTreeMap<String, String>,
    pub requires_idle: bool,
}
