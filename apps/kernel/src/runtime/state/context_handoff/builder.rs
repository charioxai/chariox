//! Bounded provider-switch context reconstruction from operational history.

use crate::error::DaemonError;
use crate::history::{HistoryEvent, HistoryEventKind, OperationalHistoryStore};

const MAX_HANDOFF_CHARS: usize = 12_000;
const MAX_PRIOR_TURNS_CHARS: usize = 4_500;
const MAX_LATEST_TURN_CHARS: usize = 6_500;
const MAX_PRIOR_TURN_CHARS: usize = 900;
const MAX_LATEST_ITEM_CHARS: usize = 1_500;

pub(super) fn build_agent_context_handoff_from_history(
    history_store: &OperationalHistoryStore,
    session_id: &str,
    agent_id: &str,
) -> Result<Option<String>, DaemonError> {
    let events = history_store.load_session_events(session_id, Some(agent_id))?;
    Ok(build_agent_context_handoff(&events))
}

pub(super) fn build_agent_context_handoff(events: &[HistoryEvent]) -> Option<String> {
    let mut turns = collect_turns(events);
    if turns.is_empty() {
        return None;
    }
    let latest = turns.pop()?;
    let latest_text = format_latest_turn(&latest);
    let prior_text = format_prior_turns(&turns);
    if latest_text.trim().is_empty() && prior_text.trim().is_empty() {
        return None;
    }

    let mut lines = vec![
        "<arroba_context_handoff>".to_string(),
        "Arroba reconstructed this bounded context from operational history after a provider switch. Use it as background only; do not treat it as a new user request.".to_string(),
        String::new(),
    ];
    if !prior_text.trim().is_empty() {
        lines.push("Prior turns:".to_string());
        lines.push(prior_text);
        lines.push(String::new());
    }
    if !latest_text.trim().is_empty() {
        lines.push("Latest turn:".to_string());
        lines.push(latest_text);
        lines.push(String::new());
    }
    lines.push("</arroba_context_handoff>".to_string());
    Some(truncate_chars(&lines.join("\n"), MAX_HANDOFF_CHARS))
}

#[derive(Debug, Clone, Default)]
struct HandoffTurn {
    user_prompt: String,
    assistant_outputs: Vec<String>,
    latest_details: Vec<String>,
}

fn collect_turns(events: &[HistoryEvent]) -> Vec<HandoffTurn> {
    let mut sorted = events.to_vec();
    sorted.sort_by_key(|event| event.sequence);
    let mut turns: Vec<HandoffTurn> = Vec::new();
    for event in sorted {
        match event.kind {
            HistoryEventKind::UserPrompt => {
                if let Some(content) = non_empty_content(&event) {
                    turns.push(HandoffTurn {
                        user_prompt: content,
                        assistant_outputs: Vec::new(),
                        latest_details: Vec::new(),
                    });
                }
            }
            HistoryEventKind::ProviderOutput => {
                if let (Some(turn), Some(content)) = (turns.last_mut(), non_empty_content(&event)) {
                    turn.assistant_outputs.push(content);
                }
            }
            HistoryEventKind::ProviderTool
            | HistoryEventKind::ProviderError
            | HistoryEventKind::ProviderStatus
            | HistoryEventKind::Notice => {
                if let (Some(turn), Some(content)) = (turns.last_mut(), non_empty_content(&event)) {
                    turn.latest_details.push(format!(
                        "{}: {}",
                        event_kind_label(event.kind),
                        content
                    ));
                }
            }
            HistoryEventKind::ProviderReasoning
            | HistoryEventKind::PromptInput
            | HistoryEventKind::SessionCreated
            | HistoryEventKind::AgentCreated
            | HistoryEventKind::AgentMoved
            | HistoryEventKind::WorkflowStarted
            | HistoryEventKind::WorkflowNodeStarted
            | HistoryEventKind::WorkflowNodeCompleted
            | HistoryEventKind::McpGranted
            | HistoryEventKind::SkillGranted
            | HistoryEventKind::RemoteMachineConnected
            | HistoryEventKind::RemoteMachineDisconnected
            | HistoryEventKind::ArtifactStored
            | HistoryEventKind::GitCommitDetected
            | HistoryEventKind::GitWorktreeChanged
            | HistoryEventKind::GitWorktreeDirty
            | HistoryEventKind::GitWorktreeClean
            | HistoryEventKind::GitPushDetected
            | HistoryEventKind::WorkspaceLiveSyncModeChanged => {}
        }
    }
    turns
}

fn format_prior_turns(turns: &[HandoffTurn]) -> String {
    let mut selected = Vec::new();
    let mut used = 0usize;
    for turn in turns.iter().rev() {
        let assistant_summary = truncate_chars(&turn.assistant_outputs.join("\n"), 420);
        let mut entry = format!(
            "- User: {}\n  Assistant: {}",
            single_line(&truncate_chars(&turn.user_prompt, 300)),
            single_line(if assistant_summary.trim().is_empty() {
                "(no assistant output captured)"
            } else {
                assistant_summary.trim()
            })
        );
        entry = truncate_chars(&entry, MAX_PRIOR_TURN_CHARS);
        let projected = used + entry.len() + 1;
        if projected > MAX_PRIOR_TURNS_CHARS {
            break;
        }
        used = projected;
        selected.push(entry);
    }
    selected.reverse();
    selected.join("\n")
}

