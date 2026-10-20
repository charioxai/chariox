//! MP-08 / MP-10: kernel instructions and their control receipts are
//! agent-only; no client projection (live echo, history, outline) shows them.
use super::PromptOrigin;

const GOAL_CHECK_PROMPT_PREFIX: &str = "goal-check-";

/// Same prompt-id rule as the main-line predicate: the goal loop's completion
/// assessment is addressed by `goal-check-{task}-{revision}-{count}`.
pub(crate) fn is_internal_prompt_echo_id(prompt_id: &str) -> bool {
    prompt_id.starts_with(GOAL_CHECK_PROMPT_PREFIX)
}

/// True for the check turn's instruction, reply and tool/status records.
pub(crate) fn is_internal_prompt_turn(
    origin: Option<PromptOrigin>,
    prompt_id: Option<&str>,
) -> bool {
    origin == Some(PromptOrigin::CompletionCheck)
        || prompt_id.is_some_and(is_internal_prompt_echo_id)
}
