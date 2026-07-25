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
    if prompt.workflow_node_run_id().is_none() {
        return prompt.prompt().to_string();
    }
    let visible = prompt
        .prompt()
        .trim()
        .replace("<endpoint-prompt>", "")
        .replace("</endpoint-prompt>", "")
        .replace("<workflow-handoff-payloads>", "")
        .replace("</workflow-handoff-payloads>", "");
    crate::prompt_assembly::unescape_prompt_component_delimiters(visible.trim())
}

#[cfg(test)]
mod tests {
    use super::workflow_prompt_history_text;
    use crate::session::{PromptQueueItem, PromptStatus};

    #[test]
    fn workflow_history_keeps_only_unwrapped_visible_prompt() {
        let prompt = PromptQueueItem::new(
            "prompt-1",
            "workflow-run:run-1",
            "agent-1",
            "<endpoint-prompt>\nRead package.json.\n</endpoint-prompt>",
            PromptStatus::Queued,
        )
        .with_hidden_system_context(
            "<workflow-runtime-instructions>\nHIDDEN_TOKEN\n</workflow-runtime-instructions>",
        )
        .with_workflow_context("run-1", "node-run-1");

        let history = workflow_prompt_history_text(&prompt);

        assert_eq!(history, "Read package.json.");
        assert!(!history.contains("endpoint-prompt"));
        assert!(!history.contains("HIDDEN_TOKEN"));
    }

    #[test]
    fn workflow_history_preserves_visible_handoff_payload() {
        let prompt = PromptQueueItem::new(
            "prompt-1",
            "workflow-run:run-1",
            "agent-1",
            "<workflow-handoff-payloads>\n[{\"message\":\"continue\"}]\n</workflow-handoff-payloads>",
            PromptStatus::Queued,
        )
        .with_hidden_system_context("<node-level-prompt>hidden</node-level-prompt>")
        .with_workflow_context("run-1", "node-run-1");

        assert_eq!(
            workflow_prompt_history_text(&prompt),
            "[{\"message\":\"continue\"}]"
        );
    }
}
