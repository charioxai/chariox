use crate::provider::RuntimeProviderRun;

pub(crate) const MANAGED_IO_INSTRUCTIONS_SOURCE_PATH: &str =
    "apps/kernel/src/provider/managed_io_instructions.md";

const MANAGED_IO_INSTRUCTIONS: &str = include_str!("managed_io_instructions.md");
const RUNTIME_INSTRUCTIONS: &str = include_str!("runtime_instructions.md");

pub(crate) fn runtime_instructions() -> &'static str {
    RUNTIME_INSTRUCTIONS.trim()
}

pub(crate) fn apply_runtime_instructions(prompt: &str) -> String {
    format!("{}\n\n{}", runtime_instructions(), prompt)
}

pub(crate) fn managed_io_instructions() -> &'static str {
    MANAGED_IO_INSTRUCTIONS.trim()
}

pub(crate) fn apply_managed_io_instructions(prompt: &str, run: &RuntimeProviderRun) -> String {
    if !run.requires_managed_io() {
        return prompt.to_string();
    }
    format!("{}\n\n{}", managed_io_instructions(), prompt)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider::{AgentEndpointMode, LaunchProviderRequest, ProviderLaunchResult};
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

        assert!(instructions.contains("arroba.read_artifact"));
        assert!(instructions.contains("arroba.write_artifact"));
        assert!(instructions.contains("arroba.edit_artifact"));
        assert!(!instructions.ends_with('\n'));
    }

    #[test]
    fn runtime_instructions_are_loaded_from_policy_file() {
        let instructions = runtime_instructions();

        assert!(instructions.contains("list_capabilities"));
        assert!(instructions.contains("request_capability"));
        assert!(instructions.contains("request_popup"));
        assert!(!instructions.ends_with('\n'));
    }

    #[test]
    fn managed_io_instructions_are_added_only_for_required_runs() {
        let unmanaged_request =
            LaunchProviderRequest::new("session", "codex", "codex", "default", "gpt-5.4");
        let unmanaged =
            RuntimeProviderRun::new("provider-run-1", &unmanaged_request, launch_result());
        assert_eq!(apply_managed_io_instructions("hello", &unmanaged), "hello");

        let managed_request =
            LaunchProviderRequest::new("session", "agent", "codex", "default", "gpt-5.4")
                .with_managed_io_required();
        let managed = RuntimeProviderRun::new("provider-run-2", &managed_request, launch_result());
        let prompt = apply_managed_io_instructions("hello", &managed);

        assert!(prompt.starts_with(managed_io_instructions()));
        assert!(prompt.ends_with("\n\nhello"));
    }

    #[test]
    fn runtime_instructions_are_always_added() {
        let prompt = apply_runtime_instructions("hello");

        assert!(prompt.starts_with(runtime_instructions()));
        assert!(prompt.ends_with("\n\nhello"));
    }
}
