use std::collections::BTreeMap;
use std::sync::{Mutex, OnceLock};

use crate::terminal::TerminalOutputKind;
use serde_json::Value;

pub(crate) const MAX_PROVIDER_OUTPUT_RECORD_BYTES: usize = 16 * 1024;
const MAX_PROVIDER_TOOL_STRING_BYTES: usize = 12 * 1024;
const PROVIDER_TOOL_DELTA_GATE_LIMIT: usize = 4096;
const PROVIDER_TOOL_DELTA_TAIL_SAMPLE_BYTES: usize = 256;
const PROVIDER_TOOL_LARGE_PAYLOAD_BYTES: usize = 512 * 1024;
const PROVIDER_TOOL_LARGE_PAYLOAD_REPORT_BYTES: usize = 1024 * 1024;
const PROVIDER_OUTPUT_TRUNCATION_LOG_DELTA_BYTES: usize = 1024 * 1024;
const PROVIDER_TOOL_METADATA_HASH_STRING_BYTES: usize = 4096;

pub(super) fn bounded_terminal_output_bytes(kind: &TerminalOutputKind, bytes: &[u8]) -> Vec<u8> {
    if bytes.len() <= MAX_PROVIDER_OUTPUT_RECORD_BYTES {
        return bytes.to_vec();
    }
    if matches!(kind, TerminalOutputKind::ProviderTool) {
        if let Some(value) = bounded_provider_tool_json(bytes) {
            return value;
        }
    }
    bounded_text_bytes(bytes)
}

pub(super) fn terminal_output_delta_bytes(
    session_id: &str,
    provider_run_id: &str,
    agent_id: Option<&str>,
    kind: &TerminalOutputKind,
    merge_key: &Option<String>,
    bytes: &[u8],
) -> Vec<u8> {
    if !matches!(kind, TerminalOutputKind::ProviderTool) {
        return bytes.to_vec();
    }
    if bytes.len() > PROVIDER_TOOL_LARGE_PAYLOAD_BYTES {
        if let Some(delta) =
            large_provider_tool_delta_bytes(session_id, provider_run_id, agent_id, merge_key, bytes)
        {
            return delta;
        }
    }
    provider_tool_delta_bytes(session_id, provider_run_id, agent_id, merge_key, bytes)
        .unwrap_or_else(|| bytes.to_vec())
}

fn large_provider_tool_delta_bytes(
    session_id: &str,
    provider_run_id: &str,
    agent_id: Option<&str>,
    merge_key: &Option<String>,
    bytes: &[u8],
) -> Option<Vec<u8>> {
    let tool_id = merge_key.clone()?;
    let key = ProviderToolDeltaKey {
        session_id: session_id.to_string(),
        provider_run_id: provider_run_id.to_string(),
        agent_id: agent_id.unwrap_or("").to_string(),
        tool_id: tool_id.clone(),
        field: "__raw_provider_tool_payload__".to_string(),
    };
    let mut gate = provider_tool_delta_gate()
        .lock()
        .expect("provider tool delta gate lock should not be poisoned");
    let previous = gate.get(&key).cloned();
    let next_state = ProviderToolDeltaState::from_bytes(bytes);
    let should_emit = previous
        .as_ref()
        .map(|previous| {
            bytes.len() < previous.byte_len
                || bytes.len().saturating_sub(previous.byte_len)
                    >= PROVIDER_TOOL_LARGE_PAYLOAD_REPORT_BYTES
                || !provider_tool_output_is_append(previous, bytes)
        })
        .unwrap_or(true);
    gate.insert(key, next_state);
    trim_provider_tool_delta_gate(&mut gate);
    drop(gate);

    if !should_emit {
        return Some(Vec::new());
    }

    let delta_bytes = previous
        .as_ref()
        .filter(|previous| bytes.len() >= previous.byte_len)
        .map(|previous| bytes.len().saturating_sub(previous.byte_len))
        .unwrap_or(bytes.len());
    serde_json::to_vec(&serde_json::json!({
        "id": tool_id,
        "status": "running",
        "arroba_truncated": true,
        "arroba_original_bytes": bytes.len(),
        "arroba_delta_bytes": delta_bytes,
        "message": format!(
            "provider tool output exceeded the live terminal cap; suppressed {} bytes of cumulative output",
            bytes.len()
        ),
    }))
    .ok()
}

