//! MP-08 / MP-10 / MP-11: kernel instructions are agent-only. A completion
//! check's whole turn (instruction, receipt reply, tool/status records) stays
//! agent-only; replies to the other kernel instructions remain ordinary
//! provider output and only their instruction echo is hidden.
use super::PromptOrigin;

pub(crate) const WORKFLOW_OUTPUT_FOLLOW_UP_PREFIX: &str = "workflow-output-follow-up-";

pub(crate) fn is_internal_prompt_echo_id(prompt_id: &str) -> bool {
    crate::durable_state::agent_lifecycle::goals::is_completion_check_prompt_id(prompt_id)
        || crate::durable_state::agent_lifecycle::goals::is_correction_prompt_id(prompt_id)
        || crate::durable_state::agent_lifecycle::goals::is_owner_resume_prompt_id(prompt_id)
        || crate::durable_state::agent_lifecycle::goals::is_goal_continuation_prompt_id(prompt_id)
        || is_agent_inbox_prompt_id(prompt_id)
        || prompt_id.starts_with(WORKFLOW_OUTPUT_FOLLOW_UP_PREFIX)
}

pub(crate) fn is_agent_inbox_prompt_id(prompt_id: &str) -> bool {
    prompt_id.starts_with("agent-event-")
}

/// True for the completion check turn's instruction, reply and tool/status
/// records, addressed by origin or by `goal-check-{task}-{revision}-{count}`.
pub(crate) fn is_internal_prompt_turn(
    origin: Option<PromptOrigin>,
    prompt_id: Option<&str>,
) -> bool {
    origin == Some(PromptOrigin::CompletionCheck)
        || prompt_id.is_some_and(
            crate::durable_state::agent_lifecycle::goals::is_completion_check_prompt_id,
        )
}
