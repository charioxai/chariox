use serde::{Deserialize, Serialize};

use super::types::unix_epoch_ms;

#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeInteractionKind {
    Choice,
    Permission,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeInteractionLevel {
    Info,
    Warning,
    Critical,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeInteractionChoiceStyle {
    Primary,
    Secondary,
    Danger,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeInteractionChoice {
    id: String,
    label: String,
    reply: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    style: Option<RuntimeInteractionChoiceStyle>,
}

impl RuntimeInteractionChoice {
    pub fn new(
        id: impl Into<String>,
        label: impl Into<String>,
        reply: impl Into<String>,
        style: Option<RuntimeInteractionChoiceStyle>,
    ) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            reply: reply.into(),
            style,
        }
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn label(&self) -> &str {
        &self.label
    }

    pub fn reply(&self) -> &str {
        &self.reply
    }

    pub fn style(&self) -> Option<RuntimeInteractionChoiceStyle> {
        self.style
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeInteractionCustomChoice {
    id: String,
    label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    placeholder: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    min_length: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    max_length: Option<usize>,
    #[serde(default, skip_serializing_if = "runtime_interaction_input_kind_is_text")]
    input_kind: RuntimeInteractionInputKind,
}

impl RuntimeInteractionCustomChoice {
    pub fn new(
        id: impl Into<String>,
        label: impl Into<String>,
        placeholder: Option<String>,
        min_length: Option<usize>,
        max_length: Option<usize>,
    ) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            placeholder,
            min_length,
            max_length,
            input_kind: RuntimeInteractionInputKind::Text,
        }
    }

    pub fn secret(
        id: impl Into<String>,
        label: impl Into<String>,
        placeholder: Option<String>,
        min_length: Option<usize>,
        max_length: Option<usize>,
    ) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            placeholder,
            min_length,
            max_length,
            input_kind: RuntimeInteractionInputKind::Secret,
        }
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn label(&self) -> &str {
        &self.label
    }

    pub fn placeholder(&self) -> Option<&str> {
        self.placeholder.as_deref()
    }

    pub fn min_length(&self) -> usize {
        self.min_length.unwrap_or(1)
    }

    pub fn max_length(&self) -> Option<usize> {
        self.max_length
    }

    pub fn input_kind(&self) -> RuntimeInteractionInputKind {
        self.input_kind
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeInteractionInputKind {
    Text,
    Secret,
}

impl Default for RuntimeInteractionInputKind {
    fn default() -> Self {
        Self::Text
    }
}

fn runtime_interaction_input_kind_is_text(kind: &RuntimeInteractionInputKind) -> bool {
    *kind == RuntimeInteractionInputKind::Text
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeInteraction {
    id: String,
    agent_id: String,
    kind: RuntimeInteractionKind,
    level: RuntimeInteractionLevel,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    title: Option<String>,
    message: String,
    choices: Vec<RuntimeInteractionChoice>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    custom_choice: Option<RuntimeInteractionCustomChoice>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    timeout_sec: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    default_on_timeout: Option<String>,
    requested_at_ms: u64,
}

impl RuntimeInteraction {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        id: impl Into<String>,
        agent_id: impl Into<String>,
        kind: RuntimeInteractionKind,
        level: RuntimeInteractionLevel,
        title: Option<String>,
        message: impl Into<String>,
        choices: Vec<RuntimeInteractionChoice>,
        custom_choice: Option<RuntimeInteractionCustomChoice>,
        timeout_sec: Option<u64>,
        default_on_timeout: Option<String>,
    ) -> Self {
        Self {
            id: id.into(),
            agent_id: agent_id.into(),
            kind,
            level,
            title,
            message: message.into(),
            choices,
            custom_choice,
            timeout_sec,
            default_on_timeout,
            requested_at_ms: unix_epoch_ms(),
        }
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn agent_id(&self) -> &str {
        &self.agent_id
    }

    pub fn kind(&self) -> RuntimeInteractionKind {
        self.kind
    }

    pub fn level(&self) -> RuntimeInteractionLevel {
        self.level
    }

    pub fn title(&self) -> Option<&str> {
        self.title.as_deref()
    }

    pub fn message(&self) -> &str {
        &self.message
    }

    pub fn choices(&self) -> &[RuntimeInteractionChoice] {
        &self.choices
    }

    pub fn custom_choice(&self) -> Option<&RuntimeInteractionCustomChoice> {
        self.custom_choice.as_ref()
    }

    pub fn timeout_sec(&self) -> Option<u64> {
        self.timeout_sec
    }

    pub fn default_on_timeout(&self) -> Option<&str> {
        self.default_on_timeout.as_deref()
    }

    pub fn requested_at_ms(&self) -> u64 {
        self.requested_at_ms
    }

    pub fn choice(&self, choice_id: &str) -> Option<&RuntimeInteractionChoice> {
        self.choices.iter().find(|choice| choice.id() == choice_id)
    }

    pub fn with_agent_id(mut self, agent_id: impl Into<String>) -> Self {
        self.agent_id = agent_id.into();
        self
    }
}
