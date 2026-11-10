use serde::Serialize;
use serde_json::{json, Value};

use super::tool_state::{non_empty, CodexToolTranscriptState};
use crate::extension::RemoteExtensionManifest;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct CodexToolTranscriptUpdate {
    id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    status: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    input: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    output: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    raw: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    images: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    images_omitted: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    placement: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    authority: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    execution_location: Option<String>,
}

pub(in crate::provider::codex_runtime) fn render_codex_tool_transcript_update(
    state: &CodexToolTranscriptState,
    remote_extension_manifest: &RemoteExtensionManifest,
) -> Option<String> {
    let item = &state.item;
    let id = item.get("id").and_then(Value::as_str)?.to_string();
    let item_type = item.get("type").and_then(Value::as_str)?;
    let status = normalize_codex_tool_status(
        item.get("status")
            .and_then(Value::as_str)
            .unwrap_or("updated"),
    );

    let mut update = match item_type {
        "commandExecution" => CodexToolTranscriptUpdate {
            id,
            tool: Some("bash".to_string()),
            status: Some(status),
            title: None,
            description: item
                .get("cwd")
                .and_then(Value::as_str)
                .and_then(non_empty)
                .map(|cwd| format!("cwd {cwd}")),
            text: None,
            input: Some(json!({
                "command": item.get("command").and_then(Value::as_str).unwrap_or_default(),
                "cwd": item.get("cwd").and_then(Value::as_str).unwrap_or_default(),
            })),
            output: prefer_output(
                item.get("aggregatedOutput").and_then(Value::as_str),
                &state.streamed_output,
            ),
            error: command_execution_error(item),
            raw: command_execution_raw(item),
            images: None,
            images_omitted: None,
            placement: None,
            authority: None,
            execution_location: None,
        },
        "fileChange" => CodexToolTranscriptUpdate {
            id,
            tool: Some("apply_patch".to_string()),
            status: Some(status),
            title: item
                .get("changes")
                .and_then(Value::as_array)
                .map(|changes| format!("{} file changes", changes.len()))
                .filter(|title| !title.starts_with('0')),
            description: None,
            text: None,
            input: None,
            output: prefer_output(None, &state.streamed_output),
            error: None,
            raw: item
                .get("changes")
                .filter(|value| !is_empty_json_value(value))
                .map(render_json_value),
            images: None,
            images_omitted: None,
            placement: None,
            authority: None,
            execution_location: None,
        },
        "mcpToolCall" => {
            let (images, images_omitted) =
                crate::provider::bounded_result_images(item.pointer("/result/content"));
            CodexToolTranscriptUpdate {
                id,
                tool: item
                    .get("tool")
                    .and_then(Value::as_str)
                    .and_then(non_empty)
                    .map(str::to_string)
                    .or_else(|| Some("mcp".to_string())),
                status: Some(status),
                title: item
                    .get("server")
                    .and_then(Value::as_str)
                    .and_then(non_empty)
                    .map(str::to_string),
                description: None,
                text: (!state.progress_messages.is_empty())
                    .then(|| state.progress_messages.join("\n")),
                input: item
                    .get("arguments")
                    .filter(|value| !is_empty_json_value(value))
                    .cloned(),
                output: item
                    .get("result")
                    .and_then(mcp_result_without_images)
                    .map(|result| render_json_value(&result)),
                error: item
                    .get("error")
                    .and_then(|value| {
                        value
                            .get("message")
                            .and_then(Value::as_str)
                            .or_else(|| value.as_str())
                    })
                    .and_then(non_empty)
                    .map(str::to_string),
                raw: None,
                images: (!images.is_empty()).then_some(images),
                images_omitted: (images_omitted > 0).then_some(images_omitted),
                placement: None,
                authority: None,
                execution_location: None,
            }
        }
        "dynamicToolCall" => CodexToolTranscriptUpdate {
            id,
            tool: item
                .get("tool")
                .and_then(Value::as_str)
                .and_then(non_empty)
                .map(str::to_string)
                .or_else(|| Some("tool".to_string())),
            status: Some(status),
            title: None,
            description: None,
            text: None,
            input: item
                .get("arguments")
                .filter(|value| !is_empty_json_value(value))
                .cloned(),
            output: item
                .get("contentItems")
                .filter(|value| !is_empty_json_value(value))
                .map(render_json_value),
            error: item
                .get("success")
                .and_then(Value::as_bool)
                .filter(|success| !success)
                .map(|_| "Dynamic tool call failed".to_string()),
            raw: None,
            images: None,
            images_omitted: None,
            placement: None,
            authority: None,
            execution_location: None,
        },
        "collabAgentToolCall" => CodexToolTranscriptUpdate {
            id,
            tool: item
                .get("tool")
                .and_then(Value::as_str)
                .and_then(non_empty)
                .map(str::to_string)
                .or_else(|| Some("collab".to_string())),
            status: Some(status),
            title: None,
            description: None,
            text: None,
            input: Some(json!({
                "prompt": item.get("prompt").cloned().unwrap_or(Value::Null),
                "receiverThreadIds": item.get("receiverThreadIds").cloned().unwrap_or(Value::Null),
                "model": item.get("model").cloned().unwrap_or(Value::Null),
                "reasoningEffort": item.get("reasoningEffort").cloned().unwrap_or(Value::Null),
            })),
            output: item
                .get("agentsStates")
                .filter(|value| !is_empty_json_value(value))
                .map(render_json_value),
            error: None,
            raw: None,
            images: None,
            images_omitted: None,
            placement: None,
            authority: None,
            execution_location: None,
        },
        "webSearch" => CodexToolTranscriptUpdate {
            id,
            tool: Some("web_search".to_string()),
            status: Some(status),
            title: None,
            description: None,
            text: None,
            input: web_search_input(item),
            output: None,
            error: None,
            raw: None,
            images: None,
            images_omitted: None,
            placement: None,
            authority: None,
            execution_location: None,
        },
        "imageView" => CodexToolTranscriptUpdate {
            id,
            tool: Some("view_image".to_string()),
            status: Some(status),
            title: None,
            description: None,
            text: None,
            input: item
                .get("path")
                .and_then(Value::as_str)
                .and_then(non_empty)
                .map(|path| json!({ "path": path })),
            output: None,
            error: None,
            raw: None,
            images: state
                .pictures
                .as_ref()
                .map(|pictures| pictures.images.clone())
                .filter(|images| !images.is_empty()),
            images_omitted: state
                .pictures
                .as_ref()
                .map(|pictures| pictures.omitted)
                .filter(|omitted| *omitted > 0),
            placement: None,
            authority: None,
            execution_location: None,
        },
        // Codex keeps the file under its own home and shows the picture itself; the item's own copy is what the pane draws.
        "imageGeneration" => CodexToolTranscriptUpdate {
            id,
            tool: Some("image_generation".to_string()),
            status: Some(status),
            title: None,
            description: None,
            text: None,
            input: item
                .get("revisedPrompt")
                .and_then(Value::as_str)
                .and_then(non_empty)
                .map(|prompt| json!({ "prompt": prompt })),
            output: None,
            error: None,
            raw: None,
            images: item
                .get("result")
                .and_then(Value::as_str)
                .and_then(non_empty)
                .map(|result| {
                    vec![if result.starts_with("data:image/") {
                        result.to_string()
                    } else {
                        format!("data:image/png;base64,{result}")
                    }]
                }),
            images_omitted: None,
            placement: None,
            authority: None,
            execution_location: None,
        },
        _ => return None,
    };
    if codex_item_is_home_proxy(item, remote_extension_manifest) {
        update.placement = Some("home-proxy".to_string());
        update.authority = Some("home".to_string());
        update.execution_location = Some("home".to_string());
    }

    serde_json::to_string(&TimedCodexToolTranscriptUpdate {
        update: &update,
        started_at_ms: state.started_at_ms,
        background: state.background.then_some(true),
        finished_at_ms: state.background.then_some(state.finished_at_ms),
    })
    .ok()
}

