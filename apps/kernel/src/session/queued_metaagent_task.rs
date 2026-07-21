use serde::{Deserialize, Serialize};

use super::{unix_epoch_ms, PromptAttachment};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QueuedMetaagentTask {
    id: String,
    metaagent_id: String,
    source_attachment_id: String,
    task_markdown: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    attachments: Vec<PromptAttachment>,
    created_at_ms: u64,
}

impl QueuedMetaagentTask {
    pub fn new(
        id: impl Into<String>,
        metaagent_id: impl Into<String>,
        source_attachment_id: impl Into<String>,
        task_markdown: impl Into<String>,
        attachments: Vec<PromptAttachment>,
    ) -> Self {
        Self {
            id: id.into(),
            metaagent_id: metaagent_id.into(),
            source_attachment_id: source_attachment_id.into(),
            task_markdown: task_markdown.into(),
            attachments,
            created_at_ms: unix_epoch_ms(),
        }
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn metaagent_id(&self) -> &str {
        &self.metaagent_id
    }

    pub fn source_attachment_id(&self) -> &str {
        &self.source_attachment_id
    }

    pub fn task_markdown(&self) -> &str {
        &self.task_markdown
    }

    pub fn attachments(&self) -> &[PromptAttachment] {
        &self.attachments
    }

    pub fn created_at_ms(&self) -> u64 {
        self.created_at_ms
    }
}
