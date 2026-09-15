use serde::Deserialize;

use crate::error::DaemonError;
use crate::local::{
    ProjectEnvironmentDefinition, ProjectEnvironmentDefinitionOrigin,
    ProjectEnvironmentSetupUtilityInput,
};

#[derive(Debug)]
pub(crate) struct ProjectEnvironmentSetupUtilityPrompt {
    pub(crate) visible_user_prompt: String,
    pub(crate) hidden_system_context: String,
}

#[derive(Debug, Deserialize)]
struct ProjectEnvironmentSetupUtilityOutput {
    definition: ProjectEnvironmentDefinition,
}

pub(crate) fn project_environment_setup_utility_prompt_assembly(
    input: &ProjectEnvironmentSetupUtilityInput,
) -> Result<ProjectEnvironmentSetupUtilityPrompt, DaemonError> {
    let schema = project_environment_setup_utility_schema();
    let input_json =
        serde_json::to_string_pretty(input).map_err(|error| DaemonError::LocalTransport {
            operation: "run project environment setup utility",
            message: format!("could not encode project environment setup input: {error}"),
        })?;
    Ok(ProjectEnvironmentSetupUtilityPrompt {
        visible_user_prompt: format!(
            "Prepare the selected project environment in the actual current worker.\n\n\
The kernel target identity and platform are fixed by this request; do not substitute another\n\
machine, host, worktree, or platform. Run the requested setup in the worker before returning.\n\
If an existing definition is present, reproduce that definition and use its Dockerfile,\n\
devcontainer, setup script, or commands as appropriate. If it is absent, inspect the project and\n\
derive a repeatable definition. Package installs, compilers/system tools, and native dependencies\n\
must be represented as repeatable setup steps. Do not copy binaries from a host or Mac, read host\n\
credential stores, install a provider SDK, or replace the home kernel.\n\n\
For a file-backed definition, include a content-only input attestation for the recipe source and\n\
any relevant lockfiles, using workspace-relative paths and sha256 digests. The kernel will verify\n\
those files on the target before declaring readiness. Never put file contents, credentials, tokens,\n\
private keys, or other secrets in the definition or its input attestations.\n\n\
For a Rust project, ensure the definition accounts for Cargo/rustc and native build dependencies.\n\
Run bounded project checks in the worker (for example `timeout 180s cargo check --workspace --locked`\n\
when applicable); the kernel will independently rerun the returned validation commands.\n\n\
Request:\n{input_json}\n\n\
Return JSON only. Return the exact definition used or discovered; do not claim readiness, include\n\
command output, or add prose.\n\n\
JSON Schema:\n{schema}",
            input_json = input_json,
            schema = serde_json::to_string_pretty(&schema).unwrap_or_else(|_| "{}".to_string()),
        ),
        hidden_system_context: crate::prompt_assembly::PromptAssemblyService::from_env()?
            .assemble_hidden_context_only(&["utility/project-environment-setup"])?.0,
    })
}

pub(crate) fn parse_project_environment_setup_utility_output(
    output: &str,
    expected: Option<&ProjectEnvironmentDefinition>,
    target_platform: &str,
) -> Result<ProjectEnvironmentDefinition, DaemonError> {
    parse_project_environment_setup_utility_output_with_policy(
        output,
        expected,
        target_platform,
        false,
    )
}

pub(crate) fn parse_project_environment_setup_utility_output_for_repair(
    output: &str,
    expected: Option<&ProjectEnvironmentDefinition>,
    target_platform: &str,
) -> Result<ProjectEnvironmentDefinition, DaemonError> {
    parse_project_environment_setup_utility_output_with_policy(
        output,
        expected,
        target_platform,
        true,
    )
}

