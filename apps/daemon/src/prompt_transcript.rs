use crate::session::PromptAttachment;

pub fn render_prompt_transcript(prompt: &str, attachments: &[PromptAttachment]) -> String {
    let text = prompt.trim_end_matches('\n');
    let _ = attachments;
    if text.is_empty() {
        String::new()
    } else {
        format!("{text}\n")
    }
}
