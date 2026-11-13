use crate::provider_output_policy::output_bounds::shrink_strings_to_fit;
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
const RESULT_IMAGES: usize = 4;
const IMAGE_BASE64_BYTES: usize = 512 * 1024;
/// The largest picture file whose base64 form `bounded_result_images` keeps.
pub(crate) const RESULT_IMAGE_FILE_BYTES: usize = IMAGE_BASE64_BYTES / 4 * 3;
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
    if serde_json::to_writer(&mut encoded, input).is_ok() {
        return encoded.0;
    }
    // A file write carries its whole content: keep its path and the start of the content, which the row shows.
    let mut shrunk = input.clone();
    if let Value::Object(object) = &mut shrunk {
        object.insert("chariox_truncated".to_string(), Value::Bool(true));
    }
    if shrink_strings_to_fit(&mut shrunk, INPUT_BYTES, TRUNCATED) {
        if let Ok(bytes) = serde_json::to_vec(&shrunk) {
            return bytes;
        }
    }
    br#"{"chariox_truncated":true}"#.to_vec()
}

fn append_result(output: &mut String, text: &str) -> bool {
    let mut length = text.len().min(RESULT_BYTES.saturating_sub(output.len()));
    while !text.is_char_boundary(length) {
        length -= 1;
    }
    output.push_str(&text[..length]);
    length < text.len()
}

/// The result's PNG, JPEG, GIF and WebP blocks as data URLs, for the transcript to draw, and how many of those were too large or past the limit; other blocks are left out.
pub(crate) fn bounded_result_images(content: Option<&Value>) -> (Vec<String>, usize) {
    let Some(Value::Array(items)) = content else {
        return (Vec::new(), 0);
    };
    let mut images = Vec::new();
    let mut omitted = 0;
    for item in items {
        if item.get("type").and_then(Value::as_str) != Some("image") {
            continue;
        }
        let Some((media_type, data)) = (|| {
            let (media_type, data) = match item.get("source") {
                Some(source) => {
                    if source.get("type")?.as_str()? != "base64" {
                        return None;
                    }
                    (
                        source.get("media_type")?.as_str()?,
                        source.get("data")?.as_str()?,
                    )
                }
                // An MCP image block carries its type and data on the block itself.
                None => (item.get("mimeType")?.as_str()?, item.get("data")?.as_str()?),
            };
            let base64 = data
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'+' | b'/' | b'='));
            (matches!(
                media_type,
                "image/png" | "image/jpeg" | "image/gif" | "image/webp"
            ) && base64)
                .then_some((media_type, data))
        })() else {
            continue;
        };
        if data.len() > IMAGE_BASE64_BYTES || images.len() == RESULT_IMAGES {
            omitted += 1;
        } else {
            images.push(format!("data:{media_type};base64,{data}"));
        }
    }
    (images, omitted)
}