/// The call's lifecycle times: when the kernel saw it start, and for a command
/// left running in the background, when it ended (`null` while it runs).
#[derive(Serialize)]
struct TimedCodexToolTranscriptUpdate<'a> {
    #[serde(flatten)]
    update: &'a CodexToolTranscriptUpdate,
    started_at_ms: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    background: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    finished_at_ms: Option<Option<u64>>,
}

fn codex_item_is_home_proxy(
    item: &Value,
    remote_extension_manifest: &RemoteExtensionManifest,
) -> bool {
    ["tool", "server"]
        .into_iter()
        .filter_map(|field| item.get(field).and_then(Value::as_str))
        .any(|name| remote_extension_manifest.home_proxy_tool(name).is_some())
}

fn normalize_codex_tool_status(status: &str) -> String {
    match status {
        "inProgress" => "running".to_string(),
        "completed" => "completed".to_string(),
        "failed" => "error".to_string(),
        "declined" => "declined".to_string(),
        other => non_empty(other).unwrap_or("updated").to_string(),
    }
}

fn command_execution_error(item: &Value) -> Option<String> {
    let status = item
        .get("status")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if status != "failed" && status != "declined" {
        return None;
    }
    item.get("aggregatedOutput")
        .and_then(Value::as_str)
        .and_then(non_empty)
        .map(str::to_string)
        .or_else(|| Some(format!("Command {status}")))
}

