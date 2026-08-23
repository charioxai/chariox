//! OpenCode transcript and status rendering.

use crate::extension::RemoteExtensionManifest;
use crate::provider::opencode_client::OpenCodePart;

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(super) struct ToolTranscriptUpdate {
    pub(super) id: String,
    pub(super) tool: String,
    pub(super) status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) input: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) output: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) raw: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) placement: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) authority: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) execution_location: Option<String>,
}

pub(super) fn render_tool_transcript_update(
    part: &OpenCodePart,
    remote_extension_manifest: &RemoteExtensionManifest,
) -> String {
    let tool_name = if part.tool.is_empty() {
        "tool"
    } else {
        part.tool.as_str()
    };
    let status = part
        .state
        .as_ref()
        .map(|state| state.status.as_str())
        .filter(|status: &&str| !status.is_empty())
        .unwrap_or("updated");
    let rendered_text = (!part.text.trim().is_empty()).then(|| part.text.trim().to_string());
    let input = part.state.as_ref().and_then(|state| {
        (!state.input.is_null() && !is_empty_json_value(&state.input)).then(|| state.input.clone())
    });
    let output = part
        .state
        .as_ref()
        .and_then(|state| non_empty(state.output.as_str()).map(str::to_string))
        .or_else(|| tool_metadata_field(part, &["output", "stdout"]));
    let description = tool_metadata_field(part, &["description"]);
    let title = part
        .state
        .as_ref()
        .and_then(|state| non_empty(state.title.as_str()).map(str::to_string));
    let error = part
        .state
        .as_ref()
        .and_then(|state| non_empty(state.error.as_str()).map(str::to_string));
    let raw = part
        .state
        .as_ref()
        .and_then(|state| non_empty(state.raw.as_str()))
        .map(render_tool_raw_detail)
        .filter(|value| {
            rendered_text.as_deref() != Some(value.as_str())
                && output.as_deref() != Some(value.as_str())
        });
    let is_home_proxy = remote_extension_manifest
        .home_proxy_tool(tool_name)
        .is_some();

    serde_json::to_string(&ToolTranscriptUpdate {
        id: part.id.clone(),
        tool: tool_name.to_string(),
        status: status.to_string(),
        title,
        description,
        text: rendered_text,
        input,
        output,
        error,
        raw,
        placement: is_home_proxy.then(|| "home-proxy".to_string()),
        authority: is_home_proxy.then(|| "home".to_string()),
        execution_location: is_home_proxy.then(|| "home".to_string()),
    })
    .unwrap_or_else(|_| {
        format!(
            "{{\"id\":{id:?},\"tool\":{tool:?},\"status\":{status:?}}}",
            id = part.id,
            tool = tool_name,
            status = status,
        )
    })
}

pub(super) fn render_session_error_transcript_update(message: &str) -> String {
    let message = non_empty(message).unwrap_or("OpenCode reported an unknown session error.");
    format!("**OpenCode error**\n\n{message}")
}

fn non_empty(value: &str) -> Option<&str> {
    let trimmed = value.trim();
    (!trimmed.is_empty()).then_some(trimmed)
}

fn is_empty_json_value(value: &serde_json::Value) -> bool {
    match value {
        serde_json::Value::Null => true,
        serde_json::Value::Array(items) => items.is_empty(),
        serde_json::Value::Object(items) => items.is_empty(),
        _ => false,
    }
}

fn tool_metadata_field(part: &OpenCodePart, keys: &[&str]) -> Option<String> {
    let metadata = part.state.as_ref()?.metadata.as_object()?;
    keys.iter().find_map(|key| {
        metadata
            .get(*key)
            .and_then(serde_json::Value::as_str)
            .and_then(non_empty)
            .map(str::to_string)
    })
}

fn render_tool_raw_detail(raw: &str) -> String {
    match serde_json::from_str::<serde_json::Value>(raw) {
        Ok(value) => serde_json::to_string_pretty(&value).unwrap_or_else(|_| raw.to_string()),
        Err(_) => raw.to_string(),
    }
}

pub(super) fn format_session_status(kind: &str) -> String {
    match kind {
        "busy" => "OpenCode is thinking...".to_string(),
        "idle" => "OpenCode is idle.".to_string(),
        "retry" | "reconnecting" => crate::provider::provider_retry_status("OpenCode", None),
        other => format!("OpenCode status: {other}"),
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn retry_and_reconnecting_statuses_use_the_shared_connection_message() {
        for status in ["retry", "reconnecting"] {
            assert_eq!(
                super::format_session_status(status),
                "OpenCode connection interrupted — retrying."
            );
        }
    }
}