fn provider_tool_delta_bytes(
    session_id: &str,
    provider_run_id: &str,
    agent_id: Option<&str>,
    merge_key: &Option<String>,
    bytes: &[u8],
) -> Option<Vec<u8>> {
    let mut value = serde_json::from_slice::<Value>(bytes).ok()?;
    let Value::Object(object) = &mut value else {
        return None;
    };
    let tool_id = merge_key
        .clone()
        .or_else(|| json_string_field(object, "id"))
        .or_else(|| json_string_field(object, "call_id"))?;
    let Some(field) = provider_tool_output_field(object) else {
        return serde_json::to_vec(&value).ok();
    };
    let Some(Value::String(text)) = object.get(&field) else {
        return serde_json::to_vec(&value).ok();
    };
    let current_bytes = text.as_bytes();
    let current_len = current_bytes.len();
    let key = ProviderToolDeltaKey {
        session_id: session_id.to_string(),
        provider_run_id: provider_run_id.to_string(),
        agent_id: agent_id.unwrap_or("").to_string(),
        tool_id,
        field: field.clone(),
    };
    let next_state = ProviderToolDeltaState::from_parts(
        current_bytes,
        provider_tool_metadata_fingerprint(object, &field),
    );
    let mut gate = provider_tool_delta_gate()
        .lock()
        .expect("provider tool delta gate lock should not be poisoned");
    let previous = gate.get(&key).cloned();
    let mut output_text = None;
    if let Some(previous) = previous {
        if provider_tool_output_is_append(&previous, current_bytes) {
            if previous.byte_len == current_len
                && previous.metadata_fingerprint == next_state.metadata_fingerprint
            {
                gate.insert(key, next_state);
                trim_provider_tool_delta_gate(&mut gate);
                return Some(Vec::new());
            }
            if previous.byte_len < current_len {
                output_text =
                    Some(String::from_utf8_lossy(&current_bytes[previous.byte_len..]).into_owned());
                object.insert("arroba_delta".to_string(), Value::Bool(true));
                object.insert(
                    "arroba_delta_offset_bytes".to_string(),
                    Value::Number((previous.byte_len as u64).into()),
                );
            }
        } else {
            output_text = Some(bounded_text_tail_string(
                current_bytes,
                MAX_PROVIDER_TOOL_STRING_BYTES,
                current_len,
            ));
            object.insert("arroba_output_replaced".to_string(), Value::Bool(true));
            object.insert(
                "arroba_original_bytes".to_string(),
                Value::Number((current_len as u64).into()),
            );
            object.insert(
                "arroba_message".to_string(),
                Value::String(
                    "provider tool output was replaced; showing the latest capped tail".to_string(),
                ),
            );
        }
    } else if current_len > MAX_PROVIDER_TOOL_STRING_BYTES {
        output_text = Some(bounded_text_tail_string(
            current_bytes,
            MAX_PROVIDER_TOOL_STRING_BYTES,
            current_len,
        ));
        object.insert("arroba_truncated".to_string(), Value::Bool(true));
        object.insert(
            "arroba_original_bytes".to_string(),
            Value::Number((current_len as u64).into()),
        );
    }
    if let Some(output_text) = output_text {
        object.insert(field, Value::String(output_text));
    }
    gate.insert(key, next_state);
    trim_provider_tool_delta_gate(&mut gate);
    serde_json::to_vec(&value).ok()
}

fn provider_tool_output_field(object: &serde_json::Map<String, Value>) -> Option<String> {
    ["output", "stdout", "stderr", "result", "content"]
        .iter()
        .filter_map(|field| {
            object
                .get(*field)
                .and_then(Value::as_str)
                .map(|text| ((*field).to_string(), text.len()))
        })
        .max_by_key(|(_, len)| *len)
        .map(|(field, _)| field)
}

fn json_string_field(object: &serde_json::Map<String, Value>, field: &str) -> Option<String> {
    object
        .get(field)
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .map(str::to_string)
}

fn provider_tool_metadata_fingerprint(
    object: &serde_json::Map<String, Value>,
    output_field: &str,
) -> u64 {
    const FNV_OFFSET: u64 = 0xcbf29ce484222325;
    object
        .iter()
        .filter(|(field, _)| field.as_str() != output_field)
        .fold(FNV_OFFSET, |hash, (field, value)| {
            hash_json_value(hash_bytes(hash, field.as_bytes()), value)
        })
}

