//! OpenCode prompt request body construction.

use serde_json::json;

use crate::error::DaemonError;
use crate::provider::AgentExecutionMode;
use crate::session::PromptAttachment;

use super::OpenCodeClient;

impl OpenCodeClient {
    pub fn submit_prompt(
        &self,
        session_id: &str,
        message_id: &str,
        prompt: &str,
        attachments: &[PromptAttachment],
        hidden_system_context: Option<&str>,
        model: Option<&str>,
        variant: Option<&str>,
        execution_mode: AgentExecutionMode,
        disable_native_writes: bool,
        allow_native_bash: bool,
    ) -> Result<(), DaemonError> {
        let mut parts = Vec::new();
        if !prompt.is_empty() {
            parts.push(json!({
                "type": "text",
                "text": prompt,
            }));
        }
        for attachment in attachments {
            parts.push(json!({
                "type": "file",
                "mime": attachment.mime(),
                "url": attachment.url(),
                "filename": attachment.filename(),
            }));
        }
        let mut body = json!({
            "messageID": message_id,
            "parts": parts,
            "agent": opencode_agent_for_execution_mode(execution_mode),
        });
        if let Some(system) = hidden_system_context
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            body["system"] = json!(system);
        }
        if let Some((provider_id, model_id)) = parse_model(model) {
            body["model"] = json!({
                "providerID": provider_id,
                "modelID": model_id,
            });
        }
        if let Some(variant) = variant.map(str::trim).filter(|value| !value.is_empty()) {
            body["variant"] = json!(variant);
        }
        if disable_native_writes {
            let mut tools = serde_json::Map::from_iter([
                ("edit".to_string(), json!(false)),
                ("write".to_string(), json!(false)),
                ("apply_patch".to_string(), json!(false)),
                ("multiedit".to_string(), json!(false)),
                ("task".to_string(), json!(false)),
            ]);
            tools.insert("bash".to_string(), json!(allow_native_bash));
            body["tools"] = serde_json::Value::Object(tools);
        }

        self.send_no_content_request(
            "POST",
            &format!("/session/{session_id}/prompt_async"),
            Some(&body),
        )?;
        Ok(())
    }
}

fn opencode_agent_for_execution_mode(execution_mode: AgentExecutionMode) -> &'static str {
    match execution_mode {
        AgentExecutionMode::Build => "build",
        AgentExecutionMode::Plan => "plan",
    }
}

pub(super) fn parse_model(model: Option<&str>) -> Option<(&str, &str)> {
    let value = model?.trim();
    if value.is_empty() || value == "default" {
        return None;
    }
    value.split_once('/')
}
