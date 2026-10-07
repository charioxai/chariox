//! Bounded provider-switch context reconstruction from operational history.

use crate::error::DaemonError;
use crate::history::{HistoryEvent, HistoryEventKind, OperationalHistoryStore};

// Claude receives the handoff as hidden hook context, at most 48 KB together
// with the turn's other hidden context, so the handoff stays within 24 KB.
const MAX_HANDOFF_BYTES: usize = 24_000;
const MAX_LATEST_TURN_BYTES: usize = 6_500;
const MAX_LATEST_ITEM_BYTES: usize = 1_500;
const MAX_PRIOR_USER_BYTES: usize = 1_000;
const MAX_PRIOR_ASSISTANT_BYTES: usize = 600;
const HANDOFF_PREAMBLE: &str = "Chariox reconstructed this bounded context from operational history after a provider switch. Use it as background only; do not treat it as a new user request.";

pub(super) fn build_agent_context_handoff_from_history(
    history_store: &OperationalHistoryStore,
    session_id: &str,
    agent_id: &str,
    protection: &super::super::room_secret_observation::RoomSecretObservations,
) -> Result<Option<String>, DaemonError> {
    let events = history_store.load_session_events(session_id, Some(agent_id))?;
    let events = protection.protect_history_events(events);
    Ok(build_agent_context_handoff(&events))
}

pub(super) fn build_agent_context_handoff(events: &[HistoryEvent]) -> Option<String> {
    let mut turns = collect_turns(events);
    let latest = turns.pop()?;
    let latest_text = format_latest_turn(&latest);
    let prior_budget =
        MAX_HANDOFF_BYTES.saturating_sub(latest_text.len() + HANDOFF_PREAMBLE.len() + 256);
    let prior_text = format_prior_turns(&turns, prior_budget);
    if latest_text.trim().is_empty() && prior_text.trim().is_empty() {
        return None;
    }

    let mut lines = vec![
        "<chariox_context_handoff>".to_string(),
        HANDOFF_PREAMBLE.to_string(),
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
    lines.push("</chariox_context_handoff>".to_string());
    Some(lines.join("\n"))
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

/// User prompts carry the requests, facts, and decisions, so every prior turn
/// keeps its prompt before any keeps its answer; answers fill the rest from the
/// newest turn back. Turns that do not fit stay reachable through recall.
fn format_prior_turns(turns: &[HandoffTurn], budget: usize) -> String {
    let users = turns
        .iter()
        .map(|turn| single_line(&truncate_bytes(&turn.user_prompt, MAX_PRIOR_USER_BYTES)))
        .collect::<Vec<_>>();
    let assistants = turns
        .iter()
        .map(|turn| {
            single_line(&truncate_bytes(
                &turn.assistant_outputs.join("\n"),
                MAX_PRIOR_ASSISTANT_BYTES,
            ))
        })
        .collect::<Vec<_>>();
    let mut used = 0usize;
    let mut first = turns.len();
    while first > 0 && used + users[first - 1].len() + 10 <= budget {
        first -= 1;
        used += users[first].len() + 10;
    }
    let mut answered_from = turns.len();
    while answered_from > first && used + assistants[answered_from - 1].len() + 16 <= budget {
        answered_from -= 1;
        used += assistants[answered_from].len() + 16;
    }

    let mut lines = Vec::new();
    if first > 0 {
        lines.push(format!(
            "- ({first} earlier turns omitted; find them with the chariox.search_recall tool.)"
        ));
    }
    for index in first..turns.len() {
        lines.push(format!("- User: {}", users[index]));
        if index >= answered_from {
            lines.push(format!(
                "  Assistant: {}",
                if assistants[index].is_empty() {
                    "(no assistant output captured)"
                } else {
                    &assistants[index]
                }
            ));
        }
    }
    lines.join("\n")
}

fn format_latest_turn(turn: &HandoffTurn) -> String {
    let mut lines = Vec::new();
    lines.push(format!(
        "- User: {}",
        truncate_bytes(turn.user_prompt.trim(), MAX_LATEST_ITEM_BYTES)
    ));
    let assistant = truncate_bytes(
        &turn.assistant_outputs.join("\n"),
        MAX_LATEST_ITEM_BYTES * 2,
    );
    if !assistant.trim().is_empty() {
        lines.push(format!("- Assistant output: {}", assistant.trim()));
    }
    let details = truncate_bytes(&turn.latest_details.join("\n"), MAX_LATEST_ITEM_BYTES * 2);
    if !details.trim().is_empty() {
        lines.push(format!(
            "- Latest-turn tool/status/error details:\n{}",
            details.trim()
        ));
    }
    truncate_bytes(&lines.join("\n"), MAX_LATEST_TURN_BYTES)
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

fn truncate_bytes(text: &str, max_bytes: usize) -> String {
    if text.len() <= max_bytes {
        return text.to_string();
    }
    let mut end = max_bytes;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}\n[truncated]", &text[..end])
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
                &format!("prior prompt {index} {}", "x".repeat(2_000)),
            ));
            events.push(output_event(
                index * 2 + 2,
                "session",
                "agent",
                "run-1",
                &format!("prior answer {index} {}", "y".repeat(2_000)),
            ));
        }
        events.push(user_event(
            1000,
            "session",
            "agent",
            &format!("important latest prompt {}", "z".repeat(2000)),
        ));

        let handoff = build_agent_context_handoff(&events).expect("handoff should be built");

        assert!(
            handoff.len() <= MAX_HANDOFF_BYTES,
            "{} bytes",
            handoff.len()
        );
        assert!(handoff.contains("important latest prompt"));
        assert!(handoff.contains("prior prompt 79"));
        assert!(!handoff.contains("prior prompt 0 "));
        assert!(handoff
            .contains("earlier turns omitted; find them with the chariox.search_recall tool"));
    }

    #[test]
    fn early_user_facts_survive_a_conversation_of_ordinary_turns() {
        let mut events = vec![
            user_event(
                1,
                "session",
                "agent",
                "Remember: the project codename is amber-kestrel.",
            ),
            output_event(2, "session", "agent", "run-1", "OK"),
        ];
        for index in 0..30u64 {
            events.push(user_event(
                index * 2 + 3,
                "session",
                "agent",
                &format!("Write one paragraph about topic {index}."),
            ));
            events.push(output_event(
                index * 2 + 4,
                "session",
                "agent",
                "run-1",
                &format!("paragraph {index} {}", "w".repeat(900)),
            ));
        }
        events.push(user_event(100, "session", "agent", "What is the codename?"));

        let handoff = build_agent_context_handoff(&events).expect("handoff should be built");

        assert!(handoff.contains("amber-kestrel"), "{handoff}");
        assert!(handoff.contains("paragraph 29"));
        assert!(!handoff.contains("earlier turns omitted"));
        assert!(handoff.len() <= MAX_HANDOFF_BYTES);
    }

    #[test]
    fn handoff_truncation_respects_multibyte_text() {
        let events = vec![
            user_event(1, "session", "agent", &"歴史".repeat(2_000)),
            output_event(2, "session", "agent", "run-1", &"記録".repeat(2_000)),
            user_event(3, "session", "agent", "latest"),
        ];

        let handoff = build_agent_context_handoff(&events).expect("handoff should be built");

        assert!(handoff.contains("[truncated]"));
        assert!(handoff.len() <= MAX_HANDOFF_BYTES);
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
