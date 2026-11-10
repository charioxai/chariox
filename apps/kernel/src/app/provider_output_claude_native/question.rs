//! MP-08 / MP-09 / MP-10 / MP-11: Claude's `AskUserQuestion` call, shown as the
//! kernel-owned question card, answered through the waiting permission hook.
use std::time::{Duration, Instant};

use serde_json::Value;

use super::claude_card_withdrawn;
use crate::provider::ProviderNativeInteractionBridge;
use crate::session::{
    NativeInteractionOrigin, RuntimeInteraction, RuntimeInteractionChoice,
    RuntimeInteractionCustomChoice, RuntimeInteractionKind, RuntimeInteractionLevel,
};

pub(super) const CLAUDE_QUESTION_TOOL: &str = "AskUserQuestion";
const OTHER_ANSWER_ID: &str = "other";
const OTHER_ANSWER_MAX_CHARS: usize = 4000;
const DONE_ID: &str = "done";
/// A card open for less than this could not be read and answered.
const SHORTEST_CARD: Duration = Duration::from_secs(5);

pub(super) struct ClaudeQuestion {
    pub(super) question: String,
    header: Option<String>,
    options: Vec<ClaudeQuestionOption>,
    multi_select: bool,
}

struct ClaudeQuestionOption {
    label: String,
    description: Option<String>,
}

/// The questions of an `AskUserQuestion` permission request, none when the
/// event is another tool's or carries no question Claude could show.
pub(super) fn claude_questions(event: &Value) -> Option<Vec<ClaudeQuestion>> {
    if event.get("tool_name").and_then(Value::as_str) != Some(CLAUDE_QUESTION_TOOL) {
        return None;
    }
    let questions = event
        .get("tool_input")?
        .get("questions")?
        .as_array()?
        .iter()
        .filter_map(|question| {
            let text = non_empty(question.get("question")?)?;
            let options = question
                .get("options")?
                .as_array()?
                .iter()
                .filter_map(|option| {
                    Some(ClaudeQuestionOption {
                        label: non_empty(option.get("label")?)?,
                        description: option.get("description").and_then(non_empty),
                    })
                })
                .collect::<Vec<_>>();
            Some(ClaudeQuestion {
                question: text,
                header: question.get("header").and_then(non_empty),
                options,
                multi_select: question
                    .get("multiSelect")
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
            })
        })
        .collect::<Vec<_>>();
    (!questions.is_empty()).then_some(questions)
}

fn non_empty(value: &Value) -> Option<String> {
    Some(value.as_str()?.trim())
        .filter(|text| !text.is_empty())
        .map(str::to_string)
}

/// The hook waits for all the answers of one call at once, so the cards of a
/// batch share one deadline inside that wait: each gets only what is left.
pub(super) struct QuestionBatchDeadline(Instant);

impl QuestionBatchDeadline {
    pub(super) fn starting_now() -> Self {
        Self(
            Instant::now()
                + Duration::from_secs(crate::provider::CLAUDE_NATIVE_PERMISSION_TIMEOUT_SECS),
        )
    }

    /// How long the next card may stay open, in whole seconds rounded up;
    /// none when too little is left.
    fn card_timeout(&self, now: Instant) -> Option<Duration> {
        let left = self.0.saturating_duration_since(now);
        let left = Duration::from_secs(left.as_secs() + u64::from(left.subsec_nanos() > 0));
        (left >= SHORTEST_CARD).then_some(left)
    }
}

pub(super) enum QuestionOutcome {
    Answered(String),
    /// Nobody answered before the deadline.
    Unanswered,
    /// The card closed because its turn was stopped.
    Withdrawn,
}

