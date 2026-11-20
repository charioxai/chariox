use super::*;
use crate::session::PromptStatus;

/// Confirmation of a prompt mutation. Content belongs to the Room feed and history.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PromptReceipt {
    id: String,
    target_agent_id: String,
    status: PromptStatus,
    created_at_ms: u64,
    updated_at_ms: u64,
}

impl PromptReceipt {
    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn target_agent_id(&self) -> &str {
        &self.target_agent_id
    }

    pub fn status(&self) -> PromptStatus {
        self.status
    }
}

impl From<&PromptQueueItem> for PromptReceipt {
    fn from(prompt: &PromptQueueItem) -> Self {
        Self {
            id: prompt.id().to_owned(),
            target_agent_id: prompt.target_agent_id().to_owned(),
            status: prompt.status(),
            created_at_ms: prompt.created_at_ms(),
            updated_at_ms: prompt.updated_at_ms(),
        }
    }
}

impl From<PromptSubmissionOutcome> for PromptSubmissionOutcome<PromptReceipt> {
    fn from(outcome: PromptSubmissionOutcome) -> Self {
        match outcome {
            PromptSubmissionOutcome::Started { prompt } => Self::Started {
                prompt: (&prompt).into(),
            },
            PromptSubmissionOutcome::Queued { prompt } => Self::Queued {
                prompt: (&prompt).into(),
            },
        }
    }
}

impl From<PromptCompletion> for PromptCompletion<PromptReceipt> {
    fn from(completion: PromptCompletion) -> Self {
        Self {
            completed: (&completion.completed).into(),
            started_next: completion.started_next.as_ref().map(PromptReceipt::from),
        }
    }
}

impl From<PromptCancellation> for PromptCancellation<PromptReceipt> {
    fn from(cancellation: PromptCancellation) -> Self {
        Self {
            prompt: (&cancellation.prompt).into(),
            started_next: cancellation.started_next.as_ref().map(PromptReceipt::from),
        }
    }
}

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
    pub outcome: PromptSubmissionOutcome<PromptReceipt>,
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
    /// Cancel only this active prompt (including its pending submission alias).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_prompt_id: Option<String>,
}

/// Cancel one task at its displayed revision, including its retained resources.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CancelAgentTaskRequest {
    pub session_id: String,
    pub attachment_id: String,
    pub task_id: String,
    pub revision: u64,
}

/// End one event a waiting task holds (a wake, or the follow of another agent) without cancelling its
/// owning task. `wake_id` is the `cancel_wake_id` of the wait event.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CancelAgentWakeRequest {
    pub session_id: String,
    pub attachment_id: String,
    pub wake_id: String,
}

/// End the wait a task is in, without waking its agent or cancelling anything else it owns.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CancelAgentWaitRequest {
    pub session_id: String,
    pub attachment_id: String,
    pub task_id: String,
    pub started_at_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SteerQueuedPromptRequest {
    pub session_id: String,
    pub attachment_id: String,
    pub target_agent_id: String,
    pub prompt_id: String,
}

/// Steer the current home turn or submit the next prompt. Native turn controls can require an exact turn.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SteerActivePromptRequest {
    pub session_id: String,
    pub attachment_id: String,
    pub target_agent_id: String,
    pub expected_active_prompt_id: String,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub require_exact_turn: bool,
    pub prompt: String,
    #[serde(default)]
    pub attachments: Vec<PromptAttachment>,
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
