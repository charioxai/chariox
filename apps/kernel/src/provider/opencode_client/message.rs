//! OpenCode message, part, and token contracts.

use serde::Deserialize;
use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct OpenCodeMessage {
    pub info: OpenCodeMessageInfo,
    #[serde(default)]
    pub parts: Vec<OpenCodePart>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct OpenCodeMessageInfo {
    pub id: String,
    #[serde(rename = "sessionID")]
    pub session_id: String,
    pub role: String,
    #[serde(rename = "parentID", default)]
    pub parent_id: Option<String>,
    #[serde(default)]
    pub finish: Option<String>,
    #[serde(default)]
    pub error: Option<Value>,
    #[serde(rename = "providerID", default)]
    pub provider_id: Option<String>,
    #[serde(rename = "modelID", default)]
    pub model_id: Option<String>,
    #[serde(default)]
    pub model: Option<OpenCodeSelectedModel>,
    #[serde(default)]
    pub variant: Option<String>,
    #[serde(default)]
    pub tokens: OpenCodeMessageTokens,
    #[serde(default)]
    pub time: OpenCodeMessageTime,
}

impl OpenCodeMessageInfo {
    pub fn is_tool_call_only_completion(&self) -> bool {
        self.finish.as_deref() == Some("tool-calls")
    }

    pub fn is_terminal_assistant_completion(&self) -> bool {
        if self.error.is_some() {
            return true;
        }
        self.time.completed.is_some()
            && self
                .finish
                .as_deref()
                .is_some_and(|finish| finish != "tool-calls" && finish != "unknown")
    }

    pub fn resolved_model(&self) -> Option<String> {
        if let (Some(provider_id), Some(model_id)) =
            (self.provider_id.as_deref(), self.model_id.as_deref())
        {
            return Some(format!("{provider_id}/{model_id}"));
        }

        self.model
            .as_ref()
            .map(|model| format!("{}/{}", model.provider_id, model.model_id))
    }

    pub fn resolved_variant(&self) -> Option<String> {
        self.variant
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
    }

    pub fn total_tokens(&self) -> u64 {
        self.tokens.total()
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
pub struct OpenCodeMessageTokens {
    #[serde(default)]
    pub input: u64,
    #[serde(default)]
    pub output: u64,
    #[serde(default)]
    pub reasoning: u64,
    #[serde(default)]
    pub cache: OpenCodeMessageCacheTokens,
}

impl OpenCodeMessageTokens {
    pub fn total(&self) -> u64 {
        self.input + self.output + self.reasoning + self.cache.read + self.cache.write
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
pub struct OpenCodeMessageCacheTokens {
    #[serde(default)]
    pub read: u64,
    #[serde(default)]
    pub write: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct OpenCodeSelectedModel {
    #[serde(rename = "providerID")]
    pub provider_id: String,
    #[serde(rename = "modelID")]
    pub model_id: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
pub struct OpenCodeMessageTime {
    #[serde(default)]
    pub completed: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct OpenCodePart {
    pub id: String,
    #[serde(rename = "sessionID", default)]
    pub session_id: String,
    #[serde(rename = "messageID", default)]
    pub message_id: String,
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(default)]
    pub text: String,
    #[serde(default)]
    pub tool: String,
    #[serde(default)]
    pub state: Option<OpenCodeToolState>,
    #[serde(default)]
    pub time: Option<OpenCodePartTime>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
pub struct OpenCodeToolState {
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub input: serde_json::Value,
    #[serde(default)]
    pub output: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub metadata: serde_json::Value,
    #[serde(default)]
    pub error: String,
    #[serde(default)]
    pub raw: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
pub struct OpenCodePartTime {
    #[serde(default)]
    pub end: Option<u64>,
}
