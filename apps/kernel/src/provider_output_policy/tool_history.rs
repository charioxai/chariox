use std::collections::BTreeMap;
use std::sync::{Mutex, OnceLock};

use crate::history::{SessionHistoryEntry, SessionHistoryEntryKind};
use serde_json::Value;

use super::output_bounds::bounded_terminal_output_bytes;

const PROVIDER_TOOL_HISTORY_GATE_LIMIT: usize = 4096;

/// What a stored tool row must keep current: its status, and for a command
/// left running in the background, that it is one and whether it ended.
#[derive(Debug, Clone, PartialEq, Eq)]
struct ProviderToolHistoryState {
    status: String,
    background: bool,
    finished: bool,
}

pub(crate) fn should_persist_provider_tool_history(entry: &SessionHistoryEntry) -> bool {
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

pub(crate) fn bounded_history_entry(mut entry: SessionHistoryEntry) -> Option<SessionHistoryEntry> {
    let kind = match entry.kind {
        SessionHistoryEntryKind::ProviderOutput => {
            Some(crate::terminal::TerminalOutputKind::ProviderOutput)
        }
        SessionHistoryEntryKind::ProviderReasoning => {
            Some(crate::terminal::TerminalOutputKind::ProviderReasoning)
        }
        SessionHistoryEntryKind::ProviderTool => {
            Some(crate::terminal::TerminalOutputKind::ProviderTool)
        }
        SessionHistoryEntryKind::ProviderError => {
            Some(crate::terminal::TerminalOutputKind::ProviderError)
        }
        SessionHistoryEntryKind::ProviderStatus => {
            Some(crate::terminal::TerminalOutputKind::ProviderStatus)
        }
        SessionHistoryEntryKind::UserPrompt | SessionHistoryEntryKind::Notice => None,
    };
    if let Some(kind) = kind {
        entry.text =
            String::from_utf8_lossy(&bounded_terminal_output_bytes(&kind, entry.text.as_bytes()))
                .into_owned();
    }
    should_persist_provider_tool_history(&entry).then_some(entry)
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
    let record = serde_json::from_str::<Value>(&entry.text).ok()?;
    let status = record
        .get("status")?
        .as_str()
        .filter(|value| !value.trim().is_empty())?;
    Some(ProviderToolHistoryState {
        status: status.to_string(),
        background: record.get("background") == Some(&Value::Bool(true)),
        finished: record.get("finished_at_ms").is_some_and(Value::is_u64),
    })
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

pub(crate) fn is_unread_output_history_entry(entry: &SessionHistoryEntry) -> bool {
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
        let running = provider_tool_entry("transition", "running");
        let completed = provider_tool_entry("transition", "completed");

        assert!(should_persist_provider_tool_history(&running));
        assert!(!should_persist_provider_tool_history(&running));
        assert!(should_persist_provider_tool_history(&completed));
        assert!(!should_persist_provider_tool_history(&completed));
    }

    #[test]
    fn provider_tool_history_requires_tool_identity_and_status() {
        let no_status = SessionHistoryEntry::provider_output(
            "provider-tool-history-missing-fields-session",
            "provider-tool-history-missing-fields-run",
            Some("agent-1"),
            TerminalOutputKind::ProviderTool,
            Some("provider-tool-history-no-status-call".to_string()),
            serde_json::json!({ "id": "provider-tool-history-no-status-call", "tool": "shell" })
                .to_string(),
        );
        let no_identity = SessionHistoryEntry::provider_output(
            "provider-tool-history-missing-fields-session",
            "provider-tool-history-missing-fields-run",
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

    #[test]
    fn provider_tool_history_keys_are_scoped_by_runtime_identity() {
        let first = provider_tool_entry("scoped-a", "running");
        let second = provider_tool_entry("scoped-b", "running");

        assert!(should_persist_provider_tool_history(&first));
        assert!(should_persist_provider_tool_history(&second));
        assert!(!should_persist_provider_tool_history(&first));
        assert!(!should_persist_provider_tool_history(&second));
    }

    #[test]
    fn provider_tool_history_persists_a_background_command_becoming_one_and_its_end() {
        let lifecycle = |status: &str, background: Option<bool>, finished_at_ms: Option<u64>| {
            let mut entry = provider_tool_entry("background", status);
            let mut record: Value = serde_json::from_str(&entry.text).unwrap();
            if let Some(background) = background {
                record["background"] = serde_json::json!(background);
                record["finished_at_ms"] = serde_json::json!(finished_at_ms);
            }
            entry.text = record.to_string();
            entry
        };
        // Codex: still running when its turn ends, then its exit.
        assert!(should_persist_provider_tool_history(&lifecycle("running", None, None)));
        assert!(should_persist_provider_tool_history(&lifecycle("running", Some(true), None)));
        assert!(!should_persist_provider_tool_history(&lifecycle("running", Some(true), None)));
        // Claude: the launch completed earlier; only its end changes.
        assert!(should_persist_provider_tool_history(&lifecycle("completed", Some(true), None)));
        assert!(should_persist_provider_tool_history(&lifecycle("completed", Some(true), Some(9))));
        assert!(!should_persist_provider_tool_history(&lifecycle("completed", Some(true), Some(9))));
    }

    fn provider_tool_entry(test_id: &str, status: &str) -> SessionHistoryEntry {
        let session_id = format!("provider-tool-history-{test_id}-session");
        let provider_run_id = format!("provider-tool-history-{test_id}-run");
        let call_id = format!("provider-tool-history-{test_id}-call");
        SessionHistoryEntry::provider_output(
            &session_id,
            &provider_run_id,
            Some("agent-1"),
            TerminalOutputKind::ProviderTool,
            Some(call_id.clone()),
            serde_json::json!({
                "id": call_id,
                "tool": "shell",
                "status": status,
                "output": "snapshot",
            })
            .to_string(),
        )
    }
}