/// Asks one question as a card. A multi-select question is asked again, with
/// the options not chosen yet and a Done choice, until the person is done.
pub(super) fn ask_claude_question(
    bridge: &dyn ProviderNativeInteractionBridge,
    session_id: &str,
    card_id: &str,
    agent_id: &str,
    origin: Option<NativeInteractionOrigin>,
    question: &ClaudeQuestion,
    deadline: &QuestionBatchDeadline,
) -> Result<QuestionOutcome, crate::error::DaemonError> {
    let mut chosen = Vec::<String>::new();
    for round in 0.. {
        let Some(timeout) = deadline.card_timeout(Instant::now()) else {
            return Ok(QuestionOutcome::Unanswered);
        };
        let interaction = claude_question_interaction(
            format!("{card_id}-{round}"),
            agent_id,
            question,
            &chosen,
            timeout.as_secs(),
        )
        .with_native_origin(origin.clone());
        let closes_at = Instant::now() + timeout;
        let resolution = bridge.request_blocking(session_id, interaction)?;
        let reply = resolution
            .reply
            .as_deref()
            .map(str::trim)
            .filter(|reply| !reply.is_empty());
        match (resolution.choice_id.as_deref(), reply) {
            (Some(DONE_ID), _) => break,
            (Some(choice_id), Some(reply)) => {
                chosen.push(reply.to_string());
                if !question.multi_select
                    || choice_id == OTHER_ANSWER_ID
                    || chosen.len() == question.options.len()
                {
                    break;
                }
            }
            _ if claude_card_withdrawn(&resolution, closes_at) => {
                return Ok(QuestionOutcome::Withdrawn)
            }
            _ => return Ok(QuestionOutcome::Unanswered),
        }
    }
    Ok(QuestionOutcome::Answered(chosen.join(", ")))
}

fn claude_question_interaction(
    id: String,
    agent_id: &str,
    question: &ClaudeQuestion,
    chosen: &[String],
    timeout_sec: u64,
) -> RuntimeInteraction {
    let mut message = question.question.clone();
    for option in &question.options {
        if let Some(description) = &option.description {
            message.push_str(&format!("\n\n{}: {description}", option.label));
        }
    }
    if !chosen.is_empty() {
        message.push_str(&format!("\n\nChosen: {}", chosen.join(", ")));
    }
    let mut choices = question
        .options
        .iter()
        .enumerate()
        .filter(|(_, option)| !chosen.contains(&option.label))
        .map(|(index, option)| {
            RuntimeInteractionChoice::new(
                format!("option-{index}"),
                option.label.clone(),
                option.label.clone(),
                None,
            )
        })
        .collect::<Vec<_>>();
    if !chosen.is_empty() {
        choices.push(RuntimeInteractionChoice::new(
            DONE_ID.to_string(),
            "Done".to_string(),
            "Done".to_string(),
            None,
        ));
    }
    RuntimeInteraction::new(
        id,
        agent_id.to_string(),
        RuntimeInteractionKind::Choice,
        RuntimeInteractionLevel::Info,
        question.header.clone(),
        message,
        choices,
        Some(RuntimeInteractionCustomChoice::new(
            OTHER_ANSWER_ID,
            "Other",
            None,
            Some(1),
            Some(OTHER_ANSWER_MAX_CHARS),
        )),
        Some(timeout_sec),
        None,
    )
}

/// The call's own input with the person's answers added, which is how Claude
/// takes an answer without showing its menu.
pub(super) fn claude_question_updated_input(event: &Value, answers: &[(String, String)]) -> Value {
    let mut input = event
        .get("tool_input")
        .cloned()
        .unwrap_or_else(|| serde_json::json!({}));
    input["answers"] = Value::Object(
        answers
            .iter()
            .map(|(question, answer)| (question.clone(), Value::String(answer.clone())))
            .collect(),
    );
    input
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_card_gets_only_what_is_left_of_the_batch_and_none_near_its_end() {
        let now = Instant::now();
        let deadline = QuestionBatchDeadline(now + Duration::from_secs(12));
        assert_eq!(deadline.card_timeout(now), Some(Duration::from_secs(12)));
        assert_eq!(
            deadline.card_timeout(now + Duration::from_secs(8)),
            None,
            "four seconds is too short to read and answer a card"
        );
        assert_eq!(
            deadline.card_timeout(now + Duration::from_millis(1_500)),
            Some(Duration::from_secs(11))
        );
        assert_eq!(deadline.card_timeout(now + Duration::from_secs(20)), None);
    }
}
