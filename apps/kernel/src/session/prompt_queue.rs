use std::collections::VecDeque;

use serde::{Deserialize, Serialize};

#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum PromptStatus {
    Queued,
    Running,
    Cancelling,
    Completed,
    Cancelled,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PromptOrigin {
    #[default]
    Arroba,
    External,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PromptAttachment {
    url: String,
    mime: String,
    filename: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    contents_base64: Option<String>,
}

impl PromptAttachment {
    pub fn new(url: impl Into<String>, mime: impl Into<String>, filename: Option<String>) -> Self {
        Self {
            url: url.into(),
            mime: mime.into(),
            filename,
            contents_base64: None,
        }
    }

    pub fn with_contents_base64(mut self, contents_base64: impl Into<String>) -> Self {
        self.contents_base64 = Some(contents_base64.into());
        self
    }

    pub fn url(&self) -> &str {
        &self.url
    }

    pub fn mime(&self) -> &str {
        &self.mime
    }

    pub fn filename(&self) -> Option<&str> {
        self.filename.as_deref()
    }

    pub fn contents_base64(&self) -> Option<&str> {
        self.contents_base64.as_deref()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PromptQueueItem {
    id: String,
    source_attachment_id: String,
    target_agent_id: String,
    prompt: String,
    attachments: Vec<PromptAttachment>,
    #[serde(default, skip_serializing, skip_deserializing)]
    hidden_system_context: String,
    status: PromptStatus,
    #[serde(default)]
    prompt_origin: PromptOrigin,
    workflow_run_id: Option<String>,
    workflow_node_run_id: Option<String>,
}

impl PromptQueueItem {
    pub fn new(
        id: impl Into<String>,
        source_attachment_id: impl Into<String>,
        target_agent_id: impl Into<String>,
        prompt: impl Into<String>,
        status: PromptStatus,
    ) -> Self {
        Self {
            id: id.into(),
            source_attachment_id: source_attachment_id.into(),
            target_agent_id: target_agent_id.into(),
            prompt: prompt.into(),
            attachments: Vec::new(),
            hidden_system_context: String::new(),
            status,
            prompt_origin: PromptOrigin::Arroba,
            workflow_run_id: None,
            workflow_node_run_id: None,
        }
    }

    pub fn with_attachments(mut self, attachments: Vec<PromptAttachment>) -> Self {
        self.attachments = attachments;
        self
    }

    pub fn with_hidden_system_context(mut self, hidden_system_context: impl Into<String>) -> Self {
        self.hidden_system_context = hidden_system_context.into();
        self
    }

    pub fn with_workflow_context(
        mut self,
        workflow_run_id: impl Into<String>,
        workflow_node_run_id: impl Into<String>,
    ) -> Self {
        self.workflow_run_id = Some(workflow_run_id.into());
        self.workflow_node_run_id = Some(workflow_node_run_id.into());
        self
    }

    pub fn with_prompt_origin(mut self, prompt_origin: PromptOrigin) -> Self {
        self.prompt_origin = prompt_origin;
        self
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn source_attachment_id(&self) -> &str {
        &self.source_attachment_id
    }

    pub fn target_agent_id(&self) -> &str {
        &self.target_agent_id
    }

    pub fn prompt(&self) -> &str {
        &self.prompt
    }

    pub fn attachments(&self) -> &[PromptAttachment] {
        &self.attachments
    }

    pub fn hidden_system_context(&self) -> &str {
        &self.hidden_system_context
    }

    pub fn status(&self) -> PromptStatus {
        self.status
    }

    pub fn prompt_origin(&self) -> PromptOrigin {
        self.prompt_origin
    }

    pub fn workflow_run_id(&self) -> Option<&str> {
        self.workflow_run_id.as_deref()
    }

    pub fn workflow_node_run_id(&self) -> Option<&str> {
        self.workflow_node_run_id.as_deref()
    }

    pub fn set_status(&mut self, status: PromptStatus) {
        self.status = status;
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum PromptSubmissionOutcome {
    Started { prompt: PromptQueueItem },
    Queued { prompt: PromptQueueItem },
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct AgentPromptState {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(in crate::session) active_prompt: Option<PromptQueueItem>,
    #[serde(default, skip_serializing_if = "VecDeque::is_empty")]
    pub(in crate::session) queued_prompts: VecDeque<PromptQueueItem>,
}

impl AgentPromptState {
    pub(in crate::session) fn from_parts(
        active_prompt: Option<PromptQueueItem>,
        queued_prompts: VecDeque<PromptQueueItem>,
    ) -> Self {
        Self {
            active_prompt,
            queued_prompts,
        }
    }

    pub fn active_prompt(&self) -> Option<&PromptQueueItem> {
        self.active_prompt.as_ref()
    }

    pub fn queued_prompts(&self) -> &VecDeque<PromptQueueItem> {
        &self.queued_prompts
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PromptCompletion {
    pub completed: PromptQueueItem,
    pub started_next: Option<PromptQueueItem>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PromptCancellation {
    pub prompt: PromptQueueItem,
    pub started_next: Option<PromptQueueItem>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct PromptDetachEffect {
    pub removed_active_prompt: bool,
    pub removed_queued_prompt_count: usize,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prompt_queue_item_does_not_serialize_hidden_system_context() {
        let item = PromptQueueItem::new(
            "prompt-1",
            "attachment-1",
            "agent-1",
            "VISIBLE_PROMPT_TOKEN",
            PromptStatus::Queued,
        )
        .with_hidden_system_context("HIDDEN_CONTEXT_TOKEN");

        let payload = serde_json::to_string(&item).expect("prompt should serialize");

        assert!(payload.contains("VISIBLE_PROMPT_TOKEN"));
        assert!(!payload.contains("HIDDEN_CONTEXT_TOKEN"));
        assert!(!payload.contains("hidden_system_context"));
    }

    #[test]
    fn prompt_queue_item_deserialization_drops_hidden_system_context() {
        let payload = r#"{
            "id":"prompt-1",
            "source_attachment_id":"attachment-1",
            "target_agent_id":"agent-1",
            "prompt":"VISIBLE_PROMPT_TOKEN",
            "attachments":[],
            "hidden_system_context":"HIDDEN_CONTEXT_TOKEN",
            "status":"Queued",
            "workflow_run_id":null,
            "workflow_node_run_id":null
        }"#;

        let item: PromptQueueItem =
            serde_json::from_str(payload).expect("prompt should deserialize");

        assert_eq!(item.prompt(), "VISIBLE_PROMPT_TOKEN");
        assert_eq!(item.hidden_system_context(), "");
    }
}
