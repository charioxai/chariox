use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::io::{self, Write};

const INPUT_BYTES: usize = 8 * 1024;
const PENDING_ENTRIES: usize = 64;
const PENDING_BYTES: usize = 256 * 1024;
const IDENTITY_BYTES: usize = 256;
const MESSAGE_BLOCKS: usize = 64;
const RESULT_BYTES: usize = 16 * 1024;
const RESULT_BLOCKS: usize = 256;
const TRUNCATED: &str = "\n[chariox: tool transcript truncated]";

struct BoundedJson(Vec<u8>);

impl Write for BoundedJson {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if bytes.len() > INPUT_BYTES.saturating_sub(self.0.len()) {
            return Err(io::Error::other("tool input transcript limit"));
        }
        self.0.extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn bounded_input(input: &Value) -> Vec<u8> {
    let mut encoded = BoundedJson(Vec::new());
    if serde_json::to_writer(&mut encoded, input).is_err() {
        return br#"{"chariox_truncated":true}"#.to_vec();
    }
    encoded.0
}

fn append_result(output: &mut String, text: &str) -> bool {
    let mut length = text.len().min(RESULT_BYTES.saturating_sub(output.len()));
    while !text.is_char_boundary(length) {
        length -= 1;
    }
    output.push_str(&text[..length]);
    length < text.len()
}

fn bounded_result(content: Option<&Value>) -> String {
    let mut output = String::new();
    let truncated = match content {
        Some(Value::String(text)) => append_result(&mut output, text),
        Some(Value::Array(items)) => {
            let mut truncated = items.len() > RESULT_BLOCKS;
            let mut text_seen = false;
            for item in items.iter().take(RESULT_BLOCKS) {
                if item.get("type").and_then(Value::as_str) != Some("text") {
                    continue;
                }
                let Some(text) = item.get("text").and_then(Value::as_str) else {
                    continue;
                };
                if (text_seen && append_result(&mut output, "\n"))
                    || append_result(&mut output, text)
                {
                    truncated = true;
                    break;
                }
                text_seen = true;
            }
            truncated
        }
        _ => false,
    };
    if truncated {
        let mut length = output.len().min(RESULT_BYTES - TRUNCATED.len());
        while !output.is_char_boundary(length) {
            length -= 1;
        }
        output.truncate(length);
        output.push_str(TRUNCATED);
    }
    output
}

#[derive(Default)]
pub(super) struct ClaudeToolTranscript {
    pending: BTreeMap<String, PendingTool>,
    pending_bytes: usize,
    truncated: bool,
    truncation_reported: bool,
    unsupported_results: Vec<Value>,
}

struct PendingTool {
    name: String,
    // Retain bounded encoded input, not an expanded JSON tree per pending call.
    input: Vec<u8>,
}

impl PendingTool {
    fn bytes(&self, id: &str) -> usize {
        id.len() + self.name.len() + self.input.len()
    }

