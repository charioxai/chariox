//! MP-08 / MP-09 / MP-10 / MP-11: Claude's `AskUserQuestion` call, shown as the
//! kernel-owned question card, answered through the waiting permission hook.
use serde_json::Value;

use crate::provider::ProviderNativeInteractionBridge;
use crate::session::{
    NativeInteractionOrigin, RuntimeInteraction, RuntimeInteractionChoice,
    RuntimeInteractionCustomChoice, RuntimeInteractionKind, RuntimeInteractionLevel,
};

pub(super) const CLAUDE_QUESTION_TOOL: &str = "AskUserQuestion";
const OTHER_ANSWER_ID: &str = "other";
const OTHER_ANSWER_MAX_CHARS: usize = 4000;
const DONE_ID: &str = "done";

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

pub(super) enum QuestionOutcome {
    Answered(String),
    /// The card closed unanswered: a question has no deadline, so only a
    /// stopped turn closes it.
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
) -> Result<QuestionOutcome, crate::error::DaemonError> {
    let mut chosen = Vec::<String>::new();
    for round in 0.. {
        let interaction =
            claude_question_interaction(format!("{card_id}-{round}"), agent_id, question, &chosen)
                .with_native_origin(origin.clone());
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
            _ => return Ok(QuestionOutcome::Withdrawn),
        }
    }
    Ok(QuestionOutcome::Answered(chosen.join(", ")))
}

fn claude_question_interaction(
    id: String,
    agent_id: &str,
    question: &ClaudeQuestion,
    chosen: &[String],
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
        None,
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
