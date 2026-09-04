use serde_json::{json, Value};
use std::collections::BTreeMap;

#[derive(Default)]
pub(super) struct ClaudeToolTranscript {
    pending: BTreeMap<String, Value>,
}

impl ClaudeToolTranscript {
    pub(super) fn observe(&mut self, value: &Value) -> Vec<Value> {
        let kind = value.get("type").and_then(Value::as_str);
        let message = value.get("message").unwrap_or(value);
        let Some(content) = message.get("content").and_then(Value::as_array) else {
            return Vec::new();
        };
        content
            .iter()
            .filter_map(|block| {
                if kind == Some("user")
                    && block.get("type").and_then(Value::as_str) == Some("tool_result")
                {
                    let id = block.get("tool_use_id").and_then(Value::as_str)?;
                    let mut payload = self.pending.remove(id)?;
                    let failed = block.get("is_error").and_then(Value::as_bool) == Some(true);
                    payload["status"] = json!(if failed { "error" } else { "completed" });
                    let output = match block.get("content") {
                        Some(Value::String(text)) => text.clone(),
                        Some(Value::Array(items)) => items
                            .iter()
                            .filter(|item| item.get("type").and_then(Value::as_str) == Some("text"))
                            .filter_map(|item| item.get("text").and_then(Value::as_str))
                            .collect::<Vec<_>>()
                            .join("\n"),
                        _ => String::new(),
                    };
                    payload[if failed { "error" } else { "output" }] = json!(output);
                    return Some(payload);
                }
                if kind != Some("assistant")
                    || block.get("type").and_then(Value::as_str) != Some("tool_use")
                {
                    return None;
                }
                let id = block.get("id").and_then(Value::as_str)?;
                let name = block.get("name").and_then(Value::as_str)?;
                if self.pending.contains_key(id) || is_unsupported_claude_stream_json_tool(name) {
                    return None;
                }
                let payload = json!({
                    "tool": name,
                    "status": "running",
                    "input": block.get("input").cloned().unwrap_or(Value::Null),
                    "id": id,
                });
                self.pending.insert(id.to_string(), payload.clone());
                Some(payload)
            })
            .collect()
    }

    pub(super) fn clear(&mut self) {
        self.pending.clear();
    }
}

pub(super) fn is_unsupported_claude_stream_json_tool(name: &str) -> bool {
    name == "ToolSearch"
}

#[cfg(test)]
mod tests {
    use super::*;

    fn invocation(id: &str, name: &str) -> Value {
        json!({"type":"assistant","message":{"content":[{
            "type":"tool_use","id":id,"name":name,"input":{"field_id":id}
        }]}})
    }

    fn result(id: &str, content: Value) -> Value {
        json!({"type":"user","message":{"content":[{
            "type":"tool_result","tool_use_id":id,"content":content
        }]}})
    }

    #[test]
    fn concurrent_tool_results_keep_their_own_input_and_ignore_duplicate_results() {
        let mut transcript = ClaudeToolTranscript::default();
        transcript.observe(&invocation("first", "slice_browser_find"));
        transcript.observe(&invocation("second", "slice_browser_fill"));
        let second = transcript.observe(&result("second", json!("filled")));
        assert_eq!(second[0]["tool"], "slice_browser_fill");
        assert_eq!(second[0]["input"]["field_id"], "second");
        assert_eq!(second[0]["status"], "completed");
        assert_eq!(second[0]["output"], "filled");
        assert!(transcript
            .observe(&result("second", json!("duplicate")))
            .is_empty());
        let first = transcript.observe(&result(
            "first",
            json!([
                {"type":"text","text":"one"},{"type":"image","data":"image-not-transcript"},
                {"type":"text","text":"two"}
            ]),
        ));
        assert_eq!(first[0]["input"]["field_id"], "first");
        assert_eq!(first[0]["output"], "one\ntwo");
    }

    #[test]
    fn reset_and_unknown_results_cannot_reuse_previous_turn_inputs() {
        let mut transcript = ClaudeToolTranscript::default();
        assert!(transcript
            .observe(&result("missing", json!("unknown")))
            .is_empty());
        transcript.observe(&invocation("old", "slice_browser_fill"));
        transcript.clear();
        assert!(transcript
            .observe(&result("old", json!("late result")))
            .is_empty());
        assert!(transcript
            .observe(&invocation("unsupported", "ToolSearch"))
            .is_empty());
        assert!(transcript
            .observe(&result("unsupported", json!("late")))
            .is_empty());
    }

    #[test]
    fn prompt_text_or_assistant_result_blocks_do_not_impersonate_tool_results() {
        let mut transcript = ClaudeToolTranscript::default();
        transcript.observe(&invocation("first", "slice_browser_fill"));
        let mut wrong_role = result("first", json!("not a result"));
        wrong_role["type"] = json!("assistant");
        assert!(transcript.observe(&wrong_role).is_empty());
        assert!(transcript
            .observe(&json!({"type":"user","message":{"content":"tool_result first"}}))
            .is_empty());
        assert_eq!(
            transcript.observe(&result("first", json!("real result")))[0]["output"],
            "real result"
        );
    }

    #[test]
    fn claude_tool_transcript_projects_actual_result_error() {
        let mut transcript = ClaudeToolTranscript::default();
        let started = transcript.observe(
            &json!({"type":"assistant","message":{"role":"assistant","content":[{
                "type":"tool_use","id":"toolu_fill","name":"mcp__chariox__slice_browser_fill",
                "input":{"text":"STALE ATTEMPT MUST NOT LAND","element_ref":"old"}
            }]}}),
        );
        let finished =
            transcript.observe(&json!({"type":"user","message":{"role":"user","content":[{
                "type":"tool_result","tool_use_id":"toolu_fill","is_error":true,
                "content":[{"type":"text","text":"environment_stale_element_reference"}]
            }]}}));
        assert_eq!(
            finished.len(),
            1,
            "the actual tool result must reach history"
        );
        assert_eq!(started[0]["status"], "running");
        assert_eq!(finished[0]["id"], "toolu_fill");
        assert_eq!(finished[0]["tool"], "mcp__chariox__slice_browser_fill");
        assert_eq!(finished[0]["input"]["text"], "STALE ATTEMPT MUST NOT LAND");
        assert_eq!(finished[0]["error"], "environment_stale_element_reference");
        assert_eq!(finished[0]["status"], "error");
    }
}
