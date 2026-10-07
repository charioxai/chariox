//! Bounded provider-switch context reconstruction from operational history.

use crate::error::DaemonError;
use crate::history::{HistoryEvent, HistoryEventKind, OperationalHistoryStore};

use super::facts::{KernelFacts, ToolCall};

/// The handoff size the tests render at: the old fixed budget.
#[cfg(test)]
pub(super) const MAX_HANDOFF_BYTES: usize = 24_000;
const MAX_LATEST_TURN_BYTES: usize = 6_500;
const MAX_LATEST_ITEM_BYTES: usize = 1_500;
const MAX_PRIOR_USER_BYTES: usize = 1_000;
const MAX_PRIOR_ASSISTANT_BYTES: usize = 600;
/// The prior turns right before the latest keep longer answers: the verbatim
/// recent tail a new session continues from.
const RECENT_TAIL_TURNS: usize = 3;
const MAX_RECENT_ASSISTANT_BYTES: usize = 3_000;
const HANDOFF_OPEN: &str = "<chariox_context_handoff>";
const HANDOFF_CLOSE: &str = "</chariox_context_handoff>";
const HANDOFF_PREAMBLE: &str = "Chariox reconstructed this bounded context from operational history after a provider switch. Use it as background only; do not treat it as a new user request.";
const BRIEF_HEADER: &str =
    "Handoff brief (written by a model from the conversation; the turns below are verbatim):\n";
const FACTS_HEADER: &str = "Recorded by the Chariox kernel:\n";
const PRIOR_TURNS_HEADER: &str = "Prior turns:\n";
const LATEST_TURN_HEADER: &str = "Latest turn:\n";
const TRUNCATED: &str = "\n[truncated]";

