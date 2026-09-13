use std::path::Path;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub const PROJECT_ENVIRONMENT_DEFINITION_SCHEMA_VERSION: u32 = 1;

const MAX_TARGET_PLATFORM_CHARS: usize = 128;
const MAX_SETUP_STEPS: usize = 64;
const MAX_VALIDATION_COMMANDS: usize = 32;
const MAX_COMMAND_CHARS: usize = 8_192;
const MAX_DEFINITION_BYTES: usize = 128 * 1024;

#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProjectEnvironmentDefinitionOrigin {
    UserAuthored,
    UtilityGenerated,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProjectEnvironmentDefinitionSource {
    Commands,
    Dockerfile,
    Devcontainer,
    SetupScript,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProjectEnvironmentSetupStepKind {
    Package,
    SystemTool,
    Compiler,
    NativeDependency,
    Command,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub struct ProjectEnvironmentSetupStep {
    pub kind: ProjectEnvironmentSetupStepKind,
    pub command: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub struct ProjectEnvironmentDefinition {
    pub schema_version: u32,
    pub origin: ProjectEnvironmentDefinitionOrigin,
    pub source: ProjectEnvironmentDefinitionSource,
    pub target_platform: String,
    pub source_path: Option<String>,
    pub setup_steps: Vec<ProjectEnvironmentSetupStep>,
    pub validation_commands: Vec<String>,
}

impl ProjectEnvironmentDefinition {
    pub fn validate(&self) -> Result<(), String> {
        if self.schema_version != PROJECT_ENVIRONMENT_DEFINITION_SCHEMA_VERSION {
            return Err(format!(
                "unsupported project environment definition schema version {}",
                self.schema_version
            ));
        }
        let target_platform = self.target_platform.trim();
        if target_platform.is_empty() {
            return Err("target platform must not be empty".to_string());
        }
        if target_platform.chars().count() > MAX_TARGET_PLATFORM_CHARS {
            return Err(format!(
                "target platform cannot exceed {MAX_TARGET_PLATFORM_CHARS} characters"
            ));
        }
        match self.source {
            ProjectEnvironmentDefinitionSource::Commands => {
                if self.source_path.is_some() {
                    return Err("command definitions must not specify a source path".to_string());
                }
            }
            ProjectEnvironmentDefinitionSource::Dockerfile
            | ProjectEnvironmentDefinitionSource::Devcontainer
            | ProjectEnvironmentDefinitionSource::SetupScript => {
                let Some(source_path) = self.source_path.as_deref() else {
                    return Err("file-backed definitions require a source path".to_string());
                };
                if source_path.trim().is_empty()
                    || source_path.contains("..")
                    || Path::new(source_path).is_absolute()
                    || Path::new(source_path).components().any(|component| {
                        matches!(
                            component,
                            std::path::Component::ParentDir
                                | std::path::Component::RootDir
                                | std::path::Component::Prefix(_)
                        )
                    })
                {
                    return Err(
                        "definition source path must be a non-empty relative path without `..`"
                            .to_string(),
                    );
                }
            }
        }
        if self.setup_steps.len() > MAX_SETUP_STEPS {
            return Err(format!(
                "environment definition cannot contain more than {MAX_SETUP_STEPS} setup steps"
            ));
        }
        if self.validation_commands.len() > MAX_VALIDATION_COMMANDS {
            return Err(format!(
                "environment definition cannot contain more than {MAX_VALIDATION_COMMANDS} validation commands"
            ));
        }
        for step in &self.setup_steps {
            validate_command(&step.command, "setup step")?;
        }
        for command in &self.validation_commands {
            validate_command(command, "validation command")?;
        }
        let encoded = serde_json::to_vec(self)
            .map_err(|error| format!("could not encode environment definition: {error}"))?;
        if encoded.len() > MAX_DEFINITION_BYTES {
            return Err(format!(
                "environment definition cannot exceed {MAX_DEFINITION_BYTES} bytes"
            ));
        }
        Ok(())
    }

    pub fn digest(&self) -> String {
        let encoded =
            serde_json::to_vec(self).expect("project environment definition is serializable");
        format!("sha256:{:x}", Sha256::digest(encoded))
    }

    pub(crate) fn with_origin(mut self, origin: ProjectEnvironmentDefinitionOrigin) -> Self {
        self.origin = origin;
        self
    }
}

fn validate_command(command: &str, label: &str) -> Result<(), String> {
    if command.trim().is_empty() {
        return Err(format!("{label} must not be empty"));
    }
    if command.chars().count() > MAX_COMMAND_CHARS {
        return Err(format!(
            "{label} cannot exceed {MAX_COMMAND_CHARS} characters"
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn definition() -> ProjectEnvironmentDefinition {
        ProjectEnvironmentDefinition {
            schema_version: PROJECT_ENVIRONMENT_DEFINITION_SCHEMA_VERSION,
            origin: ProjectEnvironmentDefinitionOrigin::UserAuthored,
            source: ProjectEnvironmentDefinitionSource::Commands,
            target_platform: "linux-x86_64".to_string(),
            source_path: None,
            setup_steps: vec![ProjectEnvironmentSetupStep {
                kind: ProjectEnvironmentSetupStepKind::Compiler,
                command: "rustup toolchain install stable".to_string(),
            }],
            validation_commands: vec!["cargo check --workspace --locked".to_string()],
        }
    }

    #[test]
    fn valid_definition_has_stable_target_bound_digest() {
        let first = definition();
        let second = definition();
        assert_eq!(first.validate(), Ok(()));
        assert_eq!(first.digest(), second.digest());
        assert!(first.digest().starts_with("sha256:"));
    }

    #[test]
    fn definition_rejects_empty_commands_and_oversized_recipe() {
        let mut invalid = definition();
        invalid.setup_steps[0].command = "  ".to_string();
        assert!(invalid
            .validate()
            .unwrap_err()
            .contains("must not be empty"));

        let mut oversized = definition();
        oversized.validation_commands = vec!["x".repeat(MAX_COMMAND_CHARS + 1)];
        assert!(oversized.validate().unwrap_err().contains("cannot exceed"));

        let mut path_escape = definition();
        path_escape.source = ProjectEnvironmentDefinitionSource::Devcontainer;
        path_escape.source_path = Some("../.devcontainer/devcontainer.json".to_string());
        assert!(path_escape
            .validate()
            .unwrap_err()
            .contains("relative path"));
    }
}
