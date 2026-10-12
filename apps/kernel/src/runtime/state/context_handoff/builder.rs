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
    protection: &super::super::room_secret_observation::RoomSecretObservations,
) -> Result<Option<String>, DaemonError> {
    let events = history_store.load_session_events(session_id, Some(agent_id))?;
    let events = protection.protect_history_events(events);
    Ok(build_agent_context_handoff(&events))
}

pub(super) fn build_agent_context_handoff(events: &[HistoryEvent]) -> Option<String> {
    let mut turns = collect_turns(events, false);
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
        "<chariox_context_handoff>".to_string(),
        "Chariox reconstructed this bounded context from operational history after a provider switch. Use it as background only; do not treat it as a new user request.".to_string(),
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
    Some(truncate_chars(&lines.join("\n"), MAX_HANDOFF_CHARS))
}

// MP-08 / MP-10 / MP-11: additive bounded rendering for recreated leases.
// Keep the main branch's local profile-switch builder contract unchanged.
pub(super) fn build_recreated_lease_context(
    events: &[HistoryEvent],
    prompt_id: &str,
    max_bytes: usize,
) -> Option<String> {
    let mut turns = collect_turns(events, true);
    if turns
        .last()
        .is_some_and(|turn| turn.prompt_id.as_deref() == Some(prompt_id))
    {
        if turns.last().is_some_and(|turn| turn.has_provider_attempt) {
            turns.last_mut()?.user_prompt.clear();
        } else {
            turns.pop();
        }
    }
    let latest = turns.pop()?;
    const OPEN: &str = "<chariox_context_handoff>\nChariox reconstructed this bounded background from home history after worker lease recreation. The active user request is supplied separately.\n\n";
    const CLOSE: &str = "\n</chariox_context_handoff>";
    const LATEST: &str = "Latest turn:\n";
    const PRIOR: &str = "Prior turns:\n";
    let mut room = max_bytes.checked_sub(OPEN.len() + CLOSE.len() + LATEST.len())?;
    let latest_text = format_recreated_latest_turn(&latest, room.min(6_500));
    if latest_text.trim().is_empty() {
        return None;
    }
    room -= latest_text.len();
    let prior_text = format_recreated_prior_turns(&turns, room.saturating_sub(PRIOR.len() + 2));
    let mut packet = OPEN.to_string();
    if !prior_text.is_empty() {
        packet.push_str(PRIOR);
        packet.push_str(&prior_text);
        packet.push_str("\n\n");
    }
    packet.push_str(LATEST);
    packet.push_str(&latest_text);
    packet.push_str(CLOSE);
    Some(packet)
}

fn format_recreated_prior_turns(turns: &[HandoffTurn], budget: usize) -> String {
    let Some(mut room) = budget.checked_sub(omitted_turns_line(turns.len()).len() + 1) else {
        return String::new();
    };
    let users = turns
        .iter()
        .map(|turn| {
            format!(
                "- User: {}",
                single_line(&truncate_bytes(&turn.user_prompt, 1_000))
            )
        })
        .collect::<Vec<_>>();
    let answers = turns
        .iter()
        .map(|turn| {
            // MP-08 / MP-10 / MP-11: a title and its explanation remain separate
            // lines; flattening them can change an assistant-only fact.
            let answer = truncate_bytes(&turn.assistant_outputs.join("\n"), 600)
                .lines()
                .map(single_line)
                .collect::<Vec<_>>()
                .join("\n    ");
            (!answer.is_empty()).then(|| format!("  Assistant: {answer}"))
        })
        .collect::<Vec<_>>();
    let mut first = turns.len();
    while first > 0 && users[first - 1].len() < room {
        first -= 1;
        room -= users[first].len() + 1;
    }
    let mut answered_from = turns.len();
    while answered_from > first {
        let cost = answers[answered_from - 1]
            .as_ref()
            .map_or(0, |answer| answer.len() + 1);
        if cost > room {
            break;
        }
        answered_from -= 1;
        room -= cost;
    }

    let mut lines = Vec::new();
    if first > 0 {
        lines.push(omitted_turns_line(first));
    }
    for (index, (user, answer)) in users.into_iter().zip(answers).enumerate().skip(first) {
        lines.push(user);
        lines.extend(answer.filter(|_| index >= answered_from));
    }
    lines.join("\n")
}

fn omitted_turns_line(count: usize) -> String {
    format!("- ({count} earlier turns omitted; find them with the chariox.search_recall tool.)")
}

fn format_recreated_latest_turn(turn: &HandoffTurn, max_bytes: usize) -> String {
    let mut lines = Vec::new();
    if turn.user_prompt.is_empty() {
        lines.push("- Interrupted attempt of the current request:".to_string());
    } else {
        lines.push(format!(
            "- User: {}",
            truncate_bytes(turn.user_prompt.trim(), 1_500)
        ));
    }
    let assistant = truncate_bytes(&turn.assistant_outputs.join("\n"), 1_500 * 2);
    if !assistant.trim().is_empty() {
        lines.push(format!("- Assistant output: {}", assistant.trim()));
    }
    let details = truncate_bytes(&turn.latest_details.join("\n"), 1_500 * 2);
    if !details.trim().is_empty() {
        lines.push(format!(
            "- Latest-turn tool/status/error details:\n{}",
            details.trim()
        ));
    }
    truncate_bytes(&lines.join("\n"), max_bytes)
}

fn truncate_bytes(text: &str, max_bytes: usize) -> String {
    const TRUNCATED: &str = "\n[truncated]";
    if text.len() <= max_bytes {
        return text.to_string();
    }
    let mut end = max_bytes.checked_sub(TRUNCATED.len()).unwrap_or(0);
    if end == 0 {
        return String::new();
    }
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}{TRUNCATED}", &text[..end])
}

