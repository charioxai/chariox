//! The handoff brief: a model-written summary of an agent's conversation that
//! a new provider session receives after a provider switch. Each switch updates
//! the stored brief with only the history after its watermark (PRESERVE / ADD /
//! UPDATE), and history larger than one utility call is read oldest first.

use crate::history::{HistoryEvent, HistoryEventKind};
use crate::runtime::agent_utility_executor::AgentUtilityPromptParts;

use super::builder::{excerpt, single_line, truncate_bytes};
use super::facts::ToolCall;

pub(super) const MAX_BRIEF_BYTES: usize = 16_000;
/// The transcript one utility call reads, about 50k tokens.
const CHUNK_BYTES: usize = 160_000;
const USER_BYTES: usize = 2_000;
const ANSWER_BYTES: usize = 2_500;
const TOOL_OUTPUT_BYTES: usize = 360;
const ERROR_BYTES: usize = 600;
const BRIEF_SECTIONS: &str = "## Goal\n## Constraints & Preferences\n## Progress\n### Done\n### In Progress\n### Blocked\n## Key Decisions\n## Next Steps\n## Critical Context";
const BRIEF_SYSTEM_CONTEXT: &str = "You write handoff briefs. A coding agent's conversation is moving to a different AI model, and the brief is how that model learns what happened. Work only from the supplied text; you have no tools, and your own session's environment is not part of the conversation.";

/// Compact transcript lines, oldest first, in chunks one utility call reads.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct TranscriptChunk {
    pub(super) text: String,
    pub(super) through_sequence: u64,
}

/// The history after a brief's watermark as transcript chunks. Tool calls keep
/// their command and outcome with a short output excerpt; the full output stays
/// in history for recall. The last chunk covers every event read, so the
/// watermark passes events that add no transcript line.
pub(super) fn transcript_chunks(events: &[HistoryEvent]) -> Vec<TranscriptChunk> {
    let Some(last_sequence) = events.iter().map(|event| event.sequence).max() else {
        return Vec::new();
    };
    let mut chunks = Vec::new();
    let mut current = String::new();
    for (sequence, line) in transcript_lines(events) {
        if !current.is_empty() && current.len() + line.len() + 1 > CHUNK_BYTES {
            chunks.push(TranscriptChunk {
                text: std::mem::take(&mut current),
                through_sequence: sequence.saturating_sub(1),
            });
        }
        current.push_str(&line);
        current.push('\n');
    }
    if !current.is_empty() || chunks.is_empty() {
        chunks.push(TranscriptChunk {
            text: current,
            through_sequence: last_sequence,
        });
    } else if let Some(last) = chunks.last_mut() {
        last.through_sequence = last_sequence;
    }
    chunks
}

fn transcript_lines(events: &[HistoryEvent]) -> Vec<(u64, String)> {
    let mut sorted = events.iter().collect::<Vec<_>>();
    sorted.sort_by_key(|event| event.sequence);
    let mut lines: Vec<(u64, String)> = Vec::new();
    let mut answer: Option<(u64, String)> = None;
    let mut tool_lines = std::collections::BTreeMap::<String, usize>::new();
    for event in sorted {
        let content = event.content.as_deref().unwrap_or_default();
        if event.kind == HistoryEventKind::ProviderOutput {
            let (_, text) = answer.get_or_insert((event.sequence, String::new()));
            text.push_str(content);
            continue;
        }
        if let Some((sequence, text)) = answer.take() {
            push_line(
                &mut lines,
                sequence,
                "Assistant",
                &excerpt(text.trim(), ANSWER_BYTES),
            );
        }
        match event.kind {
            HistoryEventKind::UserPrompt => {
                tool_lines.clear();
                push_line(
                    &mut lines,
                    event.sequence,
                    "User",
                    &excerpt(content.trim(), USER_BYTES),
                );
            }
            HistoryEventKind::ProviderTool => {
                let Some(call) = ToolCall::parse(content) else {
                    continue;
                };
                let line = format!("[Tool] {}", call.transcript_line(TOOL_OUTPUT_BYTES));
                // A tool call's updates share its id; keep its latest state.
                match call.id.as_ref().and_then(|id| tool_lines.get(id)) {
                    Some(&index) => lines[index].1 = line,
                    None => {
                        if let Some(id) = call.id {
                            tool_lines.insert(id, lines.len());
                        }
                        lines.push((event.sequence, line));
                    }
                }
            }
            HistoryEventKind::ProviderError => {
                push_line(
                    &mut lines,
                    event.sequence,
                    "Error",
                    &truncate_bytes(content.trim(), ERROR_BYTES),
                );
            }
            HistoryEventKind::GitCommitDetected => {
                push_line(
                    &mut lines,
                    event.sequence,
                    "Git commit",
                    &truncate_bytes(content.trim(), ERROR_BYTES),
                );
            }
            _ => {}
        }
    }
    if let Some((sequence, text)) = answer {
        push_line(
            &mut lines,
            sequence,
            "Assistant",
            &excerpt(text.trim(), ANSWER_BYTES),
        );
    }
    lines
}

