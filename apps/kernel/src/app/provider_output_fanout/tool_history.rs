use std::collections::BTreeMap;
use std::sync::{Mutex, OnceLock};

use crate::history::{SessionHistoryEntry, SessionHistoryEntryKind};
use serde_json::Value;

const PROVIDER_TOOL_HISTORY_GATE_LIMIT: usize = 4096;

#[derive(Debug, Clone, PartialEq, Eq)]
struct ProviderToolHistoryState {
    status: String,
}

pub(super) fn should_persist_provider_tool_history(entry: &SessionHistoryEntry) -> bool {
    if entry.kind != SessionHistoryEntryKind::ProviderTool {
        return true;
    }
    let Some(key) = provider_tool_history_key(entry) else {
        return true;
    };
    let Some(state) = provider_tool_history_state(entry) else {
        return true;
    };
    let mut gate = provider_tool_history_gate()
        .lock()
        .expect("provider tool history gate lock should not be poisoned");
    let should_persist = gate.get(&key) != Some(&state);
    if should_persist {
        gate.insert(key, state);
        while gate.len() > PROVIDER_TOOL_HISTORY_GATE_LIMIT {
            let Some(oldest_key) = gate.keys().next().cloned() else {
                break;
            };
            gate.remove(&oldest_key);
        }
    }
    should_persist
}

fn provider_tool_history_key(entry: &SessionHistoryEntry) -> Option<String> {
    let provider_run_id = entry.provider_run_id.as_deref()?;
    let tool_id = entry
        .merge_key
        .clone()
        .or_else(|| provider_tool_json_string_field(&entry.text, "id"))
        .or_else(|| provider_tool_json_string_field(&entry.text, "call_id"))?;
    Some(format!(
        "{}\u{1f}{}\u{1f}{}\u{1f}{}",
        entry.session_id,
        provider_run_id,
        entry.agent_id.as_deref().unwrap_or(""),
        tool_id
    ))
}

fn provider_tool_history_state(entry: &SessionHistoryEntry) -> Option<ProviderToolHistoryState> {
    provider_tool_json_string_field(&entry.text, "status")
        .map(|status| ProviderToolHistoryState { status })
}

fn provider_tool_json_string_field(text: &str, field: &str) -> Option<String> {
    serde_json::from_str::<Value>(text)
        .ok()?
        .get(field)?
        .as_str()
        .filter(|value| !value.trim().is_empty())
        .map(str::to_string)
}

fn provider_tool_history_gate() -> &'static Mutex<BTreeMap<String, ProviderToolHistoryState>> {
    static GATE: OnceLock<Mutex<BTreeMap<String, ProviderToolHistoryState>>> = OnceLock::new();
    GATE.get_or_init(|| Mutex::new(BTreeMap::new()))
}

#[cfg(test)]
fn reset_provider_tool_history_gate_for_tests() {
    provider_tool_history_gate()
        .lock()
        .expect("provider tool history gate lock should not be poisoned")
        .clear();
}

pub(super) fn is_unread_output_history_entry(entry: &SessionHistoryEntry) -> bool {
    matches!(
        entry.kind,
        SessionHistoryEntryKind::ProviderOutput
            | SessionHistoryEntryKind::ProviderReasoning
            | SessionHistoryEntryKind::ProviderTool
            | SessionHistoryEntryKind::ProviderError
            | SessionHistoryEntryKind::Notice
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::terminal::TerminalOutputKind;

    #[test]
    fn provider_tool_history_persists_first_running_and_status_transition() {
        reset_provider_tool_history_gate_for_tests();
        let running = provider_tool_entry("running");
        let completed = provider_tool_entry("completed");

        assert!(should_persist_provider_tool_history(&running));
        assert!(!should_persist_provider_tool_history(&running));
        assert!(should_persist_provider_tool_history(&completed));
        assert!(!should_persist_provider_tool_history(&completed));
    }

    #[test]
    fn provider_tool_history_requires_tool_identity_and_status() {
        reset_provider_tool_history_gate_for_tests();
        let no_status = SessionHistoryEntry::provider_output(
            "session-1",
            "provider-run-1",
            Some("agent-1"),
            TerminalOutputKind::ProviderTool,
            Some("call-1".to_string()),
            serde_json::json!({ "id": "call-1", "tool": "shell" }).to_string(),
        );
        let no_identity = SessionHistoryEntry::provider_output(
            "session-1",
            "provider-run-1",
            Some("agent-1"),
            TerminalOutputKind::ProviderTool,
            None,
            serde_json::json!({ "status": "running", "tool": "shell" }).to_string(),
        );

        assert!(should_persist_provider_tool_history(&no_status));
        assert!(should_persist_provider_tool_history(&no_status));
        assert!(should_persist_provider_tool_history(&no_identity));
        assert!(should_persist_provider_tool_history(&no_identity));
    }

    fn provider_tool_entry(status: &str) -> SessionHistoryEntry {
        SessionHistoryEntry::provider_output(
            "session-1",
            "provider-run-1",
            Some("agent-1"),
            TerminalOutputKind::ProviderTool,
            Some("call-1".to_string()),
            serde_json::json!({
                "id": "call-1",
                "tool": "shell",
                "status": status,
                "output": "snapshot",
            })
            .to_string(),
        )
    }
}