#[derive(Debug, Clone, Default)]
struct HandoffTurn {
    prompt_id: Option<String>,
    has_provider_attempt: bool,
    user_prompt: String,
    assistant_outputs: Vec<String>,
    output_item: Option<Option<String>>,
    latest_details: Vec<String>,
}

fn collect_turns(events: &[HistoryEvent], preserve_streamed_spacing: bool) -> Vec<HandoffTurn> {
    let mut sorted = events.to_vec();
    sorted.sort_by_key(|event| event.sequence);
    let mut turns: Vec<HandoffTurn> = Vec::new();
    for event in sorted {
        match event.kind {
            HistoryEventKind::UserPrompt => {
                if let Some(content) = non_empty_content(&event) {
                    turns.push(HandoffTurn {
                        prompt_id: event.prompt_id.clone(),
                        has_provider_attempt: false,
                        user_prompt: content,
                        assistant_outputs: Vec::new(),
                        output_item: None,
                        latest_details: Vec::new(),
                    });
                }
            }
            HistoryEventKind::ProviderOutput => {
                let content = if preserve_streamed_spacing {
                    event.content.clone()
                } else {
                    non_empty_content(&event)
                };
                if let (Some(turn), Some(content)) = (turns.last_mut(), content) {
                    turn.has_provider_attempt |= !content.trim().is_empty();
                    if preserve_streamed_spacing {
                        let item = event
                            .metadata
                            .get("merge_key")
                            .and_then(serde_json::Value::as_str)
                            .map(str::to_string);
                        let raw = event.content.as_deref().unwrap_or_default();
                        match turn.assistant_outputs.last_mut() {
                            Some(output) if turn.output_item.as_ref() == Some(&item) => {
                                output.push_str(raw)
                            }
                            _ => turn.assistant_outputs.push(raw.to_string()),
                        }
                        turn.output_item = Some(item);
                    } else {
                        turn.assistant_outputs.push(content);
                    }
                }
            }
            HistoryEventKind::ProviderTool
            | HistoryEventKind::ProviderError
            | HistoryEventKind::ProviderStatus
            | HistoryEventKind::Notice => {
                if let (Some(turn), Some(content)) = (turns.last_mut(), non_empty_content(&event)) {
                    if event.kind == HistoryEventKind::ProviderTool {
                        turn.output_item = None;
                    }
                    turn.has_provider_attempt |= event.kind != HistoryEventKind::Notice;
                    turn.latest_details.push(format!(
                        "{}: {}",
                        event_kind_label(event.kind),
                        content
                    ));
                }
            }
            HistoryEventKind::ProviderReasoning => {
                if let Some(turn) = turns.last_mut() {
                    turn.output_item = None;
                }
            }
            HistoryEventKind::PromptInput
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

    #[test]
    fn recreated_lease_excludes_unsubmitted_current_prompt() {
        let mut current = user_event(3, "session", "agent", "current request");
        current.prompt_id = Some("current".into());
        let events = vec![
            user_event(1, "session", "agent", "accepted goal"),
            output_event(2, "session", "agent", "old-run", "earlier answer"),
            current,
        ];
        let packet = build_recreated_lease_context(&events, "current", 24_000).unwrap();
        assert!(packet.contains("accepted goal"));
        assert!(packet.contains("earlier answer"));
        assert!(!packet.contains("current request"));
    }

    #[test]
    fn recreated_lease_keeps_interrupted_attempt_details_without_duplicate_request() {
        let mut current = user_event(1, "session", "agent", "retry request");
        current.prompt_id = Some("current".into());
        let events = vec![
            current,
            tool_event(2, "session", "agent", "old-run", "partial work"),
        ];
        let packet = build_recreated_lease_context(&events, "current", 24_000).unwrap();
        assert!(packet.contains("partial work"));
        assert!(!packet.contains("retry request"));
    }

    #[test]
    fn recreated_lease_budget_preserves_utf8_and_frame_with_native_context() {
        let events = vec![user_event(
            1,
            "session",
            "agent",
            &"🦀日本語".repeat(10_000),
        )];
        for room in [0, 100, 250, 500, 24_000] {
            if let Some(packet) = build_recreated_lease_context(&events, "current", room) {
                assert!(packet.len() <= room);
                assert!(packet.ends_with("</chariox_context_handoff>"));
            }
        }
        let hidden = "🦀".repeat(10_000);
        let room = crate::provider::claude_native_hidden_context_room(&hidden);
        let packet = build_recreated_lease_context(&events, "current", room).unwrap();
        crate::provider::ensure_claude_native_hidden_context_fits(
            "recreated-worker-run",
            &format!("{hidden}\n\n{packet}"),
        )
        .unwrap();
    }

    #[test]
    fn recreated_lease_keeps_early_facts_and_streamed_answer_spacing() {
        let mut events = vec![user_event(
            1,
            "session",
            "agent",
            "Keep codename amber-kestrel",
        )];
        for (seq, text) in [(2, "assistant fact: native"), (3, " "), (4, "continuity")] {
            events.push(output_event(seq, "session", "agent", "old-run", text));
        }
        for index in 0..30 {
            events.push(user_event(
                5 + index * 2,
                "session",
                "agent",
                &format!("Ordinary follow-up {index}"),
            ));
            events.push(output_event(
                6 + index * 2,
                "session",
                "agent",
                "old-run",
                "A paragraph about ordinary work.",
            ));
        }
        let packet = build_recreated_lease_context(&events, "current", 24_000).unwrap();
        assert!(packet.contains("amber-kestrel"));
        assert!(packet.contains("assistant fact: native continuity"));
        assert!(packet.len() <= 24_000);
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