pub(crate) fn bounded_result(content: Option<&Value>) -> String {
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

// A hot reload hands a turn's pending tool calls, and the background
// commands still running from any turn, to the next kernel program.
#[derive(Default, serde::Serialize, serde::Deserialize)]
pub(super) struct ClaudeToolTranscript {
    pending: BTreeMap<String, PendingTool>,
    pending_bytes: usize,
    truncated: bool,
    truncation_reported: bool,
    unsupported_results: Vec<Value>,
    /// Launched background commands by tool call id: the call's last snapshot
    /// and the task id Claude names it by. They outlive the turn that started them.
    background: BTreeMap<String, BackgroundCall>,
    /// Background calls that ended outside `observe`, waiting to be emitted.
    ended: Vec<Value>,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct PendingTool {
    name: String,
    top_level: bool,
    // Retain bounded encoded input, not an expanded JSON tree per pending call.
    input: Vec<u8>,
    started_at_ms: u64,
    background: bool,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct BackgroundCall {
    task_id: Option<String>,
    snapshot: Value,
}

impl BackgroundCall {
    fn finished(mut self, finished_at_ms: u64) -> Value {
        self.snapshot["finished_at_ms"] = json!(finished_at_ms);
        self.snapshot
    }
}

impl PendingTool {
    fn bytes(&self, id: &str) -> usize {
        id.len() + self.name.len() + self.input.len()
    }

    fn input(&self) -> Value {
        serde_json::from_slice(&self.input).unwrap_or_else(|_| json!({"chariox_truncated": true}))
    }

    fn payload(&self, id: &str) -> Value {
        let mut payload = json!({
            "tool": self.name, "status": "running", "input": self.input(), "id": id,
            "started_at_ms": self.started_at_ms,
        });
        if self.background {
            payload["background"] = json!(true);
            payload["finished_at_ms"] = Value::Null;
        }
        payload
    }
}

/// Claude's Bash and Agent tools keep a command or a helper agent running after
/// their launch result when the model asked for `run_in_background`; its Monitor tool keeps
/// a script running and reports its events unless the model made it persistent, which ends
/// only with the session.
pub(crate) fn launches_background_task(name: &str, input: Option<&Value>) -> bool {
    let flag = |field: &str| {
        input
            .and_then(|input| input.get(field))
            .and_then(Value::as_bool)
            == Some(true)
    };
    match name {
        "Bash" | "Agent" | "Task" => flag("run_in_background"),
        "Monitor" => !flag("persistent"),
        _ => false,
    }
}

/// A helper agent Claude started in the background whatever the call asked for:
/// the launch result of the Agent tool says `async_launched`.
fn is_async_helper_launch(structured: Option<&Value>) -> bool {
    structured.is_some_and(|result| {
        result.get("status").and_then(Value::as_str) == Some("async_launched")
            && result.get("agentId").and_then(Value::as_str).is_some()
    })
}

/// The task id a background launch result names: the structured result's
/// `backgroundTaskId`, a Monitor's `taskId` or a helper agent's `agentId` (the stream's
/// `tool_use_result`, the transcript's `toolUseResult`), else the launch text
/// "Command running in background with ID: <id>. ...".
pub(crate) fn launched_task_id(structured: Option<&Value>, output: &str) -> Option<String> {
    structured
        .and_then(|result| {
            result
                .get("backgroundTaskId")
                .or_else(|| result.get("taskId"))
                .or_else(|| result.get("agentId"))
        })
        .and_then(Value::as_str)
        .map(str::to_string)
        .or_else(|| {
            let rest = output
                .split_once("running in background with ID: ")
                .or_else(|| output.split_once("agentId: "))?
                .1;
            let id = rest
                .split(|c: char| c.is_whitespace() || c == '.')
                .next()
                .filter(|id| !id.is_empty())?;
            Some(id.to_string())
        })
        .filter(|id| id.len() <= IDENTITY_BYTES)
}

/// The tools whose result can say a background command has ended.
pub(crate) fn ends_background_commands(name: &str) -> bool {
    matches!(name, "TaskStop" | "KillShell" | "TaskOutput" | "BashOutput")
}

/// The background task a stop or output-poll call is about, when its result
/// says that task has ended.
pub(crate) fn ended_task_of_result(name: &str, input: &Value, output: &str) -> Option<String> {
    let task_id = || {
        ["task_id", "shell_id", "bash_id"]
            .into_iter()
            .find_map(|field| input.get(field).and_then(Value::as_str))
            .map(str::to_string)
    };
    match name {
        "TaskStop" | "KillShell" => task_id(),
        "TaskOutput" | "BashOutput" => {
            let status = output.split_once("<status>")?.1.split_once("</status>")?.0;
            matches!(status.trim(), "completed" | "failed" | "killed" | "stopped")
                .then(task_id)
                .flatten()
        }
        _ => None,
    }
}

impl ClaudeToolTranscript {
    pub(super) fn plan_proposal_id(&self, requested: Option<&str>) -> Option<String> {
        let mut candidates = self.pending.iter().filter(|(id, tool)| {
            requested.is_none_or(|requested| requested == id.as_str())
                && tool.name == "ExitPlanMode"
                && tool.top_level
        });
        let (id, _) = candidates.next()?;
        candidates.next().is_none().then(|| id.clone())
    }

    pub(super) fn has_pending_tools(&self) -> bool {
        !self.pending.is_empty()
    }

    pub(super) fn observe(&mut self, value: &Value) -> Vec<Value> {
        self.observe_at(value, crate::session::unix_epoch_ms())
    }

    /// The snapshots `value` changes; `now_ms` is when the kernel saw it.
    fn observe_at(&mut self, value: &Value, now_ms: u64) -> Vec<Value> {
        self.unsupported_results.clear();
        let kind = value.get("type").and_then(Value::as_str);
        if let Some(end) = super::task_notification::finished_background_task(value) {
            return self.finish_background_task(end.tool_use_id, end.task_id, now_ms);
        }
        let message = value.get("message").unwrap_or(value);
        let Some(content) = message.get("content").and_then(Value::as_array) else {
            return Vec::new();
        };
        self.truncated |= content.len() > MESSAGE_BLOCKS;
        let mut ended_background = Vec::new();
        let mut payloads: Vec<Value> = content
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
                    // Current Claude reads its plan file itself and returns the
                    // text here, rather than sending the old inline plan input.
                    if !failed && pending.name == "ExitPlanMode" {
                        if let Some(input) = payload["input"].as_object_mut() {
                            if !input.get("plan").and_then(Value::as_str).is_some_and(|plan| !plan.trim().is_empty()) {
                                if let Some(plan) = approved_plan(&output) {
                                    input.insert("plan".to_string(), json!(plan));
                                }
                            }
                        }
                    }
                    // A plan card consumes input.plan. Keeping the native result's
                    // copy as well can exceed the terminal record's combined limit.
                    let has_plan = pending.name == "ExitPlanMode"
                        && payload["input"]["plan"].as_str().is_some_and(|plan| !plan.trim().is_empty());
                    if failed || !has_plan {
                        payload[if failed { "error" } else { "output" }] = json!(output);
                    }
                    let (images, omitted) = bounded_result_images(block.get("content"));
                    if !images.is_empty() {
                        payload["images"] = json!(images);
                    }
                    if omitted > 0 {
                        payload["images_omitted"] = json!(omitted);
                    }
                    let background = pending.background
                        || (!failed && is_async_helper_launch(value.get("tool_use_result")));
                    if background && !pending.background {
                        payload["background"] = json!(true);
                        payload["finished_at_ms"] = Value::Null;
                    }
                    if background {
                        if failed {
                            // A launch that failed never left a command running.
                            payload["finished_at_ms"] = json!(now_ms);
                        } else if self.background.len() >= PENDING_ENTRIES {
                            // Untracked, it ends with its provider session.
                            self.truncated = true;
                        } else {
                            self.background.insert(
                                id.to_string(),
                                BackgroundCall {
                                    task_id: launched_task_id(value.get("tool_use_result"), &output),
                                    snapshot: payload.clone(),
                                },
                            );
                        }
                    }
                    if !failed {
                        if let Some(task_id) = ended_task_of_result(&pending.name, &pending.input(), &output) {
                            ended_background.push(task_id);
                        }
                    }
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
                let parent = value.get("parent_tool_use_id").and_then(Value::as_str);
                let top_level = parent.is_none();
                // A helper this agent started in the background has its own background
                // commands reported back here, so they are held for like this agent's own. A
                // foreground helper's commands end with that helper and are reported to it only.
                let reported_here = parent.is_none_or(|parent| self.background.contains_key(parent));
                let pending = PendingTool {
                    name: name.to_string(),
                    top_level,
                    input: bounded_input(block.get("input").unwrap_or(&Value::Null)),
                    started_at_ms: now_ms,
                    background: reported_here
                        && launches_background_task(name, block.get("input")),
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
            .collect();
        for task_id in ended_background {
            payloads.extend(self.finish_background_task(None, Some(&task_id), now_ms));
        }
        payloads
    }

    /// Ends the background call Claude reported by tool call id or task id.
    fn finish_background_task(
        &mut self,
        tool_use_id: Option<&str>,
        task_id: Option<&str>,
        now_ms: u64,
    ) -> Vec<Value> {
        let id = tool_use_id
            .filter(|id| self.background.contains_key(*id))
            .map(str::to_string)
            .or_else(|| {
                let task_id = task_id?;
                self.background
                    .iter()
                    .find(|(_, call)| call.task_id.as_deref() == Some(task_id))
                    .map(|(id, _)| id.clone())
            });
        id.and_then(|id| self.background.remove(&id))
            .map(|call| call.finished(now_ms))
            .into_iter()
            .collect()
    }

    /// The Claude process that ran the background commands is gone: each one
    /// still open ended no later than now.
    pub(super) fn end_background_calls(&mut self, now_ms: u64) {
        let background = std::mem::take(&mut self.background);
        self.ended
            .extend(background.into_values().map(|call| call.finished(now_ms)));
    }

    pub(super) fn take_ended_background_calls(&mut self) -> Vec<Value> {
        std::mem::take(&mut self.ended)
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

fn approved_plan(output: &str) -> Option<&str> {
    [
        "\n\n## Approved Plan:\n",
        "\n\n## Approved Plan (edited by user):\n",
    ]
    .into_iter()
    .find_map(|marker| output.split_once(marker).map(|(_, plan)| plan.trim()))
    .filter(|plan| !plan.is_empty())
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

    // Recorded from Claude Code 2.1.296 (stream-json message shapes and the
    // CLI's own `system/task_notification` schema); ids kept as recorded.
    fn background_launch(id: &str, command: &str) -> Value {
        json!({"type":"assistant","message":{"role":"assistant","content":[{
            "type":"tool_use","id":id,"name":"Bash",
            "input":{"command":command,"description":"Real background command","run_in_background":true}
        }]}})
    }

    fn background_launch_result(id: &str, task_id: &str) -> Value {
        json!({"type":"user","message":{"role":"user","content":[{
            "tool_use_id":id,"type":"tool_result","is_error":false,
            "content":format!("Command running in background with ID: {task_id}. Output is being written to: /tmp/claude-0/tasks/{task_id}.output. You will be notified when it completes. To check interim output, use Read on that file path.")
        }]},"tool_use_result":{"stdout":"","stderr":"","interrupted":false,"isImage":false,"noOutputExpected":false,"backgroundTaskId":task_id}})
    }

    fn task_notification(task_id: &str, tool_use_id: &str, status: &str) -> Value {
        json!({"type":"system","subtype":"task_notification","task_id":task_id,
            "tool_use_id":tool_use_id,"status":status,
            "output_file":format!("/tmp/claude-0/tasks/{task_id}.output"),
            "summary":"Background command \"Real background command\" completed (exit code 0)",
            "session_id":"session-1","uuid":"uuid-1"})
    }

    #[test]
    fn kwait_interim_task_notification_does_not_end_background_work() {
        let mut transcript = ClaudeToolTranscript::default();
        transcript.observe_at(&background_launch("bg", "sleep 20"), 1_000);
        transcript.observe_at(&background_launch_result("bg", "task"), 1_100);
        assert!(transcript
            .observe_at(&task_notification("task", "bg", "running"), 2_000)
            .is_empty());
        assert_eq!(
            transcript
                .observe_at(&task_notification("task", "bg", "completed"), 3_000)
                .len(),
            1
        );
    }

    #[test]
    fn kwait_nested_background_command_ends_on_replayed_task_notification() {
        for wrapped in [false, true] {
            let mut transcript = ClaudeToolTranscript::default();
            let mut agent = background_launch("agent-call", "ignored");
            agent["message"]["content"][0]["name"] = json!("Agent");
            transcript.observe_at(&agent, 1_000);
            let mut launched = background_launch_result("agent-call", "agent-task");
            launched["tool_use_result"] = json!({"agentId":"agent-task","isAsync":true});
            transcript.observe_at(&launched, 1_100);
            let mut nested = background_launch("nested-bash", "sleep 45; echo done");
            nested["parent_tool_use_id"] = json!("agent-call");
            transcript.observe_at(&nested, 1_200);
            let mut nested_result = background_launch_result("nested-bash", "bash-task");
            nested_result["parent_tool_use_id"] = json!("agent-call");
            transcript.observe_at(&nested_result, 1_300);
            // Claude reports the Agent's interim answer before its background Bash ends.
            transcript.observe_at(
                &task_notification("agent-task", "agent-call", "completed"),
                1_400,
            );
            let notification = "<task-notification><task-id>bash-task</task-id><tool-use-id>nested-bash</tool-use-id><status>completed</status></task-notification>";
            let text = if wrapped {
                format!("[SYSTEM NOTIFICATION - NOT USER INPUT]\nThis is an automated background-task event.\n{notification}")
            } else {
                notification.to_string()
            };
            let content = if wrapped {
                json!([{"type":"text","text":text}])
            } else {
                json!(text)
            };
            let ended = transcript.observe_at(&json!({"type":"user", "parent_tool_use_id":"agent-call", "message":{"role":"user","content":content}}), 2_000);
            assert_eq!(
                ended.len(),
                1,
                "wrapped={wrapped}: the nested command must not stay open"
            );
            assert_eq!(ended[0]["id"], "nested-bash");
            assert_eq!(ended[0]["finished_at_ms"], 2_000);
            assert!(transcript.background.is_empty());
        }
    }

    fn task_stop(id: &str, task_id: &str) -> [Value; 2] {
        [
            json!({"type":"assistant","message":{"role":"assistant","content":[{
                "type":"tool_use","id":id,"name":"TaskStop","input":{"task_id":task_id}
            }]}}),
            json!({"type":"user","message":{"role":"user","content":[{
                "tool_use_id":id,"type":"tool_result",
                "content":format!("{{\"message\":\"Successfully stopped task: {task_id} (sleep 300)\",\"task_id\":\"{task_id}\",\"task_type\":\"local_bash\",\"command\":\"sleep 300\"}}")
            }]}}),
        ]
    }

    #[test]
    fn foreground_calls_carry_their_start_time_and_no_background_fields() {
        let mut transcript = ClaudeToolTranscript::default();
        let started = transcript.observe_at(&invocation("call-1", "Read"), 1_000);
        assert_eq!(started[0]["started_at_ms"], 1_000);
        let completed = transcript.observe_at(&result("call-1", json!("text")), 2_000);
        assert_eq!(completed[0]["status"], "completed");
        assert_eq!(completed[0]["started_at_ms"], 1_000);
        for payload in started.iter().chain(&completed) {
            assert!(payload.get("background").is_none(), "{payload}");
            assert!(payload.get("finished_at_ms").is_none(), "{payload}");
        }
    }

    #[test]
    fn background_command_runs_past_its_launch_and_its_turn_until_claude_reports_its_end() {
        let mut transcript = ClaudeToolTranscript::default();
        let id = "toolu_01GFMN69WJrZfKDN6V2aTEy1";
        let launch =
            transcript.observe_at(&background_launch(id, "sleep 20; echo first-done"), 1_000);
        assert_eq!(launch[0]["background"], true);
        assert_eq!(launch[0]["started_at_ms"], 1_000);
        assert!(launch[0]["finished_at_ms"].is_null());
        let launched = transcript.observe_at(&background_launch_result(id, "br1ml3bl1"), 1_200);
        assert_eq!(launched[0]["status"], "completed");
        assert_eq!(launched[0]["background"], true);
        assert_eq!(launched[0]["started_at_ms"], 1_000);
        assert!(launched[0]["finished_at_ms"].is_null(), "{}", launched[0]);
        // The turn settles and a newer one starts; the command keeps running.
        transcript.clear();
        transcript.observe_at(&invocation("later", "Read"), 5_000);
        let ended = transcript.observe_at(&task_notification("br1ml3bl1", id, "completed"), 21_000);
        assert_eq!(ended.len(), 1);
        assert_eq!(ended[0]["id"], id);
        assert_eq!(ended[0]["background"], true);
        assert_eq!(ended[0]["started_at_ms"], 1_000);
        assert_eq!(ended[0]["finished_at_ms"], 21_000);
        assert_eq!(ended[0]["output"], launched[0]["output"]);
        assert!(transcript
            .observe_at(&task_notification("br1ml3bl1", id, "completed"), 22_000)
            .is_empty());
    }

    #[test]
    fn two_background_commands_end_independently_and_a_stopped_one_ends_at_its_stop() {
        let mut transcript = ClaudeToolTranscript::default();
        let (first, second) = (
            "toolu_01GFMN69WJrZfKDN6V2aTEy1",
            "toolu_01BX35fq1ULEMKJyPvA6kaF7",
        );
        transcript.observe_at(
            &background_launch(first, "sleep 20; echo first-done"),
            1_000,
        );
        transcript.observe_at(&background_launch_result(first, "br1ml3bl1"), 1_100);
        transcript.observe_at(&background_launch(second, "sleep 300"), 1_200);
        transcript.observe_at(&background_launch_result(second, "bmyee9bi9"), 1_300);
        let [stop, stopped] = task_stop("toolu_01Lkf9ksDXjV2UaE7kwgjT5f", "bmyee9bi9");
        transcript.observe_at(&stop, 2_000);
        let settled = transcript.observe_at(&stopped, 2_100);
        assert_eq!(settled.len(), 2, "{settled:?}");
        assert_eq!(settled[0]["tool"], "TaskStop");
        assert!(settled[0].get("background").is_none());
        assert_eq!(settled[1]["id"], second);
        assert_eq!(settled[1]["started_at_ms"], 1_200);
        assert_eq!(settled[1]["finished_at_ms"], 2_100);
        // Claude's stop notice for the same task changes nothing more.
        assert!(transcript
            .observe_at(&task_notification("bmyee9bi9", second, "stopped"), 2_200)
            .is_empty());
        let first_ended =
            transcript.observe_at(&task_notification("br1ml3bl1", first, "completed"), 21_000);
        assert_eq!(first_ended[0]["id"], first);
        assert_eq!(first_ended[0]["finished_at_ms"], 21_000);
    }

    // Recorded from Claude Code: the Monitor tool keeps a script running past its launch and
    // reports each event, then the end of the stream, as a task notification.
    fn monitor_launch(id: &str, persistent: bool) -> Value {
        let mut input = json!({"command":"sleep 40; echo MONITOR_EVENT_FIRED","description":"drill monitor","timeout_ms":300000});
        if persistent {
            input["persistent"] = json!(true);
        }
        json!({"type":"assistant","message":{"role":"assistant","content":[{
            "type":"tool_use","id":id,"name":"Monitor","input":input
        }]}})
    }

    fn monitor_launch_result(id: &str, task_id: &str) -> Value {
        json!({"type":"user","message":{"role":"user","content":[{
            "tool_use_id":id,"type":"tool_result",
            "content":format!("Monitor started (task {task_id}, expires in 5m unless the source ends first; you get one notice at expiry \u{2014} re-arm if you still need the watch). You will be notified on each event. Keep working \u{2014} do not poll or sleep.")
        }]},"tool_use_result":{"taskId":task_id,"timeoutMs":300000,"persistent":false}})
    }

    #[test]
    fn a_monitor_runs_past_its_launch_and_its_turn_until_its_stream_ends() {
        let mut transcript = ClaudeToolTranscript::default();
        let id = "toolu_01KamQyLbeQDEZ46fKLMXvGg";
        let launch = transcript.observe_at(&monitor_launch(id, false), 1_000);
        assert_eq!(launch[0]["background"], true, "{}", launch[0]);
        let launched = transcript.observe_at(&monitor_launch_result(id, "br087zy1c"), 1_200);
        assert_eq!(launched[0]["background"], true, "{}", launched[0]);
        assert!(launched[0]["finished_at_ms"].is_null(), "{}", launched[0]);
        let ended = transcript.observe_at(&task_notification("br087zy1c", id, "completed"), 41_000);
        assert_eq!(ended.len(), 1, "{ended:?}");
        assert_eq!(ended[0]["id"], id);
        assert_eq!(ended[0]["finished_at_ms"], 41_000);
    }

    #[test]
    fn a_persistent_monitor_is_not_held_for_because_it_never_ends() {
        let mut transcript = ClaudeToolTranscript::default();
        let launch = transcript.observe_at(&monitor_launch("persistent", true), 1_000);
        assert!(launch[0].get("background").is_none(), "{}", launch[0]);
    }

    // Recorded from Claude Code (Agent tool with `run_in_background`): the launch result is
    // `async_launched` with the helper's `agentId`, and the end arrives as a task notification.
    fn helper_launch(id: &str, background_flag: bool) -> Value {
        let mut input = json!({"description":"Map the display lifecycle","subagent_type":"Explore","prompt":"Read only"});
        if background_flag {
            input["run_in_background"] = json!(true);
        }
        json!({"type":"assistant","message":{"role":"assistant","content":[{
            "type":"tool_use","id":id,"name":"Agent","input":input
        }]}})
    }

    fn helper_launch_result(id: &str, agent_id: &str) -> Value {
        json!({"type":"user","message":{"role":"user","content":[{
            "tool_use_id":id,"type":"tool_result",
            "content":[{"type":"text","text":format!("Async agent launched successfully.\nagentId: {agent_id} (internal ID)\nThe agent is working in the background. You will be notified automatically when it completes.")}]
        }]},"tool_use_result":{"isAsync":true,"status":"async_launched","agentId":agent_id,
            "description":"Map the display lifecycle","outputFile":format!("/tmp/claude-0/tasks/{agent_id}.output")}})
    }

    #[test]
    fn a_background_helper_agent_runs_past_its_launch_until_its_task_notification() {
        let mut transcript = ClaudeToolTranscript::default();
        let id = "toolu_01T7cHLskcwCF9XiXUh2xNCy";
        let launch = transcript.observe_at(&helper_launch(id, true), 1_000);
        assert_eq!(launch[0]["background"], true, "{}", launch[0]);
        let launched = transcript.observe_at(&helper_launch_result(id, "a578456522a670c66"), 1_200);
        assert_eq!(launched[0]["status"], "completed");
        assert_eq!(launched[0]["background"], true);
        assert!(launched[0]["finished_at_ms"].is_null(), "{}", launched[0]);
        let ended = transcript.observe_at(
            &task_notification("a578456522a670c66", id, "completed"),
            9_000,
        );
        assert_eq!(ended.len(), 1, "{ended:?}");
        assert_eq!(ended[0]["id"], id);
        assert_eq!(ended[0]["finished_at_ms"], 9_000);
    }

    #[test]
    fn a_helper_agent_the_provider_launched_asynchronously_is_a_background_call_without_the_flag() {
        let mut transcript = ClaudeToolTranscript::default();
        let id = "toolu_auto";
        transcript.observe_at(&helper_launch(id, false), 1_000);
        let launched = transcript.observe_at(&helper_launch_result(id, "aauto1"), 1_200);
        assert_eq!(launched[0]["background"], true, "{}", launched[0]);
        assert!(launched[0]["finished_at_ms"].is_null(), "{}", launched[0]);
        let ended =
            transcript.observe_at(&task_notification("aauto1", "other", "completed"), 3_000);
        assert_eq!(ended[0]["id"], id);
        assert_eq!(ended[0]["finished_at_ms"], 3_000);
    }

    // Live (l10orch r1): a helper's own background `sleep` left open held the agent for the whole
    // hold limit. A foreground helper's commands are reported to that helper only; a background
    // helper's are replayed here (kwait_nested_background_command_ends_on_replayed_task_notification).
    #[test]
    fn a_background_command_of_a_foreground_helper_agent_is_not_held_for() {
        let mut transcript = ClaudeToolTranscript::default();
        let mut launch = background_launch("nested", "sleep 40");
        launch["parent_tool_use_id"] = json!("toolu_helper");
        let started = transcript.observe_at(&launch, 1_000);
        assert!(started[0].get("background").is_none(), "{}", started[0]);
        let mut result = background_launch_result("nested", "bnested1");
        result["parent_tool_use_id"] = json!("toolu_helper");
        let launched = transcript.observe_at(&result, 1_200);
        assert!(launched[0].get("background").is_none(), "{}", launched[0]);
        assert!(
            launched[0].get("finished_at_ms").is_none(),
            "{}",
            launched[0]
        );
    }

    #[test]
    fn an_output_poll_that_reports_the_end_settles_the_background_call() {
        let mut transcript = ClaudeToolTranscript::default();
        let id = "toolu_bg";
        transcript.observe_at(&background_launch(id, "make test"), 1_000);
        transcript.observe_at(&background_launch_result(id, "bash_1"), 1_100);
        transcript.observe_at(
            &json!({"type":"assistant","message":{"content":[{
                "type":"tool_use","id":"poll-1","name":"BashOutput","input":{"bash_id":"bash_1"}
            }]}}),
            2_000,
        );
        let running = transcript.observe_at(
            &result(
                "poll-1",
                json!("<status>running</status>\n\n<stdout>\nbuilding\n</stdout>"),
            ),
            2_100,
        );
        assert_eq!(running.len(), 1);
        transcript.observe_at(
            &json!({"type":"assistant","message":{"content":[{
                "type":"tool_use","id":"poll-2","name":"BashOutput","input":{"bash_id":"bash_1"}
            }]}}),
            3_000,
        );
        let done = transcript.observe_at(
            &result(
                "poll-2",
                json!("<status>completed</status>\n\n<exit_code>0</exit_code>"),
            ),
            3_100,
        );
        assert_eq!(done.len(), 2);
        assert_eq!(done[1]["id"], id);
        assert_eq!(done[1]["finished_at_ms"], 3_100);
    }

    #[test]
    fn a_failed_launch_ends_at_once_and_a_lost_process_ends_every_open_background_call() {
        let mut transcript = ClaudeToolTranscript::default();
        transcript.observe_at(&background_launch("failed", "sleep 1"), 1_000);
        let mut failure = result("failed", json!("permission denied"));
        failure["message"]["content"][0]["is_error"] = json!(true);
        let failed = transcript.observe_at(&failure, 1_100);
        assert_eq!(failed[0]["status"], "error");
        assert_eq!(failed[0]["finished_at_ms"], 1_100);

        // A launch result that names no task is still settled by its call id.
        transcript.observe_at(&background_launch("unnamed", "sleep 5"), 1_500);
        let unnamed = transcript.observe_at(&result("unnamed", json!("started")), 1_600);
        assert!(unnamed[0]["finished_at_ms"].is_null());
        let unnamed =
            transcript.observe_at(&task_notification("t9", "unnamed", "completed"), 1_700);
        assert_eq!(unnamed[0]["finished_at_ms"], 1_700);

        transcript.observe_at(&background_launch("running", "sleep 40"), 2_000);
        transcript.observe_at(&background_launch_result("running", "b1"), 2_100);
        // A hot reload carries the running call to the next kernel program.
        let mut transcript: ClaudeToolTranscript =
            serde_json::from_value(serde_json::to_value(&transcript).unwrap()).unwrap();
        assert!(transcript.take_ended_background_calls().is_empty());
        transcript.end_background_calls(9_000);
        let ended = transcript.take_ended_background_calls();
        assert_eq!(ended.len(), 1);
        assert_eq!(ended[0]["id"], "running");
        assert_eq!(ended[0]["finished_at_ms"], 9_000);
        assert!(transcript
            .observe_at(&task_notification("b1", "running", "completed"), 9_500)
            .is_empty());
    }

    #[test]
    fn plan_proposal_matches_only_its_pending_root_invocation() {
        let mut transcript = ClaudeToolTranscript::default();
        let root = invocation("root", "ExitPlanMode");
        let mut nested = invocation("nested", "ExitPlanMode");
        nested["parent_tool_use_id"] = json!("subagent");
        transcript.observe(&root);
        transcript.observe(&nested);
        transcript.observe(&invocation("write", "Write"));
        assert_eq!(
            transcript.plan_proposal_id(Some("root")).as_deref(),
            Some("root")
        );
        assert_eq!(transcript.plan_proposal_id(None).as_deref(), Some("root"));
        assert!(transcript.plan_proposal_id(Some("nested")).is_none());
        assert!(transcript.plan_proposal_id(Some("write")).is_none());
        assert!(transcript.plan_proposal_id(Some("ended")).is_none());
        transcript.observe(&invocation("another-root", "ExitPlanMode"));
        assert!(transcript.plan_proposal_id(None).is_none());
        transcript.clear();
        assert!(transcript.plan_proposal_id(Some("root")).is_none());
    }

    #[test]
    fn native_plan_file_result_supplies_the_existing_plan_card_input() {
        let mut transcript = ClaudeToolTranscript::default();
        let mut call = invocation("plan", "ExitPlanMode");
        call["message"]["content"][0]["input"] = json!({});
        transcript.observe(&call);
        let completed = transcript.observe(&result("plan", json!(
            "User has approved your plan.\n\nYour plan has been saved to: /plan.md\n\n## Approved Plan:\n1. Implement the change.\n2. Test it."
        )));
        assert_eq!(completed[0]["status"], "completed");
        assert_eq!(
            completed[0]["input"]["plan"],
            "1. Implement the change.\n2. Test it."
        );
    }

    #[test]
    fn failed_or_unrelated_results_do_not_become_native_plan_cards() {
        for (tool, failed) in [("ExitPlanMode", true), ("Bash", false)] {
            let mut transcript = ClaudeToolTranscript::default();
            let mut call = invocation("plan", tool);
            call["message"]["content"][0]["input"] = json!({});
            transcript.observe(&call);
            let mut reply = result(
                "plan",
                json!("Denied.\n\n## Approved Plan:\nDo not publish this."),
            );
            reply["message"]["content"][0]["is_error"] = json!(failed);
            let completed = transcript.observe(&reply);
            assert!(completed[0]["input"].get("plan").is_none());
        }
    }

    #[test]
    fn edited_native_plan_is_extracted_and_inline_plan_is_preserved() {
        for input in [json!({}), json!({"plan":"Inline plan"})] {
            let mut transcript = ClaudeToolTranscript::default();
            let mut call = invocation("plan", "ExitPlanMode");
            call["message"]["content"][0]["input"] = input.clone();
            transcript.observe(&call);
            let completed = transcript.observe(&result("plan", json!([
                {"type":"text","text":"Approved.\n\n## Approved Plan (edited by user):\nEdited plan"}
            ])));
            assert_eq!(
                completed[0]["input"]["plan"],
                input.get("plan").cloned().unwrap_or(json!("Edited plan"))
            );
        }
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
    fn oversized_file_write_keeps_its_path_and_the_start_of_its_content() {
        let mut transcript = ClaudeToolTranscript::default();
        let mut event = invocation("write", "Write");
        let page = format!("<!doctype html>\n{}", "<p>row</p>\n".repeat(3000));
        event["message"]["content"][0]["input"] =
            json!({"file_path": "/w/chart.html", "content": page});
        let started = transcript.observe(&event);
        assert_eq!(started[0]["input"]["file_path"], "/w/chart.html");
        assert!(started[0]["input"]["content"]
            .as_str()
            .unwrap()
            .starts_with("<!doctype html>\n<p>row</p>"));
        assert_eq!(started[0]["input"]["chariox_truncated"], true);
        let finished = transcript.observe(&result("write", json!("File created")));
        assert_eq!(finished[0]["input"]["file_path"], "/w/chart.html");
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
    fn result_images_become_data_urls_and_oversized_ones_are_counted_and_unknown_ones_left_out() {
        let mut transcript = ClaudeToolTranscript::default();
        transcript.observe(&invocation("shot", "Read"));
        let png = json!({"type":"image","source":{"type":"base64","media_type":"image/png","data":"iVBORw0KGgo="}});
        let svg = json!({"type":"image","source":{"type":"base64","media_type":"image/svg+xml","data":"PHN2Zy8+"}});
        let huge = json!({"type":"image","source":{"type":"base64","media_type":"image/png","data":"A".repeat(IMAGE_BASE64_BYTES + 1)}});
        let url = json!({"type":"image","source":{"type":"url","url":"https://example.com/a.png"}});
        let output = transcript.observe(&result(
            "shot",
            json!([{"type":"text","text":"caption"}, png, svg, huge, url]),
        ));
        assert_eq!(output[0]["output"], "caption");
        assert_eq!(
            output[0]["images"],
            json!(["data:image/png;base64,iVBORw0KGgo="])
        );
        assert_eq!(output[0]["images_omitted"], 1);
        transcript.observe(&invocation("plain", "Read"));
        let plain = transcript.observe(&result("plain", json!("text only")));
        assert!(plain[0].get("images").is_none());
    }

    #[test]
    fn result_images_match_the_protocol_snapshot() {
        let fixture: Value = serde_json::from_str(include_str!(
            "../../local/api/tests/protocol_shapes/provider-tool-images-910.json"
        ))
        .unwrap();
        let mut transcript = ClaudeToolTranscript::default();
        let mut call = invocation("toolu_chart", "Read");
        call["message"]["content"][0]["input"] = json!({"file_path": "/work/chart.png"});
        transcript.observe_at(&call, 1_000);
        let png = json!({"type":"image","source":{"type":"base64","media_type":"image/png","data":"iVBORw0KGgo="}});
        let output = transcript.observe(&result(
            "toolu_chart",
            json!([{"type":"text","text":"chart.png"}, png]),
        ));
        let mut expected = fixture["tool_with_image"].clone();
        expected["started_at_ms"] = json!(1_000);
        assert_eq!(output[0], expected);

        let mut call = invocation("toolu_huge", "Read");
        call["message"]["content"][0]["input"] = json!({"file_path": "/work/huge.png"});
        transcript.observe_at(&call, 2_000);
        let huge = json!({"type":"image","source":{"type":"base64","media_type":"image/png","data":"A".repeat(IMAGE_BASE64_BYTES + 1)}});
        let output = transcript.observe(&result(
            "toolu_huge",
            json!([{"type":"text","text":"huge.png"}, huge]),
        ));
        let mut expected = fixture["tool_with_omitted_image"].clone();
        expected["started_at_ms"] = json!(2_000);
        assert_eq!(output[0], expected);
    }

    #[test]
    fn background_lifecycle_matches_the_protocol_snapshot() {
        let fixture: Value = serde_json::from_str(include_str!(
            "../../local/api/tests/protocol_shapes/provider-tool-background-lifecycle.json"
        ))
        .unwrap();
        let start = 1_793_700_000_000;
        let mut transcript = ClaudeToolTranscript::default();
        let mut read = invocation("toolu_fg", "Read");
        read["message"]["content"][0]["input"] = json!({"file_path": "/work/notes.txt"});
        transcript.observe_at(&read, start);
        let foreground = transcript.observe_at(&result("toolu_fg", json!("notes")), start + 5);
        assert_eq!(foreground[0], fixture["claude_foreground_completed"]);

        let mut launch = background_launch("toolu_bg", "sleep 40");
        launch["message"]["content"][0]["input"]["description"] = json!("Sleep in the background");
        transcript.observe_at(&launch, start);
        let mut launched = background_launch_result("toolu_bg", "b1");
        launched.as_object_mut().unwrap().remove("tool_use_result");
        let launched = transcript.observe_at(&launched, start + 100);
        assert_eq!(launched[0], fixture["claude_background_launched"]);
        let finished = transcript.observe_at(
            &task_notification("b1", "toolu_bg", "completed"),
            start + 40_000,
        );
        assert_eq!(finished[0], fixture["claude_background_finished"]);
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
