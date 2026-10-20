//! MP-08 / MP-10 / MP-11: kernel instructions are agent-only. Their replies
//! remain ordinary provider output; only the instruction echo is hidden.
pub(crate) fn is_internal_prompt_echo_id(prompt_id: &str) -> bool {
    crate::durable_state::agent_lifecycle::is_correction_prompt_id(prompt_id)
        || crate::durable_state::agent_lifecycle::is_owner_resume_prompt_id(prompt_id)
        || is_agent_inbox_prompt_id(prompt_id)
        || prompt_id.starts_with(GOAL_CHECK_PROMPT_PREFIX)
}

// The goal loop's completion assessment embeds the whole public evidence
// bundle; the user must see the agent's work, never that instruction.
const GOAL_CHECK_PROMPT_PREFIX: &str = "goal-check-";

pub(crate) fn is_agent_inbox_prompt_id(prompt_id: &str) -> bool {
    prompt_id.starts_with("agent-event-")
}