fn command_execution_raw(item: &Value) -> Option<String> {
    let exit_code = item.get("exitCode").and_then(Value::as_i64);
    let duration = item.get("durationMs").and_then(Value::as_i64);
    let process_id = item
        .get("processId")
        .and_then(Value::as_str)
        .and_then(non_empty);
    let mut lines = Vec::new();
    if let Some(exit_code) = exit_code {
        lines.push(format!("exit_code: {exit_code}"));
    }
    if let Some(duration) = duration {
        lines.push(format!("duration_ms: {duration}"));
    }
    if let Some(process_id) = process_id {
        lines.push(format!("process_id: {process_id}"));
    }
    (!lines.is_empty()).then(|| lines.join("\n"))
}

fn prefer_output(primary: Option<&str>, streamed_output: &str) -> Option<String> {
    primary
        .and_then(non_empty)
        .map(super::output_window::bounded)
        .or_else(|| non_empty(streamed_output).map(super::output_window::bounded))
}

/// An MCP result without its image blocks, which the update carries as `images`; nothing when only pictures came back.
fn mcp_result_without_images(result: &Value) -> Option<Value> {
    let mut result = result.clone();
    if let Some(content) = result.get_mut("content").and_then(Value::as_array_mut) {
        content.retain(|block| block.get("type").and_then(Value::as_str) != Some("image"));
    }
    let empty = match &result {
        Value::Object(fields) => fields.values().all(is_empty_json_value),
        other => is_empty_json_value(other),
    };
    (!empty).then_some(result)
}

fn render_json_value(value: &Value) -> String {
    serde_json::to_string_pretty(value).unwrap_or_else(|_| value.to_string())
}

fn is_empty_json_value(value: &Value) -> bool {
    match value {
        Value::Null => true,
        Value::Array(items) => items.is_empty(),
        Value::Object(items) => items.is_empty(),
        Value::String(text) => text.trim().is_empty(),
        _ => false,
    }
}

