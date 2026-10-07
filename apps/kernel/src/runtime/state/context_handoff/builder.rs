//! Bounded provider-switch context reconstruction from operational history.

use crate::error::DaemonError;
use crate::history::{HistoryEvent, HistoryEventKind, OperationalHistoryStore};

/// The default handoff size. A Claude native turn renders it into the room its
/// other hidden context leaves under the hook ceiling instead.
pub(super) const MAX_HANDOFF_BYTES: usize = 24_000;
const MAX_LATEST_TURN_BYTES: usize = 6_500;
const MAX_LATEST_ITEM_BYTES: usize = 1_500;
const MAX_PRIOR_USER_BYTES: usize = 1_000;
const MAX_PRIOR_ASSISTANT_BYTES: usize = 600;
const HANDOFF_OPEN: &str = "<chariox_context_handoff>";
const HANDOFF_CLOSE: &str = "</chariox_context_handoff>";
const HANDOFF_PREAMBLE: &str = "Chariox reconstructed this bounded context from operational history after a provider switch. Use it as background only; do not treat it as a new user request.";
const PRIOR_TURNS_HEADER: &str = "Prior turns:\n";
const LATEST_TURN_HEADER: &str = "Latest turn:\n";
const TRUNCATED: &str = "\n[truncated]";

/// An agent's protected conversation, rendered on demand within a byte budget.
#[derive(Debug, Clone, Default)]
pub(super) struct AgentConversation {
    turns: Vec<HandoffTurn>,
}

pub(super) fn load_agent_conversation(
    history_store: &OperationalHistoryStore,
    session_id: &str,
    agent_id: &str,
    dispatching_prompt_id: Option<&str>,
    protection: &super::super::room_secret_observation::RoomSecretObservations,
) -> Result<AgentConversation, DaemonError> {
    let events = history_store.load_session_events(session_id, Some(agent_id))?;
    let events = protection.protect_history_events(events);
    let conversation = AgentConversation::from_events(&events);
    Ok(match dispatching_prompt_id {
        Some(prompt_id) => conversation.before_prompt(prompt_id),
        None => conversation,
    })
}

impl AgentConversation {
    pub(super) fn from_events(events: &[HistoryEvent]) -> Self {
        Self {
            turns: collect_turns(events),
        }
    }

    /// The conversation a dispatch of `prompt_id` continues. History records a
    /// prompt before its dispatch, and the provider receives that prompt as the
    /// request itself.
    pub(super) fn before_prompt(mut self, prompt_id: &str) -> Self {
        if self
            .turns
            .last()
            .is_some_and(|turn| turn.prompt_id.as_deref() == Some(prompt_id))
        {
            self.turns.pop();
        }
        self
    }

    pub(super) fn is_empty(&self) -> bool {
        self.turns.is_empty()
    }

    /// The handoff packet in at most `max_bytes`. Prior turns give way first,
    /// then the latest turn's details; `None` when not even its frame fits.
    pub(super) fn render(&self, max_bytes: usize) -> Option<String> {
        let (latest, prior) = self.turns.split_last()?;
        let mut packet = format!("{HANDOFF_OPEN}\n{HANDOFF_PREAMBLE}\n\n");
        let mut room = max_bytes.checked_sub(packet.len() + HANDOFF_CLOSE.len())?;
        let latest_text = format_latest_turn(
            latest,
            room.saturating_sub(LATEST_TURN_HEADER.len() + 2)
                .min(MAX_LATEST_TURN_BYTES),
        );
        if !latest_text.is_empty() {
            room -= LATEST_TURN_HEADER.len() + latest_text.len() + 2;
        }
        let prior_text =
            format_prior_turns(prior, room.saturating_sub(PRIOR_TURNS_HEADER.len() + 2));
        if latest_text.is_empty() && prior_text.is_empty() {
            return None;
        }
        for (header, text) in [
            (PRIOR_TURNS_HEADER, prior_text),
            (LATEST_TURN_HEADER, latest_text),
        ] {
            if !text.is_empty() {
                packet.push_str(header);
                packet.push_str(&text);
                packet.push_str("\n\n");
            }
        }
        packet.push_str(HANDOFF_CLOSE);
        Some(packet)
    }
}

#[derive(Debug, Clone, Default)]
struct HandoffTurn {
    prompt_id: Option<String>,
    user_prompt: String,
    /// One entry per answer item: a streamed item's deltas, or a whole block.
    assistant_outputs: Vec<String>,
    /// The merge key of the item the last output row extended.
    output_item: Option<Option<String>>,
    latest_details: Vec<String>,
}

