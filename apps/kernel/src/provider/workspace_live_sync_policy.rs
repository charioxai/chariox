pub(crate) const WORKSPACE_LIVE_SYNC_INSTRUCTIONS_SOURCE_PATH: &str =
    "apps/kernel/src/provider/workspace_live_sync_instructions.md";

pub(crate) const NATIVE_TUI_HIDDEN_INSTRUCTIONS_START: &str =
    "<<<ARROBA_NATIVE_TUI_HIDDEN_INSTRUCTIONS>>>";
pub(crate) const NATIVE_TUI_HIDDEN_INSTRUCTIONS_END: &str =
    "<<<END_ARROBA_NATIVE_TUI_HIDDEN_INSTRUCTIONS>>>";

pub(crate) fn native_tui_hidden_instructions_block(instructions: &str) -> String {
    format!(
        "{NATIVE_TUI_HIDDEN_INSTRUCTIONS_START}\n{}\n{NATIVE_TUI_HIDDEN_INSTRUCTIONS_END}",
        instructions.trim()
    )
}