/// What a web search item asks for: its query (or queries), or the page it opened or looked inside.
fn web_search_input(item: &Value) -> Option<Value> {
    fn text(value: Option<&Value>) -> Option<&str> {
        value.and_then(Value::as_str).and_then(non_empty)
    }
    let mut input = serde_json::Map::new();
    let queries: Vec<&str> = item
        .pointer("/action/queries")
        .and_then(Value::as_array)
        .map(|queries| {
            queries
                .iter()
                .filter_map(|query| text(Some(query)))
                .collect()
        })
        .unwrap_or_default();
    let query = text(item.get("query"))
        .or_else(|| text(item.pointer("/action/query")))
        .or_else(|| queries.first().copied());
    if let Some(query) = query {
        input.insert("query".to_string(), json!(query));
    }
    if queries.len() > 1 {
        input.insert("queries".to_string(), json!(queries));
    }
    for key in ["url", "pattern"] {
        if let Some(value) = text(item.pointer(&format!("/action/{key}"))) {
            input.insert(key.to_string(), json!(value));
        }
    }
    (!input.is_empty()).then_some(Value::Object(input))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rendered(item: Value) -> Value {
        let state = CodexToolTranscriptState {
            item,
            streamed_output: String::new(),
            pending_output_bytes: Vec::new(),
            output_stream: None,
            progress_messages: Vec::new(),
            last_emitted: None,
            pictures: None,
            started_at_ms: 0,
            background: false,
            finished_at_ms: None,
        };
        let text = render_codex_tool_transcript_update(&state, &RemoteExtensionManifest::default())
            .expect("a tool update");
        serde_json::from_str(&text).unwrap()
    }

    #[test]
    fn web_searches_and_image_views_are_tool_calls() {
        let search = rendered(
            json!({"type": "webSearch", "id": "ws-1", "status": "completed", "query": "svg viewbox"}),
        );
        assert_eq!(search["tool"], "web_search");
        assert_eq!(search["status"], "completed");
        assert_eq!(search["input"], json!({"query": "svg viewbox"}));
        let action = rendered(
            json!({"type": "webSearch", "id": "ws-2", "action": {"type": "search", "query": "svg path"}}),
        );
        assert_eq!(action["input"], json!({"query": "svg path"}));
        let page = rendered(
            json!({"type": "webSearch", "id": "ws-3", "query": "", "action": {"type": "openPage", "url": "https://example.com/a"}}),
        );
        assert_eq!(page["input"], json!({"url": "https://example.com/a"}));
        let find = rendered(
            json!({"type": "webSearch", "id": "ws-4", "action": {"type": "findInPage", "url": "https://example.com/a", "pattern": "tickets"}}),
        );
        assert_eq!(
            find["input"],
            json!({"url": "https://example.com/a", "pattern": "tickets"})
        );
        let many = rendered(
            json!({"type": "webSearch", "id": "ws-5", "query": "a b", "action": {"type": "search", "query": "a b", "queries": ["a b", "c d"]}}),
        );
        assert_eq!(
            many["input"],
            json!({"query": "a b", "queries": ["a b", "c d"]})
        );
        let view = rendered(json!({"type": "imageView", "id": "iv-1", "path": "/work/chart.png"}));
        assert_eq!(view["tool"], "view_image");
        assert_eq!(view["input"], json!({"path": "/work/chart.png"}));
        assert!(super::super::item::is_codex_tool_item(
            &json!({"type": "webSearch"})
        ));
    }
    #[test]
    fn an_mcp_tool_result_carries_its_image_blocks_as_pictures() {
        let chart = rendered(json!({
            "type": "mcpToolCall", "id": "mcp-1", "status": "completed", "server": "charts", "tool": "render_chart",
            "arguments": {"kind": "bar"},
            "result": {"content": [
                {"type": "text", "text": "Rendered"},
                {"type": "image", "data": "iVBORw0KGgo=", "mimeType": "image/png"},
            ], "structuredContent": null},
        }));
        assert_eq!(
            chart["images"],
            json!(["data:image/png;base64,iVBORw0KGgo="])
        );
        let output = chart["output"].as_str().expect("the text stays the output");
        assert!(output.contains("Rendered") && !output.contains("iVBORw0KGgo="));

        let picture_only = rendered(json!({
            "type": "mcpToolCall", "id": "mcp-2", "status": "completed", "tool": "render_chart",
            "result": {"content": [{"type": "image", "data": "iVBORw0KGgo=", "mimeType": "image/png"}]},
        }));
        assert_eq!(
            picture_only["images"],
            json!(["data:image/png;base64,iVBORw0KGgo="])
        );
        assert!(picture_only.get("output").is_none());

        let text_only = rendered(json!({
            "type": "mcpToolCall", "id": "mcp-3", "status": "completed", "tool": "lookup",
            "result": {"content": [{"type": "text", "text": "42"}]},
        }));
        assert!(text_only.get("images").is_none() && text_only.get("images_omitted").is_none());
        assert!(text_only["output"].as_str().unwrap().contains("42"));
    }

    // MP-08: Codex shows a built-in generated image to the user itself and keeps the file under its own
    // home, outside the workspace; the item's own picture is the only copy the pane can draw.
    #[test]
    fn generated_images_are_tool_calls_that_carry_their_picture() {
        let item = json!({"type": "imageGeneration", "id": "ig-1", "status": "completed", "revisedPrompt": "A bar chart of three models", "result": "iVBORw0KGgo=", "savedPath": "/home/codex/generated_images/t/ig-1.png"});
        assert!(super::super::item::is_codex_tool_item(&item));
        let image = rendered(item);
        assert_eq!(image["tool"], "image_generation");
        assert_eq!(image["status"], "completed");
        assert_eq!(
            image["input"],
            json!({"prompt": "A bar chart of three models"})
        );
        assert_eq!(
            image["images"],
            json!(["data:image/png;base64,iVBORw0KGgo="])
        );
        assert!(
            image.get("output").is_none(),
            "the saved path outside the workspace is not offered as a file"
        );
        let pending = rendered(
            json!({"type": "imageGeneration", "id": "ig-2", "status": "in_progress", "result": ""}),
        );
        assert!(
            pending.get("images").is_none(),
            "no picture before the generator returns one"
        );
    }
}
