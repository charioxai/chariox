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
    // MP-11 review1016: never submit private evidence to a durable provider session.
    // OpenCode's current utility endpoint creates persistent sessions. Until an
    // official nonpersistent seam is available, reject before any provider I/O.
    if envelope.private_assessment_evidence.is_some()
        && !matches!(run.adapter_key(), "codex" | "claude")
    {
        return Err(DaemonError::ProviderProtocol {
            provider_run_id: run.id().to_string(),
            operation: "assessment_session_persistence_unavailable",
            message: "provider has no nonpersistent private assessment path".into(),
        });
    }
    let scratch = AssessmentScratch::new()?;
    let mut assessment = prepare_assessment_run(run, scratch.0.clone());
    attach_assessment_context(&mut assessment, &scratch.0, &envelope.hidden_system_context)?;
    let timeout = std::time::Duration::from_millis(
        crate::durable_state::agent_lifecycle::goals::CHECK_BOUND_MS,
    );
    let policy = ProviderUtilityExecutionPolicy::MetadataOnlyDiscovery;
    let private_prompt = envelope.private_assessment_prompt();
    let input = private_prompt
        .as_ref()
        .map_or(envelope.visible_user_prompt.as_str(), |p| p.as_str());
    let output = if assessment.adapter_key() == "claude" {
        super::command_execution::execute_utility_command(
            registry, assessment, envelope, timeout, policy, None, None,
        )?
    } else {
        crate::provider::run_blocking_provider_utility_prompt(
            &assessment,
            input,
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

// MP-08 / MP-10 / MP-11: metadata discovery removes the work run's hook bridge.
// Give the fresh tool-free Claude child only this explicit kernel context, through
// the official file option; neither project hooks nor private values enter argv.
fn attach_assessment_context(
    assessment: &mut RuntimeProviderRun,
    directory: &std::path::Path,
    context: &str,
) -> Result<(), DaemonError> {
    if assessment.adapter_key() == "claude" && !context.trim().is_empty() {
        let path = directory.join("system-context.txt");
        crate::config::write_private_file(&path, context.as_bytes()).map_err(|_| {
            DaemonError::LocalTransport {
                operation: "completion assessment",
                message: "private assessment context unavailable".into(),
            }
        })?;
        assessment.append_claude_utility_system_prompt_file(&path);
    }
    Ok(())
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
        let _ = std::fs::remove_file(self.0.join("system-context.txt"));
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
    fn review1016_assessment_never_inherits_native_build_yolo_authority() {
        for provider in ["opencode", "claude", "codex"] {
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
            if provider == "claude" {
                assert!(
                    assessment
                        .pty_args()
                        .iter()
                        .any(|a| a == "--no-session-persistence"),
                    "MP-11 review1016: private assessor must never write a Claude transcript"
                );
            }
            if provider == "opencode" {
                let mut envelope = PromptEnvelope::new(
                    "public check",
                    "",
                    vec![],
                    crate::prompt_assembly::PromptManifest::default(),
                );
                envelope.assessment_only = true;
                envelope.private_assessment_evidence =
                    crate::prompt_assembly::AssessmentEvidence::new(
                        "SYNTHETIC_PRIVATE_RESULT".into(),
                    );
                let error = execute_assessment(
                    &ProviderRunRuntimeRegistry::default(),
                    &run,
                    envelope,
                    "check",
                )
                .unwrap_err();
                assert!(
                    matches!(
                        error,
                        DaemonError::ProviderProtocol {
                            operation: "assessment_session_persistence_unavailable",
                            ..
                        }
                    ),
                    "MP-11: reject before contacting a durable provider session: {error}"
                );
            }
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

    #[test]
    fn assessment_keeps_hidden_instructions_in_a_private_system_file() {
        let request =
            LaunchProviderRequest::new("room", "claude", "claude", "default", "claude-haiku-5-5");
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

        let scratch = AssessmentScratch::new().unwrap();
        let mut assessment = prepare_assessment_run(&run, scratch.0.clone());
        let context = "<user-notes>QUOTED_ASSESSMENT: COMMENT_ASSESSMENT</user-notes>";
        attach_assessment_context(&mut assessment, &scratch.0, context).unwrap();
        let args = assessment.pty_args();
        let index = args
            .iter()
            .position(|arg| arg == "--append-system-prompt-file")
            .expect("fresh Claude child needs the explicit hidden context");
        let path = std::path::PathBuf::from(&args[index + 1]);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), context);
        assert!(!args.join(" ").contains("QUOTED_ASSESSMENT"));
        assert!(assessment.mcp_servers().is_empty());
        assert!(assessment.runtime_mcp_server_url().is_none());
        assert!(assessment.resume_state().is_empty());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
        drop(scratch);
        assert!(!path.exists());
    }
}