fn push_line(lines: &mut Vec<(u64, String)>, sequence: u64, label: &str, text: &str) {
    if !text.is_empty() {
        lines.push((sequence, format!("[{label}] {}", single_line(text))));
    }
}

/// The utility prompt that folds `chunk` into `previous`.
pub(super) fn brief_prompt(
    previous: Option<&str>,
    chunk: &TranscriptChunk,
    part: usize,
    parts: usize,
) -> AgentUtilityPromptParts {
    let previous = previous.unwrap_or("(none yet)");
    let visible_user_prompt = format!(
        "Update the handoff brief with the new part of the conversation transcript below.\n\n\
         Rules:\n\
         - PRESERVE every item of the previous brief that is still valid.\n\
         - ADD the new goals, constraints, preferences, progress, decisions, facts and next steps the transcript shows.\n\
         - UPDATE items whose state changed: move finished work to Done, replace superseded decisions, refresh Next Steps. Drop only what the transcript makes obsolete.\n\
         - Copy exact values verbatim: names, codes, numbers, ports, paths, commands, URLs, error messages, and everything the user asked to remember.\n\
         - Describe what was done and decided, not raw tool output.\n\
         - Describe only the conversation in the transcript. Never mention your own session: its working directory, mode, permissions or plan files.\n\
         - At most 900 words. Output only the brief, in exactly this Markdown structure:\n\n\
         {BRIEF_SECTIONS}\n\n\
         <previous_brief>\n{previous}\n</previous_brief>\n\n\
         <transcript part=\"{part} of {parts}\">\n{}</transcript>",
        chunk.text,
        part = part + 1,
    );
    AgentUtilityPromptParts {
        visible_user_prompt,
        hidden_system_context: BRIEF_SYSTEM_CONTEXT.to_string(),
    }
}

