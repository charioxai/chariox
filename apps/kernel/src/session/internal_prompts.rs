//! MP-08 / MP-10 / MP-11: kernel instructions are agent-only. Their replies
//! remain ordinary provider output; only the instruction echo is hidden.
pub(crate) fn is_internal_prompt_echo_id(prompt_id: &str) -> bool {
    crate::durable_state::agent_lifecycle::goals::is_completion_check_prompt_id(prompt_id)
        || crate::durable_state::agent_lifecycle::goals::is_correction_prompt_id(prompt_id)
        || is_agent_inbox_prompt_id(prompt_id)
}

pub(crate) fn is_agent_inbox_prompt_id(prompt_id: &str) -> bool {
    prompt_id.starts_with("agent-event-")
}
