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
    let (visible, _) = split_workflow_prompt_for_hidden_context(prompt.prompt().to_string());
    workflow_visible_prompt_text(&visible)
}

pub(crate) fn workflow_visible_prompt_text(prompt: &str) -> String {
    let visible = prompt
        .trim()
        .replace("<endpoint-prompt>", "")
        .replace("</endpoint-prompt>", "")
        .replace("<workflow-handoff-payloads>", "")
        .replace("</workflow-handoff-payloads>", "");
    crate::prompt_assembly::unescape_prompt_component_delimiters(visible.trim())
}

pub(crate) fn split_workflow_prompt_for_hidden_context(prompt: String) -> (String, String) {
    const HIDDEN_MARKERS: &[&str] = &[
        crate::provider::NATIVE_TUI_HIDDEN_INSTRUCTIONS_START,
        "<workflow-level-prompt>",
        "<node-level-prompt>",
        "<workflow-runtime-instructions>",
        "<system-node-level-prompt>",
        "Workflow-level prompt:\n",
    ];
    if let Some(index) = HIDDEN_MARKERS
        .iter()
        .filter_map(|marker| prompt.find(marker))
        .min()
    {
        let visible = prompt[..index].to_string();
        let hidden = prompt[index..].to_string();
        return (visible, strip_native_hidden_markers(hidden));
    }
    (prompt, String::new())
}

fn strip_native_hidden_markers(value: String) -> String {
    value
        .replace(crate::provider::NATIVE_TUI_HIDDEN_INSTRUCTIONS_START, "")
        .replace(crate::provider::NATIVE_TUI_HIDDEN_INSTRUCTIONS_END, "")
        .trim()
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::{split_workflow_prompt_for_hidden_context, workflow_prompt_history_text};
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

    #[test]
    fn workflow_history_defensively_hides_legacy_assembled_prompt_text() {
        let prompt = PromptQueueItem::new(
            "prompt-1",
            "workflow-run:run-1",
            "agent-1",
            "<endpoint-prompt>\nRead package.json.\n</endpoint-prompt>\n\n\
             <node-level-prompt>\nHIDDEN_TOKEN\n</node-level-prompt>",
            PromptStatus::Queued,
        )
        .with_workflow_context("run-1", "node-run-1");

        assert_eq!(workflow_prompt_history_text(&prompt), "Read package.json.");
    }

    #[test]
    fn workflow_prompt_split_keeps_visible_and_private_components_separate() {
        let prompt = "<endpoint-prompt>\nvisible\n</endpoint-prompt>\n\n\
                      <node-level-prompt>\nhidden\n</node-level-prompt>";

        let (visible, hidden) = split_workflow_prompt_for_hidden_context(prompt.to_string());

        assert_eq!(
            visible.trim(),
            "<endpoint-prompt>\nvisible\n</endpoint-prompt>"
        );
        assert_eq!(hidden, "<node-level-prompt>\nhidden\n</node-level-prompt>");
    }
}