/// The brief in a utility answer, or `None` when it lacks the brief's frame.
pub(super) fn parse_brief(output: &str) -> Option<String> {
    let start = output.find("## Goal")?;
    let brief = output[start..]
        .trim_end()
        .trim_end_matches("```")
        .trim_end();
    brief
        .contains("## Next Steps")
        .then(|| truncate_bytes(brief, MAX_BRIEF_BYTES))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::history::{HistoryEventTurnContext, SessionHistoryEntry};
    use crate::terminal::TerminalOutputKind;

    fn user(sequence: u64, text: &str) -> HistoryEvent {
        HistoryEvent::transcript(
            sequence,
            &SessionHistoryEntry::user_prompt("session", "attachment", "agent", text),
            HistoryEventTurnContext::default(),
        )
    }

    fn output(
        sequence: u64,
        kind: TerminalOutputKind,
        merge_key: Option<&str>,
        text: &str,
    ) -> HistoryEvent {
        HistoryEvent::transcript(
            sequence,
            &SessionHistoryEntry::provider_output(
                "session",
                "run-1",
                Some("agent"),
                kind,
                merge_key.map(str::to_string),
                text,
            ),
            HistoryEventTurnContext::default(),
        )
    }

    #[test]
    fn transcript_keeps_prompts_answers_and_tool_outcomes_without_bulk_output() {
        let bulk = format!("head {} tail", "x".repeat(50_000));
        let events = vec![
            user(1, "Remember the codename amber-kestrel."),
            output(2, TerminalOutputKind::ProviderOutput, None, "O"),
            output(3, TerminalOutputKind::ProviderOutput, None, "K"),
            user(4, "Read the log."),
            output(5, TerminalOutputKind::ProviderTool, None, r#"{"id":"call-1","tool":"bash","status":"running","input":{"command":"cat big.log"}}"#),
            output(
                6,
                TerminalOutputKind::ProviderTool,
                None,
                &serde_json::json!({"id":"call-1","tool":"bash","status":"completed","input":{"command":"cat big.log"},"output":bulk,"raw":"exit_code: 0"}).to_string(),
            ),
            output(7, TerminalOutputKind::ProviderOutput, None, "The log is fine."),
        ];

        let chunks = transcript_chunks(&events);

        assert_eq!(chunks.len(), 1);
        let text = &chunks[0].text;
        assert!(text.contains("[User] Remember the codename amber-kestrel."));
        assert!(text.contains("[Assistant] OK"));
        assert_eq!(text.matches("cat big.log").count(), 1, "{text}");
        assert!(text.contains("exit 0"));
        assert!(text.contains("head") && text.contains("tail"));
        assert!(text.len() < 2_000, "{} bytes", text.len());
        assert_eq!(chunks[0].through_sequence, 7);
    }

    #[test]
    fn long_history_is_read_oldest_first_in_bounded_chunks() {
        let mut events = Vec::new();
        for index in 0..400u64 {
            events.push(user(
                index * 2 + 1,
                &format!("prompt {index} {}", "p".repeat(3_000)),
            ));
            events.push(output(
                index * 2 + 2,
                TerminalOutputKind::ProviderOutput,
                None,
                &"a".repeat(2_000),
            ));
        }
        events.push(output(
            900,
            TerminalOutputKind::ProviderStatus,
            None,
            "completed",
        ));

        let chunks = transcript_chunks(&events);

        assert!(chunks.len() > 1);
        assert!(chunks.iter().all(|chunk| chunk.text.len() <= CHUNK_BYTES));
        assert!(chunks[0].text.starts_with("[User] prompt 0 "));
        assert!(chunks
            .windows(2)
            .all(|pair| pair[0].through_sequence < pair[1].through_sequence));
        let prompts = chunks
            .iter()
            .flat_map(|chunk| chunk.text.lines())
            .filter_map(|line| {
                line.strip_prefix("[User] prompt ")?
                    .split(' ')
                    .next()?
                    .parse::<u64>()
                    .ok()
            })
            .collect::<Vec<_>>();
        assert_eq!(prompts, (0..400).collect::<Vec<_>>());
        assert_eq!(chunks.last().unwrap().through_sequence, 900);
    }

    #[test]
    fn brief_prompt_carries_the_previous_brief_and_update_rules() {
        let chunk = TranscriptChunk {
            text: "[User] Use port 41000.\n".to_string(),
            through_sequence: 9,
        };
        let prompt = brief_prompt(Some("## Goal\nShip it."), &chunk, 1, 3);

        let text = &prompt.visible_user_prompt;
        assert!(text.contains("PRESERVE") && text.contains("ADD") && text.contains("UPDATE"));
        assert!(text.contains("<previous_brief>\n## Goal\nShip it.\n</previous_brief>"));
        assert!(
            text.contains("<transcript part=\"2 of 3\">\n[User] Use port 41000.\n</transcript>")
        );
        assert!(text.contains("## Critical Context"));
    }

    #[test]
    fn a_brief_needs_its_frame_and_stays_bounded() {
        assert_eq!(parse_brief("I cannot help."), None);
        assert_eq!(parse_brief("## Goal\nShip it."), None);
        let brief =
            parse_brief("Here you go:\n```markdown\n## Goal\nShip it.\n## Next Steps\nTest.\n```")
                .unwrap();
        assert_eq!(brief, "## Goal\nShip it.\n## Next Steps\nTest.");
        let long = format!("## Goal\n{}\n## Next Steps\n", "g".repeat(40_000));
        assert!(parse_brief(&long).unwrap().len() <= MAX_BRIEF_BYTES);
    }
}