/// An agent's protected conversation with the kernel's facts about it and
/// its handoff brief, rendered on demand within a byte budget.
#[derive(Debug, Clone, Default)]
pub(super) struct AgentConversation {
    turns: Vec<HandoffTurn>,
    pub(super) facts: KernelFacts,
    pub(super) brief: Option<String>,
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
        let mut sorted = events.to_vec();
        sorted.sort_by_key(|event| event.sequence);
        Self {
            turns: collect_turns(&sorted),
            facts: KernelFacts::from_events(&sorted),
            brief: None,
        }
    }

    /// The conversation a dispatch of `prompt_id` continues. History records a
    /// prompt before its dispatch, and the provider receives that prompt as the
    /// request itself. An interrupted trailing attempt keeps its output and
    /// details, with only the duplicated user line removed.
    pub(super) fn before_prompt(mut self, prompt_id: &str) -> Self {
        if self
            .turns
            .last()
            .is_some_and(|turn| turn.prompt_id.as_deref() == Some(prompt_id))
        {
            let turn = self.turns.last_mut().unwrap();
            if turn.has_provider_attempt {
                // The retry supplies the request, but needs the half-applied work
                // and the failure that interrupted it.
                turn.user_prompt.clear();
            } else {
                self.turns.pop();
            }
        }
        self
    }

    pub(super) fn is_empty(&self) -> bool {
        self.turns.is_empty()
    }

    /// The handoff packet in at most `max_bytes`: the brief, the kernel's
    /// facts, the prior turns and the latest turn. Under a tight budget prior
    /// turns give way first, then the facts, the latest turn's details and the
    /// brief; `None` when not even the frame fits.
    pub(super) fn render(&self, max_bytes: usize) -> Option<String> {
        let (brief, facts) = (self.brief.as_deref(), self.facts.render(max_bytes));
        let (latest, prior) = self.turns.split_last()?;
        let mut packet = format!("{HANDOFF_OPEN}\n{HANDOFF_PREAMBLE}\n\n");
        let mut room = max_bytes.checked_sub(packet.len() + HANDOFF_CLOSE.len())?;
        let section = |header: &str, text: String, room: &mut usize| {
            if !text.is_empty() {
                *room -= header.len() + text.len() + 2;
            }
            text
        };
        let fit = |header: &str, room: usize| room.saturating_sub(header.len() + 2);
        let brief = section(
            BRIEF_HEADER,
            truncate_bytes(
                brief.unwrap_or_default().trim(),
                fit(BRIEF_HEADER, room) / 2,
            ),
            &mut room,
        );
        let latest = section(
            LATEST_TURN_HEADER,
            format_latest_turn(
                latest,
                fit(LATEST_TURN_HEADER, room).min(MAX_LATEST_TURN_BYTES),
            ),
            &mut room,
        );
        let facts = section(
            FACTS_HEADER,
            truncate_bytes(&facts, fit(FACTS_HEADER, room) / 2),
            &mut room,
        );
        let prior = format_prior_turns(prior, fit(PRIOR_TURNS_HEADER, room));
        if latest.is_empty() && prior.is_empty() {
            return None;
        }
        for (header, text) in [
            (BRIEF_HEADER, brief),
            (FACTS_HEADER, facts),
            (PRIOR_TURNS_HEADER, prior),
            (LATEST_TURN_HEADER, latest),
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
    /// Each detail line with the tool call id it reports, if any.
    latest_details: Vec<(Option<String>, String)>,
    // Notices are attributed by sequence and do not prove this request ran.
    has_provider_attempt: bool,
}

impl HandoffTurn {
    /// Deltas of one streamed item join as written; separate items, or text
    /// on either side of a tool call, start a new line.
    fn push_output(&mut self, event: &HistoryEvent, content: &str) {
        let item = answer_item(event).map(str::to_string);
        match self.assistant_outputs.last_mut() {
            Some(output) if self.output_item.as_ref() == Some(&item) => output.push_str(content),
            _ => self.assistant_outputs.push(content.to_string()),
        }
        self.output_item = Some(item);
    }

    fn answer(&self) -> String {
        self.assistant_outputs.join("\n")
    }

    /// A tool call reads as one line, what ran and how it ended, and its
    /// later updates replace that line.
    fn add_detail(&mut self, kind: HistoryEventKind, content: &str) {
        let call = (kind == HistoryEventKind::ProviderTool)
            .then(|| ToolCall::parse(content))
            .flatten();
        let Some(call) = call else {
            self.latest_details
                .push((None, format!("{}: {content}", event_kind_label(kind))));
            return;
        };
        let line = format!("tool: {}", call.transcript_line(MAX_LATEST_ITEM_BYTES));
        match self
            .latest_details
            .iter()
            .position(|(id, _)| id.is_some() && *id == call.id)
        {
            Some(index) => self.latest_details[index].1 = line,
            None => self.latest_details.push((call.id, line)),
        }
    }
}

fn collect_turns(sorted: &[HistoryEvent]) -> Vec<HandoffTurn> {
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
                        has_provider_attempt: false,
                    });
                }
            }
            HistoryEventKind::ProviderOutput => {
                // Answers stream as deltas whose spacing is part of the text.
                if let (Some(turn), Some(content)) = (turns.last_mut(), event.content.as_ref()) {
                    turn.has_provider_attempt |= !content.trim().is_empty();
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
                    turn.has_provider_attempt |= event.kind != HistoryEventKind::Notice;
                    turn.add_detail(event.kind, &content);
                }
            }
            HistoryEventKind::ProviderReasoning => {
                // Claude -p shares the text merge key across blocks. Thinking
                // between them is a block boundary, not an answer delta.
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
    let recent_from = turns.len().saturating_sub(RECENT_TAIL_TURNS);
    let answers = turns
        .iter()
        .enumerate()
        .map(|(index, turn)| {
            let max_bytes = if index >= recent_from {
                MAX_RECENT_ASSISTANT_BYTES
            } else {
                MAX_PRIOR_ASSISTANT_BYTES
            };
            let answer = single_line(&truncate_bytes(&turn.answer(), max_bytes));
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
    if turn.user_prompt.is_empty() {
        lines.push("- Interrupted attempt of the current request:".to_string());
    } else {
        lines.push(format!(
            "- User: {}",
            truncate_bytes(turn.user_prompt.trim(), MAX_LATEST_ITEM_BYTES)
        ));
    }
    let assistant = truncate_bytes(&turn.answer(), MAX_LATEST_ITEM_BYTES * 2);
    if !assistant.trim().is_empty() {
        lines.push(format!("- Assistant output: {}", assistant.trim()));
    }
    let details = truncate_bytes(
        &turn
            .latest_details
            .iter()
            .map(|(_, line)| line.as_str())
            .collect::<Vec<_>>()
            .join("\n"),
        MAX_LATEST_ITEM_BYTES * 2,
    );
    if !details.trim().is_empty() {
        lines.push(format!(
            "- Latest-turn tool/status/error details:\n{}",
            details.trim()
        ));
    }
    truncate_bytes(&lines.join("\n"), max_bytes)
}

/// The streamed item or whole block an answer row belongs to.
pub(super) fn answer_item(event: &HistoryEvent) -> Option<&str> {
    event
        .metadata
        .get("merge_key")
        .and_then(serde_json::Value::as_str)
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

pub(super) fn single_line(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The start and end of `text` in at most `max_bytes`, on one line.
pub(super) fn excerpt(text: &str, max_bytes: usize) -> String {
    let text = single_line(text);
    if text.len() <= max_bytes {
        return text;
    }
    let half = max_bytes.saturating_sub(5) / 2;
    let mut head = half;
    while !text.is_char_boundary(head) {
        head -= 1;
    }
    let mut tail = text.len() - half;
    while !text.is_char_boundary(tail) {
        tail += 1;
    }
    format!("{} ... {}", &text[..head], &text[tail..])
}

/// At most `max_bytes`, marker included.
pub(super) fn truncate_bytes(text: &str, max_bytes: usize) -> String {
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
            {
                let mut notice =
                    error_event(6, "session", "agent", "run-1", "advanced queued backlog");
                notice.kind = HistoryEventKind::Notice;
                notice
            },
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

    #[test]
    fn claude_text_blocks_separated_by_thinking_stay_apart() {
        let mut thinking = item_event(3, "claude:run-1:reasoning", "thinking");
        thinking.kind = HistoryEventKind::ProviderReasoning;
        let events = vec![
            user_event(1, "session", "agent", "inspect"),
            item_event(2, "claude:run-1:assistant", "Checking."),
            thinking,
            item_event(4, "claude:run-1:assistant", "The parser is half migrated."),
        ];
        let handoff = build_agent_context_handoff(&events, MAX_HANDOFF_BYTES).unwrap();
        assert!(
            handoff.contains("Checking.\nThe parser is half migrated."),
            "{handoff}"
        );
    }

    #[test]
    fn the_packet_leads_with_the_brief_and_the_kernel_facts() {
        let mut events = vec![
            user_event(
                1,
                "session",
                "agent",
                "Remember the codename amber-kestrel.",
            ),
            output_event(2, "session", "agent", "run-1", "OK"),
            HistoryEvent::transcript(
                3,
                &SessionHistoryEntry::provider_output(
                    "session",
                    "run-1",
                    Some("agent"),
                    TerminalOutputKind::ProviderTool,
                    None,
                    r#"{"id":"exec-1","tool":"bash","status":"completed","input":{"command":"cargo test"},"raw":"exit_code: 0"}"#,
                ),
                HistoryEventTurnContext::default(),
            ),
        ];
        for index in 0..200u64 {
            events.push(user_event(
                index * 2 + 4,
                "session",
                "agent",
                &format!("filler {index} {}", "f".repeat(800)),
            ));
            events.push(output_event(
                index * 2 + 5,
                "session",
                "agent",
                "run-1",
                &"a".repeat(800),
            ));
        }
        let mut conversation = AgentConversation::from_events(&events);
        conversation.brief = Some("## Goal\nShip amber-kestrel.\n## Next Steps\nTest.".to_string());

        let packet = conversation.render(90_000).unwrap();
        let brief = packet.find(BRIEF_HEADER).expect("brief");
        let facts = packet.find(FACTS_HEADER).expect("facts");
        let prior = packet.find(PRIOR_TURNS_HEADER).expect("prior turns");
        assert!(brief < facts && facts < prior, "{packet}");
        assert!(packet.contains("- `cargo test` -> exit 0"));
        assert!(packet.contains("earlier turns omitted"));
        assert!(packet.len() <= 90_000);

        // A small room keeps the brief and the latest request before anything else.
        let small = conversation.render(2_000).unwrap();
        assert!(small.len() <= 2_000, "{}", small.len());
        assert!(small.contains("Ship amber-kestrel."), "{small}");
        assert!(small.contains("filler 199"), "{small}");
    }

    #[test]
    fn the_turns_before_the_latest_keep_their_answers_verbatim_longer() {
        let mut events = Vec::new();
        for index in 0..6u64 {
            events.push(user_event(
                index * 2 + 1,
                "session",
                "agent",
                &format!("question {index}"),
            ));
            events.push(output_event(
                index * 2 + 2,
                "session",
                "agent",
                "run-1",
                &format!("answer {index} {} end-{index}", "w".repeat(2_000)),
            ));
        }
        let packet = build_agent_context_handoff(&events, MAX_HANDOFF_BYTES).unwrap();
        assert!(
            packet.contains("end-4") && packet.contains("end-2"),
            "{packet}"
        );
        assert!(!packet.contains("end-1"), "{packet}");
    }

    #[test]
    fn the_latest_tool_call_reads_as_one_line_with_its_result() {
        let events = vec![
            user_event(1, "session", "agent", "Multiply."),
            tool_event(
                2,
                "session",
                "agent",
                "run-1",
                r#"{"id":"call-1","tool":"bash","status":"running","input":{"command":"python3 -c 'print(7*6)'"}}"#,
            ),
            tool_event(
                3,
                "session",
                "agent",
                "run-1",
                r#"{"id":"call-1","tool":"bash","status":"completed","input":{"command":"python3 -c 'print(7*6)'"},"output":"42","raw":"exit_code: 0"}"#,
            ),
        ];

        let handoff = build_agent_context_handoff(&events, MAX_HANDOFF_BYTES).unwrap();

        assert!(
            handoff.contains("tool: bash python3 -c 'print(7*6)' -> exit 0: 42"),
            "{handoff}"
        );
        let latest = handoff.split(LATEST_TURN_HEADER).nth(1).unwrap();
        assert_eq!(latest.matches("print(7*6)").count(), 1, "{handoff}");
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
