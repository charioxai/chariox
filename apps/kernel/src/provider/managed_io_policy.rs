use crate::provider::RuntimeProviderRun;

pub(crate) const MANAGED_IO_INSTRUCTIONS_SOURCE_PATH: &str =
    "apps/kernel/src/provider/managed_io_instructions.md";

const MANAGED_IO_INSTRUCTIONS: &str = include_str!("managed_io_instructions.md");
const NATIVE_PERMISSION_INSTRUCTIONS: &str = include_str!("native_permission_instructions.md");
const RUNTIME_INSTRUCTIONS: &str = include_str!("runtime_instructions.md");
pub(crate) const NATIVE_TUI_HIDDEN_INSTRUCTIONS_START: &str =
    "<<<ARROBA_NATIVE_TUI_HIDDEN_INSTRUCTIONS>>>";
pub(crate) const NATIVE_TUI_HIDDEN_INSTRUCTIONS_END: &str =
    "<<<END_ARROBA_NATIVE_TUI_HIDDEN_INSTRUCTIONS>>>";

pub(crate) fn runtime_instructions() -> &'static str {
    RUNTIME_INSTRUCTIONS.trim()
}

pub(crate) fn apply_runtime_instructions(prompt: &str) -> String {
    format!("{}\n\n{}", runtime_instructions(), prompt)
}

pub(crate) fn managed_io_instructions() -> &'static str {
    MANAGED_IO_INSTRUCTIONS.trim()
}

pub(crate) fn native_permission_instructions() -> &'static str {
    NATIVE_PERMISSION_INSTRUCTIONS.trim()
}

pub(crate) fn native_tui_hidden_instructions_block(instructions: &str) -> String {
    format!(
        "{NATIVE_TUI_HIDDEN_INSTRUCTIONS_START}\n{}\n{NATIVE_TUI_HIDDEN_INSTRUCTIONS_END}",
        instructions.trim()
    )
}

fn execution_path_instructions_for_run(run: &RuntimeProviderRun) -> &'static str {
    if run.requires_managed_io() {
        managed_io_instructions()
    } else {
        native_permission_instructions()
    }
}

pub(crate) fn apply_execution_path_instructions(prompt: &str, run: &RuntimeProviderRun) -> String {
    let path_instructions = execution_path_instructions_for_run(run);
    format!("{}\n\n{}", path_instructions, prompt)
}

pub(crate) fn apply_structured_prompt_instructions(
    prompt: &str,
    run: &RuntimeProviderRun,
) -> String {
    if !run.client_interface().is_arroba() {
        let hidden_instructions = format!(
            "{}\n\n{}",
            execution_path_instructions_for_run(run),
            runtime_instructions()
        );
        return format!(
            "{}\n\n{prompt}",
            native_tui_hidden_instructions_block(&hidden_instructions)
        );
    }

    let prompt = apply_runtime_instructions(prompt);
    apply_execution_path_instructions(&prompt, run)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider::{
        AgentEndpointMode, LaunchProviderRequest, ProviderClientInterface, ProviderLaunchResult,
    };
    use std::collections::BTreeMap;

    fn launch_result() -> ProviderLaunchResult {
        ProviderLaunchResult {
            endpoint_mode: AgentEndpointMode::Managed,
            process_label: "codex".to_string(),
            pty_target: None,
            pty_program: None,
            pty_args: Vec::new(),
            pty_env: BTreeMap::new(),
            pty_env_remove: Vec::new(),
            working_directory: None,
            structured_endpoint: Some("ws://127.0.0.1:43112".to_string()),
        }
    }

    #[test]
    fn managed_io_instructions_are_loaded_from_policy_file() {
        let instructions = managed_io_instructions();

        assert!(instructions.contains("arroba_read_artifact"));
        assert!(instructions.contains("arroba_write_artifact"));
        assert!(instructions.contains("arroba_edit_artifact"));
        assert!(instructions.contains("mcp__arroba__patch_artifact"));
        assert!(instructions.contains("Avoid any bare or OpenCode-prefixed `apply_patch`"));
        assert!(!instructions.ends_with('\n'));
    }

    #[test]
    fn runtime_instructions_are_loaded_from_policy_file() {
        let instructions = runtime_instructions();

        assert!(instructions.contains("list_capabilities"));
        assert!(instructions.contains("request_capability"));
        assert!(instructions.contains("request_popup"));
        assert!(!instructions.contains("native provider actions"));
        assert!(!instructions.ends_with('\n'));
    }

    #[test]
    fn native_permission_instructions_are_loaded_from_policy_file() {
        let instructions = native_permission_instructions();

        assert!(instructions.contains("native approval request"));
        assert!(instructions.contains("request_popup"));
        assert!(!instructions.ends_with('\n'));
    }

    #[test]
    fn execution_path_instructions_select_native_block_for_unmanaged_runs() {
        let unmanaged_request =
            LaunchProviderRequest::new("session", "codex", "codex", "default", "gpt-5.4");
        let unmanaged =
            RuntimeProviderRun::new("provider-run-1", &unmanaged_request, launch_result());
        let prompt = apply_execution_path_instructions("hello", &unmanaged);

        assert!(prompt.starts_with(native_permission_instructions()));
        assert!(prompt.ends_with("\n\nhello"));
    }

    #[test]
    fn execution_path_instructions_select_managed_block_for_required_runs() {
        let managed_request =
            LaunchProviderRequest::new("session", "agent", "codex", "default", "gpt-5.4")
                .with_managed_io_required();
        let managed = RuntimeProviderRun::new("provider-run-2", &managed_request, launch_result());
        let prompt = apply_execution_path_instructions("hello", &managed);

        assert!(prompt.starts_with(managed_io_instructions()));
        assert!(prompt.ends_with("\n\nhello"));
    }

    #[test]
    fn runtime_instructions_are_always_added() {
        let prompt = apply_runtime_instructions("hello");

        assert!(prompt.starts_with(runtime_instructions()));
        assert!(prompt.ends_with("\n\nhello"));
    }

    #[test]
    fn structured_prompt_instructions_are_added_for_arroba_runs() {
        let request = LaunchProviderRequest::new("session", "codex", "codex", "default", "gpt-5.4");
        let run = RuntimeProviderRun::new("provider-run-3", &request, launch_result());
        let prompt = apply_structured_prompt_instructions("hello", &run);

        assert!(prompt.starts_with(native_permission_instructions()));
        assert!(prompt.contains(runtime_instructions()));
        assert!(prompt.ends_with("\n\nhello"));
    }

    #[test]
    fn structured_prompt_instructions_for_native_tui_runs_are_marked_hidden() {
        let request = LaunchProviderRequest::new("session", "codex", "codex", "default", "gpt-5.4")
            .with_client_interface(ProviderClientInterface::NativeTui);
        let run = RuntimeProviderRun::new("provider-run-4", &request, launch_result());
        let prompt = apply_structured_prompt_instructions("hello", &run);

        assert!(prompt.starts_with(NATIVE_TUI_HIDDEN_INSTRUCTIONS_START));
        assert!(prompt.contains(runtime_instructions()));
        assert!(prompt.contains(native_permission_instructions()));
        assert!(prompt.ends_with(&format!("{NATIVE_TUI_HIDDEN_INSTRUCTIONS_END}\n\nhello")));
    }
}
