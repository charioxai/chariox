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

const MISSING_USER_INPUT_ERROR_PREFIX: &str = "project environment setup requires user input: ";
const MAX_MISSING_USER_INPUTS: usize = 16;
const MAX_MISSING_USER_INPUT_LABEL_CHARS: usize = 96;

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum ProjectEnvironmentSetupMissingUserInputKind {
    SelectedCredential,
    HostVerification,
    ProjectConfiguration,
    Toolchain,
}

impl ProjectEnvironmentSetupMissingUserInputKind {
    fn display_name(self) -> &'static str {
        match self {
            Self::SelectedCredential => "selected credential",
            Self::HostVerification => "host verification",
            Self::ProjectConfiguration => "project configuration",
            Self::Toolchain => "toolchain input",
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
struct ProjectEnvironmentSetupMissingUserInput {
    kind: ProjectEnvironmentSetupMissingUserInputKind,
    #[serde(default)]
    label: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ProjectEnvironmentSetupUtilityOutput {
    definition: ProjectEnvironmentDefinition,
    #[serde(default)]
    missing_user_inputs: Vec<ProjectEnvironmentSetupMissingUserInput>,
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
    let visible_user_prompt =
        project_environment_setup_utility_visible_prompt(&input_json, &schema);
    Ok(ProjectEnvironmentSetupUtilityPrompt {
        visible_user_prompt,
        hidden_system_context: crate::prompt_assembly::PromptAssemblyService::from_env()?
            .assemble_hidden_context_only(&["utility/project-environment-setup"])?
            .0,
    })
}

fn project_environment_setup_utility_visible_prompt(
    input_json: &str,
    schema: &serde_json::Value,
) -> String {
    format!(
        "Prepare the selected project environment in the actual current worker.\n\n\
The kernel target identity and platform are fixed by this request; do not substitute another\n\
machine, host, worktree, or platform. Run the requested setup in the worker before returning.\n\
The definition in the request is the selected environment recipe. When it is present, reproduce,\n\
apply, and verify it first; do not discover a different recipe or invoke repair while setup and\n\
validation pass. Invoke repair only after applying or validating that selected definition fails.\n\
When no definition is present, inspect project-declared setup evidence in the actual worktree,\n\
including but not limited to package manifests and lockfiles (package.json, pyproject.toml,\n\
go.mod, Cargo.toml), Makefiles/build files, Dockerfiles, devcontainers, setup and CI scripts,\n\
tool-version files, and README or contributing build instructions. There is no finite language\n\
allowlist: account for every language/toolchain and required system or native dependency that the\n\
project evidence requires. Represent each required package, compiler, system tool, native\n\
dependency, and command as a repeatable setup step with bounded validation. If project evidence\n\
requires SSH or tmux, include the appropriate openssh-client/ssh and tmux system-tool steps and\n\
validate them; do not assume they are present in the managed image.\n\n\
Prefer an existing project Dockerfile, devcontainer, setup script, lockfile, or verified\n\
environment recipe over inventing equivalent commands. Do not copy binaries from a host or Mac,\n\
read host credential stores, install a provider SDK, or replace the home kernel.\n\n\
Use only explicitly selected and already materialized credential mechanisms, such as the selected\n\
Git credential helper or an SSH agent socket and the worker's configured host verification. Never\n\
copy SSH private keys, include credential values in the definition or attestations, use\n\
ssh-keyscan, disable StrictHostKeyChecking (no, off, or accept-new), write known_hosts data, or\n\
trust an arbitrary host. If a selected credential, host verification, project configuration, or\n\
toolchain input is genuinely missing, return missing_user_inputs with only its fixed category and\n\
a short non-secret label; never return the missing value. The kernel will keep setup failed until\n\
that user input is supplied.\n\n\
For a file-backed definition, include a content-only input attestation for the recipe source and\n\
any relevant lockfiles, using workspace-relative paths and sha256 digests. The kernel will verify\n\
those files on the target before declaring readiness. Never put file contents, credentials, tokens,\n\
private keys, or other secrets in the definition or its input attestations.\n\n\
Run bounded project checks in the worker when applicable; the kernel independently reruns the\n\
returned validation commands on this same worker.\n\n\
Request:\n{input_json}\n\n\
Return JSON only. Return the exact definition used or discovered; do not claim readiness, include\n\
command output, or add prose.\n\n\
JSON Schema:\n{schema}",
        input_json = input_json,
        schema = serde_json::to_string_pretty(schema).unwrap_or_else(|_| "{}".to_string()),
    )
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
    validate_missing_user_input_report(&parsed.missing_user_inputs)?;
    if !parsed.missing_user_inputs.is_empty() {
        return Err(missing_user_input_error(&parsed.missing_user_inputs));
    }
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
    if allow_definition_revision {
        if let Some(expected) = expected {
            if !validation_commands_include_original(
                &expected.validation_commands,
                &parsed.definition.validation_commands,
            ) {
                return Err(DaemonError::LocalTransport {
                    operation: "run project environment setup utility",
                    message:
                        "repair utility definition must retain every original validation command"
                            .to_string(),
                });
            }
        }
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

fn validate_missing_user_input_report(
    missing_user_inputs: &[ProjectEnvironmentSetupMissingUserInput],
) -> Result<(), DaemonError> {
    for input in missing_user_inputs {
        let Some(label) = input.label.as_deref() else {
            continue;
        };
        let lower = label.to_ascii_lowercase();
        let safe = !label.trim().is_empty()
            && label.chars().count() <= MAX_MISSING_USER_INPUT_LABEL_CHARS
            && label.chars().all(|character| {
                character.is_ascii_alphanumeric()
                    || matches!(character, ' ' | '_' | '-' | '.' | ':')
            })
            && ![
                "private", "secret", "token", "password", "value=", "key=", "-----",
            ]
            .iter()
            .any(|marker| lower.contains(marker));
        if !safe {
            return Err(DaemonError::LocalTransport {
                operation: "run project environment setup utility",
                message: "utility returned an unsafe missing user input report".to_string(),
            });
        }
    }
    Ok(())
}

fn missing_user_input_error(
    missing_user_inputs: &[ProjectEnvironmentSetupMissingUserInput],
) -> DaemonError {
    let mut categories = Vec::new();
    for input in missing_user_inputs {
        let category = input.kind.display_name();
        if !categories.contains(&category) {
            categories.push(category);
        }
    }
    DaemonError::LocalTransport {
        operation: "run project environment setup utility",
        message: format!("{MISSING_USER_INPUT_ERROR_PREFIX}{}", categories.join(", ")),
    }
}

pub(crate) fn project_environment_setup_utility_missing_input_message(
    error: &DaemonError,
) -> Option<&str> {
    match error {
        DaemonError::LocalTransport { operation, message }
            if *operation == "run project environment setup utility"
                && message.starts_with(MISSING_USER_INPUT_ERROR_PREFIX) =>
        {
            Some(message.as_str())
        }
        _ => None,
    }
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
                                "sha256": {"type": "string", "pattern": "^sha256:[0-9a-f]{64}$"}
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
            },
            "missing_user_inputs": {
                "type": "array",
                "maxItems": MAX_MISSING_USER_INPUTS,
                "items": {
                    "type": "object",
                    "required": ["kind"],
                    "additionalProperties": false,
                    "properties": {
                        "kind": {
                            "type": "string",
                            "enum": [
                                "selected_credential",
                                "host_verification",
                                "project_configuration",
                                "toolchain"
                            ]
                        },
                        "label": {"type": "string", "minLength": 1, "maxLength": MAX_MISSING_USER_INPUT_LABEL_CHARS}
                    }
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

fn validation_commands_include_original(original: &[String], revised: &[String]) -> bool {
    let mut remaining = revised.iter().collect::<Vec<_>>();
    original.iter().all(|command| {
        let Some(index) = remaining.iter().position(|candidate| *candidate == command) else {
            return false;
        };
        remaining.remove(index);
        true
    })
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

    fn definition_with_validation_commands(commands: &[&str]) -> ProjectEnvironmentDefinition {
        let mut definition = definition();
        definition.validation_commands = commands.iter().map(|command| (*command).into()).collect();
        definition
    }

    fn heterogeneous_definition() -> ProjectEnvironmentDefinition {
        ProjectEnvironmentDefinition {
            schema_version: 1,
            origin: ProjectEnvironmentDefinitionOrigin::UtilityGenerated,
            source: ProjectEnvironmentDefinitionSource::Commands,
            target_platform: "linux-x86_64".to_string(),
            source_path: None,
            inputs: Vec::new(),
            setup_steps: vec![
                ProjectEnvironmentSetupStep {
                    kind: ProjectEnvironmentSetupStepKind::Package,
                    command: "npm ci --ignore-scripts".to_string(),
                },
                ProjectEnvironmentSetupStep {
                    kind: ProjectEnvironmentSetupStepKind::Compiler,
                    command: "python3 -m venv .venv".to_string(),
                },
                ProjectEnvironmentSetupStep {
                    kind: ProjectEnvironmentSetupStepKind::SystemTool,
                    command: "apt-get install -y openssh-client tmux".to_string(),
                },
                ProjectEnvironmentSetupStep {
                    kind: ProjectEnvironmentSetupStepKind::NativeDependency,
                    command: "pkg-config --version".to_string(),
                },
                ProjectEnvironmentSetupStep {
                    kind: ProjectEnvironmentSetupStepKind::Command,
                    command: "node --version && python3 --version && go version".to_string(),
                },
            ],
            validation_commands: vec![
                "node --version".to_string(),
                "python3 --version".to_string(),
                "go version".to_string(),
                "command -v ssh".to_string(),
                "command -v tmux".to_string(),
            ],
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

    #[test]
    fn repair_parser_rejects_dropped_original_validation_commands() {
        let expected = definition_with_validation_commands(&[
            "cargo check --workspace --locked",
            "cargo test --workspace --locked",
        ]);
        let mut repaired = expected.clone();
        repaired.setup_steps[0].command =
            "rustup toolchain install stable --profile minimal".into();
        repaired.validation_commands.pop();

        let error = parse_project_environment_setup_utility_output_for_repair(
            &serde_json::json!({"definition": repaired}).to_string(),
            Some(&expected),
            "linux-x86_64",
        )
        .expect_err("repair must not drop a stored validation command");
        assert!(error
            .to_string()
            .contains("must retain every original validation command"));
    }

    #[test]
    fn repair_parser_rejects_replaced_original_validation_commands() {
        let expected = definition_with_validation_commands(&[
            "cargo check --workspace --locked",
            "cargo test --workspace --locked",
        ]);
        let mut repaired = expected.clone();
        repaired.setup_steps[0].command =
            "rustup toolchain install stable --profile minimal".into();
        repaired.validation_commands[1] = "true".into();

        let error = parse_project_environment_setup_utility_output_for_repair(
            &serde_json::json!({"definition": repaired}).to_string(),
            Some(&expected),
            "linux-x86_64",
        )
        .expect_err("repair must not replace a stored validation command");
        assert!(error
            .to_string()
            .contains("must retain every original validation command"));
    }

    #[test]
    fn repair_parser_accepts_expansion_preserving_all_original_validation_commands() {
        let expected = definition_with_validation_commands(&[
            "cargo check --workspace --locked",
            "cargo test --workspace --locked",
        ]);
        let mut repaired = expected.clone();
        repaired.setup_steps[0].command =
            "rustup toolchain install stable --profile minimal".into();
        repaired
            .validation_commands
            .push("cargo fmt --all -- --check".into());

        let parsed = parse_project_environment_setup_utility_output_for_repair(
            &serde_json::json!({"definition": repaired.clone()}).to_string(),
            Some(&expected),
            "linux-x86_64",
        )
        .expect("repair may add validation commands while retaining the stored checks");
        assert_eq!(parsed.setup_steps, repaired.setup_steps);
        assert_eq!(parsed.validation_commands, repaired.validation_commands);
        assert_eq!(
            parsed.origin,
            ProjectEnvironmentDefinitionOrigin::UtilityGenerated
        );

        let error = parse_project_environment_setup_utility_output(
            &serde_json::json!({"definition": repaired}).to_string(),
            Some(&expected),
            "linux-x86_64",
        )
        .expect_err("ordinary parser must remain strict about definition revisions");
        assert!(error.to_string().contains("changed the selected"));
    }

    #[test]
    fn parser_accepts_heterogeneous_project_requirements_without_language_allowlist() {
        let definition = heterogeneous_definition();
        let output = serde_json::json!({"definition": definition});
        let parsed = parse_project_environment_setup_utility_output(
            &output.to_string(),
            None,
            "linux-x86_64",
        )
        .expect("heterogeneous project requirements should be representable");

        assert_eq!(parsed.setup_steps.len(), 5);
        assert!(parsed
            .setup_steps
            .iter()
            .any(|step| step.kind == ProjectEnvironmentSetupStepKind::Package));
        assert!(parsed
            .setup_steps
            .iter()
            .any(|step| step.kind == ProjectEnvironmentSetupStepKind::Compiler));
        assert!(parsed
            .setup_steps
            .iter()
            .any(|step| step.kind == ProjectEnvironmentSetupStepKind::SystemTool));
        assert!(parsed
            .setup_steps
            .iter()
            .any(|step| step.kind == ProjectEnvironmentSetupStepKind::NativeDependency));
        assert!(parsed
            .setup_steps
            .iter()
            .any(|step| step.kind == ProjectEnvironmentSetupStepKind::Command));
        let system_tool_step = parsed
            .setup_steps
            .iter()
            .find(|step| step.kind == ProjectEnvironmentSetupStepKind::SystemTool)
            .expect("the heterogeneous recipe should retain its system-tool step");
        assert!(system_tool_step.command.contains("openssh-client"));
        assert!(system_tool_step.command.contains("tmux"));
        assert!(parsed
            .validation_commands
            .iter()
            .any(|command| command == "command -v ssh"));
        assert!(parsed
            .validation_commands
            .iter()
            .any(|command| command == "command -v tmux"));
    }

    #[test]
    fn prompt_requires_project_evidence_and_safe_selected_credentials() {
        let prompt = project_environment_setup_utility_visible_prompt(
            "{\"definition\":null}",
            &project_environment_setup_utility_schema(),
        );
        for fragment in [
            "selected environment recipe",
            "invoke repair only after",
            "package.json",
            "pyproject.toml",
            "go.mod",
            "Cargo.toml",
            "including but not limited to",
            "no finite language",
            "openssh-client/ssh",
            "tmux",
            "Git credential helper",
            "SSH agent socket",
            "missing_user_inputs",
            "ssh-keyscan",
            "StrictHostKeyChecking",
            "configured host verification",
        ] {
            assert!(prompt.contains(fragment), "prompt omitted `{fragment}`");
        }
    }

    #[test]
    fn parser_reports_missing_user_input_categories_without_echoing_labels() {
        let output = serde_json::json!({
            "definition": definition(),
            "missing_user_inputs": [
                {"kind": "selected_credential", "label": "GitHub credential"},
                {"kind": "host_verification"}
            ]
        });
        let error = parse_project_environment_setup_utility_output(
            &output.to_string(),
            None,
            "linux-x86_64",
        )
        .expect_err("missing user input must keep setup from becoming ready");
        let message = error.to_string();
        assert!(message.contains("selected credential"));
        assert!(message.contains("host verification"));
        assert!(!message.contains("GitHub credential"));
        assert_eq!(
            project_environment_setup_utility_missing_input_message(&error),
            Some("project environment setup requires user input: selected credential, host verification"),
            "the state layer should be able to recognize only the safe category diagnostic"
        );
    }

    #[test]
    fn parser_rejects_secret_bearing_missing_input_labels_without_echoing_them() {
        let output = serde_json::json!({
            "definition": definition(),
            "missing_user_inputs": [
                {"kind": "selected_credential", "label": "token=super-secret"}
            ]
        });
        let error = parse_project_environment_setup_utility_output(
            &output.to_string(),
            None,
            "linux-x86_64",
        )
        .expect_err("secret-bearing labels must not be accepted");
        let message = error.to_string();
        assert!(message.contains("unsafe missing user input report"));
        assert!(!message.contains("super-secret"));
    }
}