    fn payload(&self, id: &str) -> Value {
        let input: Value = serde_json::from_slice(&self.input)
            .unwrap_or_else(|_| json!({"chariox_truncated": true}));
        json!({"tool": self.name, "status": "running", "input": input, "id": id})
    }
}

impl ClaudeToolTranscript {
    pub(super) fn observe(&mut self, value: &Value) -> Vec<Value> {
        self.unsupported_results.clear();
        let kind = value.get("type").and_then(Value::as_str);
        let message = value.get("message").unwrap_or(value);
        let Some(content) = message.get("content").and_then(Value::as_array) else {
            return Vec::new();
        };
        self.truncated |= content.len() > MESSAGE_BLOCKS;
        content
            .iter()
            .take(MESSAGE_BLOCKS)
            .filter_map(|block| {
                if kind == Some("user")
                    && block.get("type").and_then(Value::as_str) == Some("tool_result")
                {
                    let id = block.get("tool_use_id").and_then(Value::as_str)?;
                    if id.len() > IDENTITY_BYTES {
                        self.truncated = true;
                        return None;
                    }
                    let pending = self.pending.remove(id)?;
                    self.pending_bytes -= pending.bytes(id);
                    let mut payload = pending.payload(id);
                    let failed = block.get("is_error").and_then(Value::as_bool) == Some(true);
                    payload["status"] = json!(if failed { "error" } else { "completed" });
                    let output = bounded_result(block.get("content"));
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
                if id.len() > IDENTITY_BYTES || name.len() > IDENTITY_BYTES {
                    self.truncated = true;
                    return None;
                }
                if is_unsupported_claude_stream_json_tool(name) {
                    self.unsupported_results.push(json!({
                        "type": "tool_result", "tool_use_id": id, "is_error": true,
                        "content": "Chariox does not execute Claude stream-json tool `ToolSearch` in this runtime path. If this is a Chariox workflow turn, do not search for workflow tools; emit the required fenced JSON fallback directly.",
                    }));
                    return None;
                }
                if self.pending.contains_key(id) {
                    return None;
                }
                if self.pending.len() >= PENDING_ENTRIES {
                    self.truncated = true;
                    return None;
                }
                let pending = PendingTool {
                    name: name.to_string(),
                    input: bounded_input(block.get("input").unwrap_or(&Value::Null)),
                };
                if pending.bytes(id) > PENDING_BYTES.saturating_sub(self.pending_bytes) {
                    self.truncated = true;
                    return None;
                }
                let payload = pending.payload(id);
                self.pending_bytes += pending.bytes(id);
                self.pending.insert(id.to_string(), pending);
                Some(payload)
            })
            .collect()
    }

    pub(super) fn clear(&mut self) {
        self.pending.clear();
        self.pending_bytes = 0;
        self.truncated = false;
        self.truncation_reported = false;
        self.unsupported_results.clear();
    }

    pub(super) fn take_unsupported_results(&mut self) -> Vec<Value> {
        std::mem::take(&mut self.unsupported_results)
    }

    pub(super) fn take_truncation_notice(&mut self) -> bool {
        if !self.truncated || self.truncation_reported {
            return false;
        }
        self.truncation_reported = true;
        true
    }
}

fn is_unsupported_claude_stream_json_tool(name: &str) -> bool {
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
    fn oversized_tool_input_is_marked_before_retention() {
        let mut transcript = ClaudeToolTranscript::default();
        let mut event = invocation("large", "slice_browser_fill");
        event["message"]["content"][0]["input"] = json!({"text": "x".repeat(1_000_000)});
        let started = transcript.observe(&event);
        assert!(serde_json::to_vec(&started).unwrap().len() < 10_000);
        assert_eq!(started[0]["input"]["chariox_truncated"], true);
        let finished = transcript.observe(&result("large", json!("done")));
        assert_eq!(finished[0]["input"]["chariox_truncated"], true);
        assert_eq!(finished[0]["output"], "done");
    }

    #[test]
    fn unsupported_rejections_share_message_and_identity_budgets() {
        let mut transcript = ClaudeToolTranscript::default();
        let mut blocks =
            vec![invocation(&"x".repeat(1_000_000), "ToolSearch")["message"]["content"][0].clone()];
        blocks.extend((0..1000).map(|index| {
            invocation(&format!("id-{index}"), "ToolSearch")["message"]["content"][0].clone()
        }));
        assert!(transcript
            .observe(&json!({"type":"assistant","message":{"content":blocks}}))
            .is_empty());
        let rejected = transcript.take_unsupported_results();
        assert_eq!(rejected.len(), 63);
        assert!(serde_json::to_vec(&rejected).unwrap().len() < 32 * 1024);
        assert_eq!(rejected[0]["tool_use_id"], "id-0");
        assert_eq!(rejected[0]["is_error"], true);
        assert!(transcript.take_truncation_notice());
        assert!(transcript.take_unsupported_results().is_empty());
    }

    #[test]
    fn oversized_result_text_is_bounded_without_joining_all_blocks() {
        for content in [
            json!("é".repeat(100_000)),
            json!((0..1000)
                .map(|_| json!({"type":"text","text":"界".repeat(1000)}))
                .collect::<Vec<_>>()),
        ] {
            let mut transcript = ClaudeToolTranscript::default();
            transcript.observe(&invocation("large", "slice_browser_find"));
            let mut completed = result("large", content);
            completed["message"]["content"][0]["is_error"] = json!(true);
            let finished = transcript.observe(&completed);
            let text = finished[0]["error"].as_str().unwrap();
            assert!(text.len() <= 16 * 1024);
            assert!(text.ends_with("[chariox: tool transcript truncated]"));
            assert_eq!(finished[0]["status"], "error");
        }
    }

    #[test]
    fn unmatched_invocations_have_a_count_budget_and_completed_slots_are_reusable() {
        let mut transcript = ClaudeToolTranscript::default();
        let mut retained = 0;
        for index in 0..1000 {
            retained += transcript
                .observe(&invocation(&format!("id-{index}"), "fill"))
                .len();
        }
        assert_eq!(retained, 64);
        assert!(transcript.take_truncation_notice());
        assert!(
            !transcript.take_truncation_notice(),
            "overflow notice must not flood a turn"
        );
        assert_eq!(transcript.observe(&result("id-0", json!("done"))).len(), 1);
        assert_eq!(transcript.observe(&invocation("new", "fill")).len(), 1);
        assert!(transcript
            .observe(&result("id-999", json!("untracked")))
            .is_empty());
        transcript.clear();
        assert!(!transcript.take_truncation_notice());
        assert_eq!(transcript.observe(&invocation("fresh", "fill")).len(), 1);
    }

    #[test]
    fn pending_bytes_are_bounded_even_before_the_entry_limit() {
        let mut transcript = ClaudeToolTranscript::default();
        let mut accepted = 0;
        for index in 0..64 {
            let mut event = invocation(&format!("id-{index}"), "fill");
            event["message"]["content"][0]["input"] = json!({"text":"x".repeat(7000)});
            accepted += transcript.observe(&event).len();
        }
        assert_eq!(accepted, 37);
        assert!(transcript.take_truncation_notice());
        for index in 0..accepted {
            assert_eq!(
                transcript
                    .observe(&result(&format!("id-{index}"), json!("done")))
                    .len(),
                1
            );
        }
        let mut refilled = 0;
        for index in 0..64 {
            let mut event = invocation(&format!("refill-{index}"), "fill");
            event["message"]["content"][0]["input"] = json!({"text":"x".repeat(7000)});
            refilled += transcript.observe(&event).len();
        }
        assert_eq!(refilled, 37, "completion must release the full byte budget");
    }

    #[test]
    fn identity_and_per_message_budgets_do_not_retain_or_emit_unbounded_batches() {
        let mut transcript = ClaudeToolTranscript::default();
        assert!(transcript
            .observe(&invocation(&"x".repeat(1_000_000), "fill"))
            .is_empty());
        assert!(transcript
            .observe(&invocation("id", &"x".repeat(1_000_000)))
            .is_empty());
        assert!(transcript.take_truncation_notice());
        transcript.clear();
        let blocks = (0..1000)
            .map(|index| {
                invocation(&format!("id-{index}"), "fill")["message"]["content"][0].clone()
            })
            .collect::<Vec<_>>();
        let emitted = transcript.observe(&json!({"type":"assistant","message":{"content":blocks}}));
        assert_eq!(emitted.len(), 64);
        assert!(transcript.take_truncation_notice());
        assert!(transcript
            .observe(&result("id-999", json!("untracked")))
            .is_empty());
    }

    #[test]
    fn exact_byte_limits_preserve_content_and_unicode_truncation_is_valid() {
        let mut transcript = ClaudeToolTranscript::default();
        let mut event = invocation("exact", "fill");
        event["message"]["content"][0]["input"] = json!("x".repeat(8190));
        assert_eq!(
            transcript.observe(&event)[0]["input"]
                .as_str()
                .unwrap()
                .len(),
            8190
        );
        let output = transcript.observe(&result("exact", json!("x".repeat(16384))));
        assert_eq!(output[0]["output"].as_str().unwrap().len(), 16384);
        assert!(!output[0]["output"].as_str().unwrap().contains("truncated"));
        transcript.observe(&invocation("blocks", "find"));
        let blocks = (0..1000)
            .map(|_| json!({"type":"image","data":"ignored"}))
            .collect::<Vec<_>>();
        let output = transcript.observe(&result("blocks", json!(blocks)));
        assert_eq!(output[0]["output"], TRUNCATED);
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
