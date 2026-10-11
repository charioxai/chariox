//! MP-08 / MP-09 / MP-10 / MP-11 #971: tool-free native assessment policy.
use super::runtime_slots::ProviderRunRuntimeRegistry;
use crate::error::DaemonError;
use crate::prompt_assembly::PromptEnvelope;
use crate::provider::RuntimeProviderRun;
use crate::provider::{AgentExecutionMode, AgentPermissionLevel, ProviderUtilityExecutionPolicy};

pub(in crate::provider) fn prepare_assessment_run(
    run: &RuntimeProviderRun,
    directory: std::path::PathBuf,
) -> RuntimeProviderRun {
    let mut assessment = run.clone();
    assessment.set_execution_config(AgentExecutionMode::Plan, AgentPermissionLevel::Required);
    assessment.set_metadata_only_discovery(directory);
    assessment
}

pub(super) fn execute_assessment(
    registry: &ProviderRunRuntimeRegistry,
    run: &RuntimeProviderRun,
    envelope: PromptEnvelope,
    prompt_id: &str,
) -> Result<super::ProviderPromptSubmitAcknowledgement, DaemonError> {
    let scratch = AssessmentScratch::new()?;
    let assessment = prepare_assessment_run(run, scratch.0.clone());
    let timeout = std::time::Duration::from_millis(
        crate::durable_state::agent_lifecycle::goals::CHECK_BOUND_MS,
    );
    let policy = ProviderUtilityExecutionPolicy::MetadataOnlyDiscovery;
    let output = if assessment.adapter_key() == "claude" {
        super::command_execution::execute_utility_command(
            registry, assessment, envelope, timeout, policy,
        )?
    } else {
        crate::provider::run_blocking_provider_utility_prompt(
            &assessment,
            &envelope.visible_user_prompt,
            &envelope.hidden_system_context,
            timeout,
            "completion assessment",
            policy,
        )?
    };
    registry.store_assessment_output(
        run.id(),
        crate::provider::ProviderPromptSignalBatch {
            chunks: vec![crate::provider::ProviderPromptChunk {
                kind: crate::terminal::TerminalOutputKind::ProviderOutput,
                merge_key: Some(format!("completion-assessment:{prompt_id}")),
                bytes: output.into_bytes(),
            }],
            prompt_completed: true,
            ..Default::default()
        },
    );
    Ok(super::ProviderPromptSubmitAcknowledgement {
        resume_state: run.resume_state().clone(),
    })
}

// The kernel's configured TMPDIR is private runtime scratch, outside the Project.
// Only this newly created, empty directory is removed; profiles stay product-owned.
struct AssessmentScratch(std::path::PathBuf);
impl AssessmentScratch {
    fn new() -> Result<Self, DaemonError> {
        let path = std::env::temp_dir().join(format!(
            "chariox-completion-assessment-{:032x}",
            rand::random::<u128>()
        ));
        let created = {
            #[cfg(unix)]
            {
                use std::os::unix::fs::DirBuilderExt;
                std::fs::DirBuilder::new().mode(0o700).create(&path)
            }
            #[cfg(not(unix))]
            {
                std::fs::create_dir(&path)
            }
        };
        created.map_err(|_| DaemonError::LocalTransport {
            operation: "completion assessment",
            message: "neutral assessment scratch unavailable".into(),
        })?;
        Ok(Self(path))
    }
}
impl Drop for AssessmentScratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir(&self.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider::{
        AgentEndpointMode, AgentExecutionMode, AgentPermissionLevel, LaunchProviderRequest,
        ProviderLaunchResult,
    };

    #[test]
    fn review971_assessment_never_inherits_native_build_yolo_authority() {
        for provider in ["codex", "claude", "opencode"] {
            let request =
                LaunchProviderRequest::new("room", provider, provider, "default", "default")
                    .with_execution_mode(AgentExecutionMode::Build)
                    .with_permission_level(AgentPermissionLevel::Yolo)
                    .with_runtime_mcp_binding(crate::provider::RuntimeMcpBinding::new(
                        "http://127.0.0.1:1/mcp",
                        "synthetic-unused",
                    ))
                    .with_provider_config_override("features.shell_tool", serde_json::json!(true))
                    .with_mcp_servers(vec![serde_json::from_value(serde_json::json!({
                "name":"ordinary-tools", "transport":{"type":"stdio","command":"ordinary-unused"}
            }))
            .unwrap()]);
            let run = RuntimeProviderRun::new(
                "run",
                &request,
                ProviderLaunchResult {
                    endpoint_mode: AgentEndpointMode::Managed,
                    process_label: "ordinary".into(),
                    pty_target: None,
                    pty_program: None,
                    pty_args: vec![],
                    pty_env: Default::default(),
                    pty_env_remove: vec![],
                    working_directory: Some("/ordinary-project".into()),
                    structured_endpoint: None,
                },
            );
            let assessment = prepare_assessment_run(&run, "/neutral-assessment".into());
            assert_eq!(
                assessment.execution_mode(),
                AgentExecutionMode::Plan,
                "{provider} native work must be disabled"
            );
            assert_eq!(
                assessment.permission_level(),
                AgentPermissionLevel::Required
            );
            assert!(assessment.metadata_only_discovery());
            assert!(assessment.mcp_servers().is_empty());
            assert!(assessment.runtime_mcp_server_url().is_none());
            assert!(assessment.provider_config_overrides().is_empty());
            assert!(!run.mcp_servers().is_empty());
            assert!(run.runtime_mcp_server_url().is_some());
            assert_eq!(
                assessment.working_directory(),
                Some(&std::path::PathBuf::from("/neutral-assessment"))
            );
            assert_eq!(
                run.execution_mode(),
                AgentExecutionMode::Build,
                "ordinary authority must survive the temporary check"
            );
        }
    }
}