impl HandoffTurn {
    /// Deltas of one streamed item join as written; separate items, or text
    /// on either side of a tool call, start a new line.
    fn push_output(&mut self, event: &HistoryEvent, content: &str) {
        let item = event
            .metadata
            .get("merge_key")
            .and_then(serde_json::Value::as_str)
            .map(str::to_string);
        match self.assistant_outputs.last_mut() {
            Some(output) if self.output_item.as_ref() == Some(&item) => output.push_str(content),
            _ => self.assistant_outputs.push(content.to_string()),
        }
        self.output_item = Some(item);
    }

    fn answer(&self) -> String {
        self.assistant_outputs.join("\n")
    }
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
                        prompt_id: event.prompt_id.clone(),
                        user_prompt: content,
                        assistant_outputs: Vec::new(),
                        output_item: None,
                        latest_details: Vec::new(),
                    });
                }
            }
            HistoryEventKind::ProviderOutput => {
                // Answers stream as deltas whose spacing is part of the text.
                if let (Some(turn), Some(content)) = (turns.last_mut(), event.content.as_ref()) {
                    turn.push_output(&event, content);
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
/// newest turn back. Turns that do not fit stay reachable through recall. Every
/// line is charged with its newline, and the omitted-turns line up front.
fn format_prior_turns(turns: &[HandoffTurn], budget: usize) -> String {
    let Some(mut room) = budget.checked_sub(omitted_turns_line(turns.len()).len() + 1) else {
        return String::new();
    };
    let users = turns
        .iter()
        .map(|turn| {
            format!(
                "- User: {}",
                single_line(&truncate_bytes(&turn.user_prompt, MAX_PRIOR_USER_BYTES))
            )
        })
        .collect::<Vec<_>>();
    let answers = turns
        .iter()
        .map(|turn| {
            let answer = single_line(&truncate_bytes(&turn.answer(), MAX_PRIOR_ASSISTANT_BYTES));
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

fn format_latest_turn(turn: &HandoffTurn, max_bytes: usize) -> String {
    let mut lines = Vec::new();
    lines.push(format!(
        "- User: {}",
        truncate_bytes(turn.user_prompt.trim(), MAX_LATEST_ITEM_BYTES)
    ));
    let assistant = truncate_bytes(&turn.answer(), MAX_LATEST_ITEM_BYTES * 2);
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
    truncate_bytes(&lines.join("\n"), max_bytes)
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

/// At most `max_bytes`, marker included.
fn truncate_bytes(text: &str, max_bytes: usize) -> String {
    if text.len() <= max_bytes {
        return text.to_string();
    }
    let Some(mut end) = max_bytes.checked_sub(TRUNCATED.len()) else {
        return String::new();
    };
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}{TRUNCATED}", &text[..end])
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

        let handoff = build_agent_context_handoff(&events, MAX_HANDOFF_BYTES)
            .expect("handoff should be built");

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

        let handoff = build_agent_context_handoff(&events, MAX_HANDOFF_BYTES)
            .expect("handoff should be built");

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

        let handoff = build_agent_context_handoff(&events, MAX_HANDOFF_BYTES)
            .expect("handoff should be built");

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

        let handoff = build_agent_context_handoff(&events, MAX_HANDOFF_BYTES)
            .expect("handoff should be built");

        assert!(handoff.contains("[truncated]"));
        assert!(handoff.len() <= MAX_HANDOFF_BYTES);
    }

    #[test]
    fn the_dispatching_prompt_leaves_the_last_completed_turn_latest() {
        let mut active = user_event(5, "session", "agent", "the request being dispatched");
        active.prompt_id = Some("prompt-active".to_string());
        let events = vec![
            user_event(1, "session", "agent", "older prompt"),
            output_event(2, "session", "agent", "run-1", "older answer"),
            user_event(3, "session", "agent", "run the migration"),
            error_event(
                4,
                "session",
                "agent",
                "run-1",
                "auth failed: quota exhausted",
            ),
            active,
        ];

        let handoff = AgentConversation::from_events(&events)
            .before_prompt("prompt-active")
            .render(MAX_HANDOFF_BYTES)
            .expect("handoff should be built");

        assert!(
            !handoff.contains("the request being dispatched"),
            "{handoff}"
        );
        let latest = handoff
            .split(LATEST_TURN_HEADER)
            .nth(1)
            .expect("latest turn");
        assert!(latest.starts_with("- User: run the migration"), "{handoff}");
        assert!(
            latest.contains("error: auth failed: quota exhausted"),
            "{handoff}"
        );
        assert!(AgentConversation::from_events(&events[4..])
            .before_prompt("prompt-active")
            .is_empty());
    }

    #[test]
    fn streamed_answer_chunks_render_as_one_answer() {
        let events = vec![
            user_event(1, "session", "agent", "Invent a release name."),
            item_event(2, "msg-1", "silver-l"),
            item_event(3, "msg-1", "antern"),
            user_event(4, "session", "agent", "Multiply."),
            item_event(5, "msg-2", "410"),
            item_event(6, "msg-2", "39518"),
            item_event(7, "msg-2", " is the"),
            item_event(8, "msg-2", " product."),
        ];

        let handoff = build_agent_context_handoff(&events, MAX_HANDOFF_BYTES).unwrap();

        assert!(handoff.contains("Assistant: silver-lantern"), "{handoff}");
        assert!(
            handoff.contains("- Assistant output: 41039518 is the product."),
            "{handoff}"
        );
    }

    #[test]
    fn whole_answer_blocks_stay_apart() {
        let events = vec![
            user_event(1, "session", "agent", "What is in the file?"),
            item_event(
                2,
                "claude-transcript:run-1:assistant:m1:0",
                "Let me check the file.",
            ),
            item_event(
                3,
                "claude-transcript:run-1:assistant:m2:0",
                "The file contains a parser.",
            ),
            user_event(4, "session", "agent", "And the other one?"),
            item_event(5, "claude:run-1:assistant", "Checking."),
            tool_event(6, "session", "agent", "run-1", "cat other.rs"),
            item_event(7, "claude:run-1:assistant", "It holds the lexer."),
        ];

        let handoff = build_agent_context_handoff(&events, MAX_HANDOFF_BYTES).unwrap();

        assert!(
            handoff.contains("Assistant: Let me check the file. The file contains a parser."),
            "{handoff}"
        );
        assert!(
            handoff.contains("- Assistant output: Checking.\nIt holds the lexer."),
            "{handoff}"
        );
    }

    fn build_agent_context_handoff(events: &[HistoryEvent], max_bytes: usize) -> Option<String> {
        AgentConversation::from_events(events).render(max_bytes)
    }

    #[test]
    fn handoff_shrinks_to_any_budget_and_keeps_the_latest_request_longest() {
        let mut events = Vec::new();
        for index in 0..40u64 {
            events.push(user_event(
                index * 2 + 1,
                "session",
                "agent",
                &format!("prior prompt {index} {}", "x".repeat(700)),
            ));
            events.push(output_event(
                index * 2 + 2,
                "session",
                "agent",
                "run-1",
                &format!("prior answer {index} {}", "y".repeat(700)),
            ));
        }
        events.push(user_event(1_000, "session", "agent", "latest request"));
        events.push(tool_event(
            1_001,
            "session",
            "agent",
            "run-1",
            &"t".repeat(5_000),
        ));

        for max_bytes in [0, 200, 400, 1_000, 3_000, 8_000, 16_000, MAX_HANDOFF_BYTES] {
            let handoff = build_agent_context_handoff(&events, max_bytes);
            let len = handoff.as_ref().map_or(0, String::len);
            assert!(
                len <= max_bytes,
                "{len} bytes for a {max_bytes}-byte budget"
            );
            if max_bytes >= 400 {
                assert!(handoff.unwrap().contains("latest request"), "{max_bytes}");
            }
        }
        let small = build_agent_context_handoff(&events, 8_000).unwrap();
        assert!(small.contains("prior prompt 39"));
        assert!(!small.contains("prior prompt 0 "));
    }

    #[test]
    fn handoff_budget_is_strict_for_unanswered_turns_and_truncated_text() {
        let mut events = (0..600u64)
            .map(|index| user_event(index + 1, "session", "agent", &format!("question {index}")))
            .collect::<Vec<_>>();
        events.push(user_event(1_000, "session", "agent", &"z".repeat(10_000)));

        let handoff = build_agent_context_handoff(&events, MAX_HANDOFF_BYTES)
            .expect("handoff should be built");

        assert!(
            handoff.len() <= MAX_HANDOFF_BYTES,
            "{} bytes",
            handoff.len()
        );
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

    fn item_event(sequence: u64, merge_key: &str, output: &str) -> HistoryEvent {
        HistoryEvent::transcript(
            sequence,
            &SessionHistoryEntry::provider_output(
                "session",
                "run-1",
                Some("agent"),
                TerminalOutputKind::ProviderOutput,
                Some(merge_key.to_string()),
                output,
            ),
            HistoryEventTurnContext::default(),
        )
    }

    fn error_event(
        sequence: u64,
        session_id: &str,
        agent_id: &str,
        provider_run_id: &str,
        error: &str,
    ) -> HistoryEvent {
        HistoryEvent::transcript(
            sequence,
            &SessionHistoryEntry::provider_output(
                session_id,
                provider_run_id,
                Some(agent_id),
                TerminalOutputKind::ProviderError,
                None,
                error,
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