fn parse_project_environment_setup_utility_output_with_policy(
    output: &str,
    expected: Option<&ProjectEnvironmentDefinition>,
    target_platform: &str,
    allow_definition_revision: bool,
) -> Result<ProjectEnvironmentDefinition, DaemonError> {
    let json = extract_json_object(output).ok_or_else(|| DaemonError::LocalTransport {
        operation: "run project environment setup utility",
        message: "project environment setup utility did not return a JSON object".to_string(),
    })?;
    // Keep the rejection diagnostic stable when the schema validator reports
    // only its generic `minItems` failure. An empty list is still rejected
    // before parsing or accepting the utility definition.
    if serde_json::from_str::<serde_json::Value>(json)
        .ok()
        .and_then(|value| {
            value
                .pointer("/definition/validation_commands")
                .and_then(serde_json::Value::as_array)
                .map(Vec::is_empty)
        })
        == Some(true)
    {
        return Err(DaemonError::LocalTransport {
            operation: "run project environment setup utility",
            message: "environment definition must include at least one validation command"
                .to_string(),
        });
    }
    let schema = project_environment_setup_utility_schema();
    crate::transport::runtime_tools::validate_json_output_schema(
        "project_environment_setup_utility_output",
        &schema,
        json,
    )
    .map_err(|warning| DaemonError::LocalTransport {
        operation: "run project environment setup utility",
        message: format!("project environment setup utility output failed validation: {warning}"),
    })?;
    let parsed =
        serde_json::from_str::<ProjectEnvironmentSetupUtilityOutput>(json).map_err(|error| {
            DaemonError::LocalTransport {
                operation: "run project environment setup utility",
                message: format!(
                    "project environment setup utility output was not parseable: {error}"
                ),
            }
        })?;
    parsed
        .definition
        .validate()
        .map_err(|message| DaemonError::LocalTransport {
            operation: "run project environment setup utility",
            message,
        })?;
    if parsed.definition.target_platform != target_platform {
        return Err(DaemonError::LocalTransport {
            operation: "run project environment setup utility",
            message: "utility returned a definition for a different target platform".to_string(),
        });
    }
    let definition = match expected {
        Some(expected) => {
            let returned = parsed.definition.with_origin(expected.origin);
            if !allow_definition_revision && returned != *expected {
                return Err(DaemonError::LocalTransport {
                    operation: "run project environment setup utility",
                    message: "utility changed the selected environment definition".to_string(),
                });
            }
            returned
        }
        None => parsed
            .definition
            .with_origin(ProjectEnvironmentDefinitionOrigin::UtilityGenerated),
    };
    if definition.validation_commands.is_empty() {
        return Err(DaemonError::LocalTransport {
            operation: "run project environment setup utility",
            message: "environment definition must include at least one validation command"
                .to_string(),
        });
    }
    Ok(definition)
}

fn project_environment_setup_utility_schema() -> serde_json::Value {
    serde_json::json!({
        "type": "object",
        "required": ["definition"],
        "additionalProperties": false,
        "properties": {
            "definition": {
                "type": "object",
                "required": [
                    "schema_version", "origin", "source", "target_platform",
                    "source_path", "setup_steps", "validation_commands"
                ],
                "additionalProperties": false,
                "properties": {
                    "schema_version": {"type": "integer", "const": 1},
                    "origin": {"type": "string", "enum": ["user_authored", "utility_generated"]},
                    "source": {"type": "string", "enum": ["commands", "dockerfile", "devcontainer", "setup_script"]},
                    "target_platform": {"type": "string", "minLength": 1, "maxLength": 128},
                    "source_path": {"type": ["string", "null"]},
                    "inputs": {
                        "type": "array",
                        "maxItems": 64,
                        "items": {
                            "type": "object",
                            "required": ["kind", "path", "sha256"],
                            "additionalProperties": false,
                            "properties": {
                                "kind": {"type": "string", "enum": ["recipe", "lockfile"]},
                                "path": {"type": "string", "minLength": 1, "maxLength": 512},
                                "sha256": {"type": "string", "pattern": "^sha256:[0-9a-fA-F]{64}$"}
                            }
                        }
                    },
                    "setup_steps": {
                        "type": "array",
                        "maxItems": 64,
                        "items": {
                            "type": "object",
                            "required": ["kind", "command"],
                            "additionalProperties": false,
                            "properties": {
                                "kind": {
                                    "type": "string",
                                    "enum": [
                                        "package",
                                        "system_tool",
                                        "compiler",
                                        "native_dependency",
                                        "command"
                                    ]
                                },
                                "command": {
                                    "type": "string",
                                    "minLength": 1,
                                    "maxLength": 8192
                                }
                            }
                        }
                    },
                    "validation_commands": {"type": "array", "minItems": 1, "maxItems": 32, "items": {"type": "string", "minLength": 1, "maxLength": 8192}}
                }
            }
        }
    })
}