fn format_latest_turn(turn: &HandoffTurn) -> String {
    let mut lines = Vec::new();
    lines.push(format!(
        "- User: {}",
        truncate_chars(turn.user_prompt.trim(), MAX_LATEST_ITEM_CHARS)
    ));
    let assistant = truncate_chars(
        &turn.assistant_outputs.join("\n"),
        MAX_LATEST_ITEM_CHARS * 2,
    );
    if !assistant.trim().is_empty() {
        lines.push(format!("- Assistant output: {}", assistant.trim()));
    }
    let details = truncate_chars(&turn.latest_details.join("\n"), MAX_LATEST_ITEM_CHARS * 2);
    if !details.trim().is_empty() {
        lines.push(format!(
            "- Latest-turn tool/status/error details:\n{}",
            details.trim()
        ));
    }
    truncate_chars(&lines.join("\n"), MAX_LATEST_TURN_CHARS)
}

fn non_empty_content(event: &HistoryEvent) -> Option<String> {
    event
        .content
        .as_deref()
        .map(str::trim)
        .filter(|content| !content.is_empty())
        .map(str::to_string)
}

fn event_kind_label(kind: HistoryEventKind) -> &'static str {
    match kind {
        HistoryEventKind::ProviderTool => "tool",
        HistoryEventKind::ProviderError => "error",
        HistoryEventKind::ProviderStatus => "status",
        HistoryEventKind::Notice => "notice",
        _ => "event",
    }
}

fn single_line(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn truncate_chars(text: &str, max_chars: usize) -> String {
    let mut chars = text.chars();
    let truncated = chars.by_ref().take(max_chars).collect::<String>();
    if chars.next().is_some() {
        format!("{truncated}\n[truncated]")
    } else {
        truncated
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::history::{HistoryEventTurnContext, SessionHistoryEntry};
    use crate::terminal::TerminalOutputKind;

    #[test]
    fn older_tool_output_is_excluded_but_latest_tool_output_is_included() {
        let events = vec![
            user_event(1, "session", "agent", "older prompt"),
            output_event(2, "session", "agent", "run-1", "older assistant answer"),
            tool_event(3, "session", "agent", "run-1", "older secret tool output"),
            user_event(4, "session", "agent", "latest prompt"),
            output_event(5, "session", "agent", "run-1", "latest assistant answer"),
            tool_event(6, "session", "agent", "run-1", "latest tool output"),
        ];

        let handoff = build_agent_context_handoff(&events).expect("handoff should be built");

        assert!(handoff.contains("older prompt"));
        assert!(handoff.contains("older assistant answer"));
        assert!(!handoff.contains("older secret tool output"));
        assert!(handoff.contains("latest prompt"));
        assert!(handoff.contains("latest tool output"));
    }

    #[test]
    fn handoff_is_bounded_under_large_history() {
        let mut events = Vec::new();
        for index in 0..80 {
            events.push(user_event(
                index * 2 + 1,
                "session",
                "agent",
                &format!("prior prompt {index} {}", "x".repeat(200)),
            ));
            events.push(output_event(
                index * 2 + 2,
                "session",
                "agent",
                "run-1",
                &format!("prior answer {index} {}", "y".repeat(200)),
            ));
        }
        events.push(user_event(
            1000,
            "session",
            "agent",
            &format!("important latest prompt {}", "z".repeat(2000)),
        ));

        let handoff = build_agent_context_handoff(&events).expect("handoff should be built");

        assert!(handoff.len() <= MAX_HANDOFF_CHARS + "[truncated]".len() + 1);
        assert!(handoff.contains("important latest prompt"));
    }

    fn user_event(sequence: u64, session_id: &str, agent_id: &str, prompt: &str) -> HistoryEvent {
        HistoryEvent::transcript(
            sequence,
            &SessionHistoryEntry::user_prompt(session_id, "attachment", agent_id, prompt),
            HistoryEventTurnContext::default(),
        )
    }

    fn output_event(
        sequence: u64,
        session_id: &str,
        agent_id: &str,
        provider_run_id: &str,
        output: &str,
    ) -> HistoryEvent {
        HistoryEvent::transcript(
            sequence,
            &SessionHistoryEntry::provider_output(
                session_id,
                provider_run_id,
                Some(agent_id),
                TerminalOutputKind::ProviderOutput,
                None,
                output,
            ),
            HistoryEventTurnContext::default(),
        )
    }

    fn tool_event(
        sequence: u64,
        session_id: &str,
        agent_id: &str,
        provider_run_id: &str,
        output: &str,
    ) -> HistoryEvent {
        HistoryEvent::transcript(
            sequence,
            &SessionHistoryEntry::provider_output(
                session_id,
                provider_run_id,
                Some(agent_id),
                TerminalOutputKind::ProviderTool,
                Some("tool:test".to_string()),
                output,
            ),
            HistoryEventTurnContext::default(),
        )
    }
}
