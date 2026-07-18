use crate::session::{PromptAttachment, PromptQueueItem};

pub fn render_prompt_transcript(prompt: &str, attachments: &[PromptAttachment]) -> String {
    let text = prompt.trim_end_matches('\n');
    let _ = attachments;
    if text.is_empty() {
        String::new()
    } else {
        format!("{text}\n")
    }
}

pub(crate) fn workflow_prompt_history_text(prompt: &PromptQueueItem) -> String {
    if prompt.workflow_node_run_id().is_none() || prompt.hidden_system_context().trim().is_empty() {
        return prompt.prompt().to_string();
    }
    match (
        prompt.prompt().trim(),
        prompt.hidden_system_context().trim(),
    ) {
        ("", hidden) => hidden.to_string(),
        (visible, hidden) => format!("{visible}\n\n{hidden}"),
    }
}