fn extract_json_object(output: &str) -> Option<&str> {
    let trimmed = output.trim();
    if trimmed.starts_with('{') && trimmed.ends_with('}') {
        return Some(trimmed);
    }
    let start = trimmed.find('{')?;
    let end = trimmed.rfind('}')?;
    (start < end).then_some(&trimmed[start..=end])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::local::{
        ProjectEnvironmentDefinitionSource, ProjectEnvironmentSetupStep,
        ProjectEnvironmentSetupStepKind,
    };

    fn definition() -> ProjectEnvironmentDefinition {
        ProjectEnvironmentDefinition {
            schema_version: 1,
            origin: ProjectEnvironmentDefinitionOrigin::UtilityGenerated,
            source: ProjectEnvironmentDefinitionSource::Commands,
            target_platform: "linux-x86_64".to_string(),
            source_path: None,
            inputs: Vec::new(),
            setup_steps: vec![ProjectEnvironmentSetupStep {
                kind: ProjectEnvironmentSetupStepKind::Compiler,
                command: "rustup toolchain install stable".to_string(),
            }],
            validation_commands: vec!["cargo check --workspace --locked".to_string()],
        }
    }

    #[test]
    fn parser_rejects_definition_for_another_platform() {
        let output = serde_json::json!({"definition": definition()});
        let error = parse_project_environment_setup_utility_output(
            &output.to_string(),
            None,
            "linux-aarch64",
        )
        .expect_err("platform mismatch should fail");
        assert!(error.to_string().contains("different target platform"));
    }

    #[test]
    fn parser_normalizes_generated_origin_and_requires_validation() {
        let mut definition = definition();
        definition.origin = ProjectEnvironmentDefinitionOrigin::UserAuthored;
        let output = serde_json::json!({"definition": definition});
        let parsed = parse_project_environment_setup_utility_output(
            &output.to_string(),
            None,
            "linux-x86_64",
        )
        .expect("valid utility definition should parse");
        assert_eq!(
            parsed.origin,
            ProjectEnvironmentDefinitionOrigin::UtilityGenerated
        );

        let mut without_validation = definition;
        without_validation.validation_commands.clear();
        let error = parse_project_environment_setup_utility_output(
            &serde_json::json!({"definition": without_validation}).to_string(),
            None,
            "linux-x86_64",
        )
        .expect_err("readiness without a validation command should fail");
        assert!(error.to_string().contains("at least one validation"));
    }

    #[test]
    fn repair_parser_accepts_revised_definition_but_preserves_selected_origin() {
        let mut expected = definition();
        expected.origin = ProjectEnvironmentDefinitionOrigin::UserAuthored;
        let mut repaired = expected.clone();
        repaired.origin = ProjectEnvironmentDefinitionOrigin::UtilityGenerated;
        repaired.setup_steps[0].command =
            "rustup toolchain install stable --profile minimal".into();
        let output = serde_json::json!({"definition": repaired});

        let parsed = parse_project_environment_setup_utility_output_for_repair(
            &output.to_string(),
            Some(&expected),
            "linux-x86_64",
        )
        .expect("repair output should be allowed to revise a failed definition");
        assert_eq!(parsed.setup_steps, repaired.setup_steps);
        assert_eq!(
            parsed.origin,
            ProjectEnvironmentDefinitionOrigin::UserAuthored
        );

        let error = parse_project_environment_setup_utility_output(
            &output.to_string(),
            Some(&expected),
            "linux-x86_64",
        )
        .expect_err("ordinary utility output must retain strict selected-definition equality");
        assert!(error.to_string().contains("changed the selected"));
    }
}