fn hash_json_value(hash: u64, value: &Value) -> u64 {
    match value {
        Value::Null => hash_bytes(hash, b"n"),
        Value::Bool(value) => hash_bytes(hash, if *value { b"t" } else { b"f" }),
        Value::Number(value) => hash_bytes(hash, value.to_string().as_bytes()),
        Value::String(value) => hash_bounded_string(hash, value),
        Value::Array(values) => values.iter().fold(hash_bytes(hash, b"["), hash_json_value),
        Value::Object(object) => object
            .iter()
            .fold(hash_bytes(hash, b"{"), |hash, (field, value)| {
                hash_json_value(hash_bytes(hash, field.as_bytes()), value)
            }),
    }
}

fn hash_bytes(hash: u64, bytes: &[u8]) -> u64 {
    const FNV_PRIME: u64 = 0x100000001b3;
    bytes
        .iter()
        .fold(hash, |hash, byte| {
            (hash ^ u64::from(*byte)).wrapping_mul(FNV_PRIME)
        })
        .wrapping_mul(FNV_PRIME)
}

fn hash_bounded_string(hash: u64, value: &str) -> u64 {
    let bytes = value.as_bytes();
    let hash = hash_bytes(hash, &(bytes.len() as u64).to_le_bytes());
    if bytes.len() <= PROVIDER_TOOL_METADATA_HASH_STRING_BYTES {
        return hash_bytes(hash, bytes);
    }
    let head_len = PROVIDER_TOOL_METADATA_HASH_STRING_BYTES / 2;
    let tail_len = PROVIDER_TOOL_METADATA_HASH_STRING_BYTES.saturating_sub(head_len);
    let hash = hash_bytes(hash, &bytes[..head_len]);
    hash_bytes(hash, &bytes[bytes.len().saturating_sub(tail_len)..])
}

fn bounded_provider_tool_json(bytes: &[u8]) -> Option<Vec<u8>> {
    let mut value = serde_json::from_slice::<Value>(bytes).ok()?;
    let mut truncated = false;
    truncate_json_strings(&mut value, &mut truncated);
    if truncated {
        if let Value::Object(object) = &mut value {
            object.insert("arroba_truncated".to_string(), Value::Bool(true));
            object.insert(
                "arroba_original_bytes".to_string(),
                Value::Number((bytes.len() as u64).into()),
            );
        }
    }
    let encoded = serde_json::to_vec(&value).ok()?;
    if encoded.len() <= MAX_PROVIDER_OUTPUT_RECORD_BYTES {
        return Some(encoded);
    }
    let mut fallback = serde_json::Map::new();
    if let Value::Object(object) = &value {
        for key in ["id", "tool", "status", "name", "command"] {
            if let Some(value) = object.get(key).cloned() {
                fallback.insert(key.to_string(), value);
            }
        }
    }
    fallback.insert("arroba_truncated".to_string(), Value::Bool(true));
    fallback.insert(
        "arroba_original_bytes".to_string(),
        Value::Number((bytes.len() as u64).into()),
    );
    fallback.insert(
        "message".to_string(),
        Value::String(
            "provider tool payload omitted because it exceeded the terminal event size limit"
                .to_string(),
        ),
    );
    serde_json::to_vec(&Value::Object(fallback)).ok()
}

fn truncate_json_strings(value: &mut Value, truncated: &mut bool) {
    match value {
        Value::String(text) => {
            if text.len() > MAX_PROVIDER_TOOL_STRING_BYTES {
                let original_len = text.len();
                *text = bounded_text_string(
                    text.as_bytes(),
                    MAX_PROVIDER_TOOL_STRING_BYTES,
                    original_len,
                );
                *truncated = true;
            }
        }
        Value::Array(items) => {
            for item in items {
                truncate_json_strings(item, truncated);
            }
        }
        Value::Object(object) => {
            for value in object.values_mut() {
                truncate_json_strings(value, truncated);
            }
        }
        Value::Null | Value::Bool(_) | Value::Number(_) => {}
    }
}

fn bounded_text_bytes(bytes: &[u8]) -> Vec<u8> {
    bounded_text_string(bytes, MAX_PROVIDER_OUTPUT_RECORD_BYTES, bytes.len()).into_bytes()
}

