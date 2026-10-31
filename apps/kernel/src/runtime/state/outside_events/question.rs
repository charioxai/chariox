//! The decision as an outside holder is shown it.
use crate::local::{EventQuestion, EventQuestionChoice, EventQuestionTextReply};
use crate::session::RuntimeInteraction;

const MESSAGE_BYTES: usize = 8 * 1024;
const CHOICES: usize = 8;
const LABEL_CHARS: usize = 120;

/// Only decisions a holder may answer reach this: no reply texts, passkey
/// flags or secret inputs are in the result.
pub(super) fn event_question(interaction: &RuntimeInteraction) -> EventQuestion {
    EventQuestion {
        title: interaction.title().map(str::to_owned),
        message: bounded(interaction.message(), MESSAGE_BYTES),
        choices: interaction
            .choices()
            .iter()
            .take(CHOICES)
            .map(|choice| EventQuestionChoice {
                id: choice.id().into(),
                label: choice.label().chars().take(LABEL_CHARS).collect(),
                style: choice.style(),
            })
            .collect(),
        text_reply: interaction
            .custom_choice()
            .map(|custom| EventQuestionTextReply {
                id: custom.id().into(),
                label: custom.label().chars().take(LABEL_CHARS).collect(),
                placeholder: custom.placeholder().map(str::to_owned),
                min_length: custom.min_length(),
                max_length: custom.max_length(),
            }),
        expires_at_ms: interaction.timeout_sec().map(|seconds| {
            interaction
                .requested_at_ms()
                .saturating_add(seconds.saturating_mul(1000))
        }),
        default_on_timeout: interaction.default_on_timeout().map(str::to_owned),
    }
}

fn bounded(text: &str, limit: usize) -> String {
    let end = (0..=limit.min(text.len()))
        .rev()
        .find(|&end| text.is_char_boundary(end))
        .unwrap_or(0);
    text[..end].into()
}