fn bounded_text_string(bytes: &[u8], limit: usize, original_len: usize) -> String {
    let marker = format!(
        "\n\n[arroba: output truncated, omitted {} bytes]\n",
        original_len.saturating_sub(limit),
    );
    let marker_bytes = marker.as_bytes();
    let keep_limit = limit.saturating_sub(marker_bytes.len());
    let mut keep = keep_limit.min(bytes.len());
    while keep > 0 && std::str::from_utf8(&bytes[..keep]).is_err() {
        keep -= 1;
    }
    let mut text = String::from_utf8_lossy(&bytes[..keep]).into_owned();
    text.push_str(&marker);
    text
}

fn bounded_text_tail_string(bytes: &[u8], limit: usize, original_len: usize) -> String {
    if bytes.len() <= limit {
        return String::from_utf8_lossy(bytes).into_owned();
    }
    let marker = format!(
        "[arroba: output truncated, omitted {} leading bytes]\n\n",
        original_len.saturating_sub(limit),
    );
    let marker_bytes = marker.as_bytes();
    let keep_limit = limit.saturating_sub(marker_bytes.len());
    let mut keep = keep_limit.min(bytes.len());
    let start = bytes.len().saturating_sub(keep);
    while keep > 0 && std::str::from_utf8(&bytes[start..]).is_err() {
        keep -= 1;
    }
    let start = bytes.len().saturating_sub(keep);
    let mut text = marker;
    text.push_str(&String::from_utf8_lossy(&bytes[start..]));
    text
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct ProviderToolDeltaKey {
    session_id: String,
    provider_run_id: String,
    agent_id: String,
    tool_id: String,
    field: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ProviderToolDeltaState {
    byte_len: usize,
    tail_sample: Vec<u8>,
    metadata_fingerprint: u64,
}

impl ProviderToolDeltaState {
    fn from_bytes(bytes: &[u8]) -> Self {
        Self::from_parts(bytes, 0)
    }

    fn from_parts(bytes: &[u8], metadata_fingerprint: u64) -> Self {
        Self {
            byte_len: bytes.len(),
            tail_sample: provider_tool_delta_tail_sample(bytes),
            metadata_fingerprint,
        }
    }
}

fn provider_tool_delta_gate()
-> &'static Mutex<BTreeMap<ProviderToolDeltaKey, ProviderToolDeltaState>> {
    static GATE: OnceLock<Mutex<BTreeMap<ProviderToolDeltaKey, ProviderToolDeltaState>>> =
        OnceLock::new();
    GATE.get_or_init(|| Mutex::new(BTreeMap::new()))
}

#[cfg(test)]
fn reset_provider_tool_delta_gate_for_tests() {
    provider_tool_delta_gate()
        .lock()
        .expect("provider tool delta gate lock should not be poisoned")
        .clear();
}

#[cfg(test)]
fn provider_tool_delta_test_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

fn provider_tool_delta_tail_sample(bytes: &[u8]) -> Vec<u8> {
    let start = bytes
        .len()
        .saturating_sub(PROVIDER_TOOL_DELTA_TAIL_SAMPLE_BYTES);
    bytes[start..].to_vec()
}

fn provider_tool_output_is_append(previous: &ProviderToolDeltaState, current: &[u8]) -> bool {
    if previous.byte_len > current.len() {
        return false;
    }
    if previous.tail_sample.is_empty() {
        return true;
    }
    let start = previous.byte_len.saturating_sub(previous.tail_sample.len());
    current
        .get(start..previous.byte_len)
        .map(|sample| sample == previous.tail_sample.as_slice())
        .unwrap_or(false)
}

fn trim_provider_tool_delta_gate(
    gate: &mut BTreeMap<ProviderToolDeltaKey, ProviderToolDeltaState>,
) {
    while gate.len() > PROVIDER_TOOL_DELTA_GATE_LIMIT {
        let Some(oldest_key) = gate.keys().next().cloned() else {
            break;
        };
        gate.remove(&oldest_key);
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct ProviderOutputTruncationLogKey {
    session_id: String,
    provider_run_id: String,
    agent_id: String,
    kind: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
struct ProviderOutputTruncationLogState {
    last_original_bytes: usize,
    suppressed_logs: u64,
}

fn provider_output_truncation_log_gate()
-> &'static Mutex<BTreeMap<ProviderOutputTruncationLogKey, ProviderOutputTruncationLogState>> {
    static GATE: OnceLock<
        Mutex<BTreeMap<ProviderOutputTruncationLogKey, ProviderOutputTruncationLogState>>,
    > = OnceLock::new();
    GATE.get_or_init(|| Mutex::new(BTreeMap::new()))
}

pub(super) fn should_log_provider_output_truncation(
    session_id: &str,
    provider_run_id: &str,
    agent_id: Option<&str>,
    kind: &str,
    original_bytes: usize,
) -> Option<u64> {
    let key = ProviderOutputTruncationLogKey {
        session_id: session_id.to_string(),
        provider_run_id: provider_run_id.to_string(),
        agent_id: agent_id.unwrap_or("").to_string(),
        kind: kind.to_string(),
    };
    let mut gate = provider_output_truncation_log_gate()
        .lock()
        .expect("provider output truncation log gate lock should not be poisoned");
    let state = gate.entry(key).or_default();
    let should_log = state.last_original_bytes == 0
        || original_bytes < state.last_original_bytes
        || original_bytes.saturating_sub(state.last_original_bytes)
            >= PROVIDER_OUTPUT_TRUNCATION_LOG_DELTA_BYTES;
    if should_log {
        let suppressed_logs = state.suppressed_logs;
        state.last_original_bytes = original_bytes;
        state.suppressed_logs = 0;
        Some(suppressed_logs)
    } else {
        state.suppressed_logs = state.suppressed_logs.saturating_add(1);
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bounded_terminal_output_truncates_plain_output_with_marker() {
        let bytes = vec![b'x'; MAX_PROVIDER_OUTPUT_RECORD_BYTES + 1024];

        let bounded = bounded_terminal_output_bytes(&TerminalOutputKind::ProviderOutput, &bytes);

        assert!(bounded.len() <= MAX_PROVIDER_OUTPUT_RECORD_BYTES);
        let text = String::from_utf8(bounded).expect("bounded output should remain utf8");
        assert!(text.contains("[arroba: output truncated"));
    }

    #[test]
    fn bounded_terminal_output_preserves_provider_tool_json() {
        let payload = serde_json::json!({
            "id": "call-1",
            "tool": "shell",
            "status": "running",
            "output": "x".repeat(MAX_PROVIDER_OUTPUT_RECORD_BYTES + 1024),
        });
        let bytes = serde_json::to_vec(&payload).expect("payload should encode");

        let bounded = bounded_terminal_output_bytes(&TerminalOutputKind::ProviderTool, &bytes);

        assert!(bounded.len() <= MAX_PROVIDER_OUTPUT_RECORD_BYTES);
        let value: Value =
            serde_json::from_slice(&bounded).expect("bounded tool should remain json");
        assert_eq!(value.get("id").and_then(Value::as_str), Some("call-1"));
        assert_eq!(
            value.get("arroba_truncated").and_then(Value::as_bool),
            Some(true)
        );
    }

    #[test]
    fn provider_tool_output_uses_delta_for_cumulative_payloads() {
        let _guard = provider_tool_delta_test_lock()
            .lock()
            .expect("provider tool delta test lock should not be poisoned");
        reset_provider_tool_delta_gate_for_tests();
        let first = serde_json::json!({
            "id": "call-1",
            "tool": "shell",
            "status": "running",
            "output": "hello ",
        });
        let second = serde_json::json!({
            "id": "call-1",
            "tool": "shell",
            "status": "running",
            "output": "hello world",
        });

        let first = terminal_output_delta_bytes(
            "session-delta",
            "provider-run-1",
            Some("agent-1"),
            &TerminalOutputKind::ProviderTool,
            &Some("call-1".to_string()),
            first.to_string().as_bytes(),
        );
        let second = terminal_output_delta_bytes(
            "session-delta",
            "provider-run-1",
            Some("agent-1"),
            &TerminalOutputKind::ProviderTool,
            &Some("call-1".to_string()),
            second.to_string().as_bytes(),
        );

        let first: Value = serde_json::from_slice(&first).expect("first delta should be json");
        let second: Value = serde_json::from_slice(&second).expect("second delta should be json");
        assert_eq!(first.get("output").and_then(Value::as_str), Some("hello "));
        assert_eq!(second.get("output").and_then(Value::as_str), Some("world"));
        assert_eq!(
            second.get("arroba_delta").and_then(Value::as_bool),
            Some(true)
        );
    }

    #[test]
    fn provider_tool_output_emits_same_length_metadata_changes() {
        let _guard = provider_tool_delta_test_lock()
            .lock()
            .expect("provider tool delta test lock should not be poisoned");
        reset_provider_tool_delta_gate_for_tests();
        let running = serde_json::json!({
            "id": "call-1",
            "status": "running",
            "output": "hello",
        });
        let completed = serde_json::json!({
            "id": "call-1",
            "status": "completed",
            "output": "hello",
        });

        let _ = terminal_output_delta_bytes(
            "session-metadata",
            "provider-run-1",
            Some("agent-1"),
            &TerminalOutputKind::ProviderTool,
            &Some("call-1".to_string()),
            running.to_string().as_bytes(),
        );
        let completed = terminal_output_delta_bytes(
            "session-metadata",
            "provider-run-1",
            Some("agent-1"),
            &TerminalOutputKind::ProviderTool,
            &Some("call-1".to_string()),
            completed.to_string().as_bytes(),
        );

        let completed: Value =
            serde_json::from_slice(&completed).expect("metadata update should be json");
        assert_eq!(
            completed.get("status").and_then(Value::as_str),
            Some("completed")
        );
        assert_eq!(
            completed.get("output").and_then(Value::as_str),
            Some("hello")
        );
    }

    #[test]
    fn provider_tool_output_caps_replacement_tail() {
        let _guard = provider_tool_delta_test_lock()
            .lock()
            .expect("provider tool delta test lock should not be poisoned");
        reset_provider_tool_delta_gate_for_tests();
        let first = serde_json::json!({
            "id": "call-1",
            "status": "running",
            "output": "first output",
        });
        let replacement = serde_json::json!({
            "id": "call-1",
            "status": "running",
            "output": "x".repeat(MAX_PROVIDER_TOOL_STRING_BYTES + 1024),
        });

        let _ = terminal_output_delta_bytes(
            "session-replacement",
            "provider-run-1",
            Some("agent-1"),
            &TerminalOutputKind::ProviderTool,
            &Some("call-1".to_string()),
            first.to_string().as_bytes(),
        );
        let replacement = terminal_output_delta_bytes(
            "session-replacement",
            "provider-run-1",
            Some("agent-1"),
            &TerminalOutputKind::ProviderTool,
            &Some("call-1".to_string()),
            replacement.to_string().as_bytes(),
        );

        let replacement: Value =
            serde_json::from_slice(&replacement).expect("replacement should be json");
        let output = replacement
            .get("output")
            .and_then(Value::as_str)
            .expect("replacement should include capped output");
        assert!(output.len() <= MAX_PROVIDER_TOOL_STRING_BYTES);
        assert_eq!(
            replacement
                .get("arroba_output_replaced")
                .and_then(Value::as_bool),
            Some(true)
        );
    }

    #[test]
    fn provider_tool_output_suppresses_repeated_large_cumulative_payloads() {
        let _guard = provider_tool_delta_test_lock()
            .lock()
            .expect("provider tool delta test lock should not be poisoned");
        reset_provider_tool_delta_gate_for_tests();
        let first = vec![b'a'; PROVIDER_TOOL_LARGE_PAYLOAD_BYTES + 1];
        let second = vec![b'a'; PROVIDER_TOOL_LARGE_PAYLOAD_BYTES + 128];
        let third =
            vec![
                b'a';
                PROVIDER_TOOL_LARGE_PAYLOAD_BYTES + PROVIDER_TOOL_LARGE_PAYLOAD_REPORT_BYTES + 129
            ];

        let first = terminal_output_delta_bytes(
            "session-large-delta",
            "provider-run-1",
            Some("agent-1"),
            &TerminalOutputKind::ProviderTool,
            &Some("call-large".to_string()),
            &first,
        );
        let second = terminal_output_delta_bytes(
            "session-large-delta",
            "provider-run-1",
            Some("agent-1"),
            &TerminalOutputKind::ProviderTool,
            &Some("call-large".to_string()),
            &second,
        );
        let third = terminal_output_delta_bytes(
            "session-large-delta",
            "provider-run-1",
            Some("agent-1"),
            &TerminalOutputKind::ProviderTool,
            &Some("call-large".to_string()),
            &third,
        );

        assert!(!first.is_empty());
        assert!(second.is_empty());
        assert!(!third.is_empty());
        let first: Value = serde_json::from_slice(&first).expect("first summary should be json");
        assert_eq!(first.get("id").and_then(Value::as_str), Some("call-large"));
        assert_eq!(
            first.get("arroba_truncated").and_then(Value::as_bool),
            Some(true)
        );
    }
}
