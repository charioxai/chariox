use std::collections::BTreeSet;
use std::path::Path;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub const PROJECT_ENVIRONMENT_DEFINITION_SCHEMA_VERSION: u32 = 1;
/// Requests that the worker kernel calculate the file digest from its
/// materialized worktree. This is used by read-only provider discovery when
/// the provider cannot execute a hashing command (for example, OpenCode's
/// discovery policy denies bash).
pub(crate) const KERNEL_COMPUTED_INPUT_ATTESTATION: &str = "sha256:kernel";

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

#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProjectEnvironmentInputKind {
    Recipe,
    Lockfile,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Ord, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProjectEnvironmentPathBase {
    PreparationHome,
    Workspace,
}

/// A relative directory that setup is expected to make available for named
/// commands after the definition has been applied. The base is explicit so
/// the worker can resolve it without interpreting shell syntax or trusting a
/// provider-provided absolute path.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub struct ProjectEnvironmentPathEntry {
    pub base: ProjectEnvironmentPathBase,
    pub path: String,
}

/// A content-only attestation for a project file that the worker must observe
/// in its already-materialized worktree. The path and digest are safe to
/// persist and transfer; file bytes and credentials never cross this seam.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub struct ProjectEnvironmentInput {
    pub kind: ProjectEnvironmentInputKind,
    pub path: String,
    pub sha256: String,
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
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub inputs: Vec<ProjectEnvironmentInput>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub path_entries: Vec<ProjectEnvironmentPathEntry>,
    pub setup_steps: Vec<ProjectEnvironmentSetupStep>,
    pub validation_commands: Vec<String>,
}

impl ProjectEnvironmentDefinition {
    pub fn validate(&self) -> Result<(), String> {
        self.validate_with_source_attestation(true)
    }

    pub(crate) fn canonicalize_input_attestations(&self) -> Result<Self, String> {
        let mut canonical = self.clone();
        for input in &mut canonical.inputs {
            input.sha256 = canonicalize_input_digest(&input.sha256, &input.path)?;
        }
        Ok(canonical)
    }

    pub(crate) fn validate_for_repair(&self) -> Result<(), String> {
        if !self.is_unattested_file_backed() {
            return self.validate();
        }
        self.validate_with_source_attestation(false)
    }

    pub(crate) fn validate_for_utility_output(&self) -> Result<(), String> {
        self.validate_with_source_attestation_and_kernel_attestations(true, true)
    }

    pub(crate) fn is_unattested_file_backed(&self) -> bool {
        matches!(
            self.source,
            ProjectEnvironmentDefinitionSource::Dockerfile
                | ProjectEnvironmentDefinitionSource::Devcontainer
                | ProjectEnvironmentDefinitionSource::SetupScript
        ) && self.source_path.as_deref().is_some_and(|source_path| {
            !self.inputs.iter().any(|input| {
                input.kind == ProjectEnvironmentInputKind::Recipe && input.path == source_path
            })
        })
    }

    fn validate_with_source_attestation(
        &self,
        require_source_attestation: bool,
    ) -> Result<(), String> {
        self.validate_with_source_attestation_and_kernel_attestations(
            require_source_attestation,
            false,
        )
    }

    fn validate_with_source_attestation_and_kernel_attestations(
        &self,
        require_source_attestation: bool,
        allow_kernel_attestations: bool,
    ) -> Result<(), String> {
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
        validate_path_entries(&self.path_entries)?;
        validate_inputs(&self.inputs, allow_kernel_attestations)?;
        if require_source_attestation {
            if let Some(source_path) = self.source_path.as_deref() {
                if !self.inputs.iter().any(|input| {
                    input.kind == ProjectEnvironmentInputKind::Recipe && input.path == source_path
                }) {
                    return Err(
                        "file-backed definitions must attest source_path with a recipe input"
                            .to_string(),
                    );
                }
            }
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

const MAX_PROJECT_ENVIRONMENT_INPUTS: usize = 64;
const MAX_PROJECT_ENVIRONMENT_INPUT_PATH_CHARS: usize = 512;
const MAX_PROJECT_ENVIRONMENT_PATH_ENTRIES: usize = 64;
const MAX_PROJECT_ENVIRONMENT_PATH_CHARS: usize = 512;

fn validate_path_entries(entries: &[ProjectEnvironmentPathEntry]) -> Result<(), String> {
    if entries.len() > MAX_PROJECT_ENVIRONMENT_PATH_ENTRIES {
        return Err(format!(
            "environment definition cannot contain more than {MAX_PROJECT_ENVIRONMENT_PATH_ENTRIES} executable path entries"
        ));
    }
    let mut paths = BTreeSet::new();
    for entry in entries {
        let path = Path::new(&entry.path);
        if entry.path.trim().is_empty()
            || entry.path.chars().count() > MAX_PROJECT_ENVIRONMENT_PATH_CHARS
            || entry.path.chars().any(|character| character == '\0')
            || path.is_absolute()
            || path.components().any(|component| {
                matches!(
                    component,
                    std::path::Component::ParentDir
                        | std::path::Component::RootDir
                        | std::path::Component::Prefix(_)
                )
            })
        {
            return Err(
                "environment executable path must be a non-empty relative path without `..`"
                    .to_string(),
            );
        }
        if !paths.insert((entry.base, entry.path.clone())) {
            return Err(format!(
                "environment executable path is listed more than once: {}",
                entry.path
            ));
        }
    }
    Ok(())
}

fn validate_inputs(
    inputs: &[ProjectEnvironmentInput],
    allow_kernel_attestations: bool,
) -> Result<(), String> {
    if inputs.len() > MAX_PROJECT_ENVIRONMENT_INPUTS {
        return Err(format!(
            "environment definition cannot contain more than {MAX_PROJECT_ENVIRONMENT_INPUTS} recipe or lockfile inputs"
        ));
    }
    let mut paths = BTreeSet::new();
    for input in inputs {
        let path = Path::new(&input.path);
        if input.path.trim().is_empty()
            || input.path.chars().count() > MAX_PROJECT_ENVIRONMENT_INPUT_PATH_CHARS
            || path.is_absolute()
            || path.components().any(|component| {
                matches!(
                    component,
                    std::path::Component::ParentDir
                        | std::path::Component::RootDir
                        | std::path::Component::Prefix(_)
                )
            })
        {
            return Err(
                "environment input path must be a non-empty relative path without `..`".to_string(),
            );
        }
        if !paths.insert(input.path.clone()) {
            return Err(format!(
                "environment input path is listed more than once: {}",
                input.path
            ));
        }
        if allow_kernel_attestations && input.sha256 == KERNEL_COMPUTED_INPUT_ATTESTATION {
            continue;
        }
        if canonicalize_input_digest(&input.sha256, &input.path)? != input.sha256 {
            return Err(format!(
                "environment input digest must be a 256-bit sha256 value: {}",
                input.path
            ));
        }
    }
    Ok(())
}

fn canonicalize_input_digest(value: &str, path: &str) -> Result<String, String> {
    let Some(digest) = value.strip_prefix("sha256:") else {
        return Err(format!(
            "environment input digest must use sha256: prefix: {path}"
        ));
    };
    if digest.len() != 64 || !digest.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(format!(
            "environment input digest must be a 256-bit sha256 value: {path}"
        ));
    }
    Ok(format!("sha256:{}", digest.to_ascii_lowercase()))
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
            inputs: Vec::new(),
            path_entries: Vec::new(),
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

    #[test]
    fn definition_validates_definition_derived_executable_paths() {
        let mut definition = definition();
        definition.path_entries = vec![
            ProjectEnvironmentPathEntry {
                base: ProjectEnvironmentPathBase::PreparationHome,
                path: "go/bin".to_string(),
            },
            ProjectEnvironmentPathEntry {
                base: ProjectEnvironmentPathBase::Workspace,
                path: ".venv/bin".to_string(),
            },
        ];
        assert!(definition.validate().is_ok());

        let mut escaped = definition.clone();
        escaped.path_entries[0].path = "../go/bin".to_string();
        assert!(escaped.validate().unwrap_err().contains("relative path"));

        let mut duplicate = definition;
        duplicate.path_entries.push(ProjectEnvironmentPathEntry {
            base: ProjectEnvironmentPathBase::PreparationHome,
            path: "go/bin".to_string(),
        });
        assert!(duplicate
            .validate()
            .unwrap_err()
            .contains("listed more than once"));
    }

    #[test]
    fn file_backed_definition_requires_recipe_attestation_and_valid_digests() {
        let mut definition = definition();
        definition.source = ProjectEnvironmentDefinitionSource::Devcontainer;
        definition.source_path = Some(".devcontainer/devcontainer.json".to_string());
        assert!(definition
            .validate()
            .unwrap_err()
            .contains("attest source_path"));

        definition.inputs = vec![ProjectEnvironmentInput {
            kind: ProjectEnvironmentInputKind::Recipe,
            path: ".devcontainer/devcontainer.json".to_string(),
            sha256: format!("sha256:{}", "a".repeat(64)),
        }];
        assert_eq!(definition.validate(), Ok(()));
        let original_digest = definition.digest();

        definition.inputs[0].sha256 = "sha256:not-a-digest".to_string();
        assert_ne!(
            definition.digest(),
            original_digest,
            "the persisted definition identity must include input content identities"
        );
        assert!(definition
            .validate()
            .unwrap_err()
            .contains("256-bit sha256"));
    }

    #[test]
    fn legacy_unattested_file_backed_definition_is_repairable_but_not_reusable() {
        let mut definition = definition();
        definition.source = ProjectEnvironmentDefinitionSource::Devcontainer;
        definition.source_path = Some(".devcontainer/devcontainer.json".to_string());
        assert!(definition.validate().is_err());
        assert!(definition.is_unattested_file_backed());
        assert_eq!(definition.validate_for_repair(), Ok(()));
    }

    #[test]
    fn input_attestation_digests_require_canonical_lowercase_hex() {
        let mut definition = definition();
        definition.source = ProjectEnvironmentDefinitionSource::Devcontainer;
        definition.source_path = Some(".devcontainer/devcontainer.json".to_string());
        definition.inputs = vec![ProjectEnvironmentInput {
            kind: ProjectEnvironmentInputKind::Recipe,
            path: ".devcontainer/devcontainer.json".to_string(),
            sha256: format!("sha256:{}", "A".repeat(64)),
        }];
        assert!(definition
            .validate()
            .unwrap_err()
            .contains("256-bit sha256"));
    }

    #[test]
    fn kernel_input_attestation_marker_is_utility_only() {
        let mut definition = definition();
        definition.source = ProjectEnvironmentDefinitionSource::Devcontainer;
        definition.source_path = Some(".devcontainer/devcontainer.json".to_string());
        definition.inputs = vec![ProjectEnvironmentInput {
            kind: ProjectEnvironmentInputKind::Recipe,
            path: ".devcontainer/devcontainer.json".to_string(),
            sha256: KERNEL_COMPUTED_INPUT_ATTESTATION.to_string(),
        }];

        assert!(definition.validate().is_err());
        assert_eq!(definition.validate_for_utility_output(), Ok(()));
    }

    #[test]
    fn input_attestations_are_unique_and_do_not_allow_path_escape() {
        let mut definition = definition();
        definition.inputs = vec![
            ProjectEnvironmentInput {
                kind: ProjectEnvironmentInputKind::Lockfile,
                path: "Cargo.lock".to_string(),
                sha256: format!("sha256:{}", "b".repeat(64)),
            },
            ProjectEnvironmentInput {
                kind: ProjectEnvironmentInputKind::Recipe,
                path: "Cargo.lock".to_string(),
                sha256: format!("sha256:{}", "c".repeat(64)),
            },
        ];
        assert!(definition
            .validate()
            .unwrap_err()
            .contains("listed more than once"));

        definition.inputs[1].path = "../Cargo.toml".to_string();
        assert!(definition.validate().unwrap_err().contains("relative path"));
    }

    #[test]
    fn file_backed_script_and_application_private_key_fixture_remain_usable() {
        let mut valid = definition();
        valid.source = ProjectEnvironmentDefinitionSource::SetupScript;
        valid.source_path = Some("scripts/setup.sh".to_string());
        valid.inputs = vec![ProjectEnvironmentInput {
            kind: ProjectEnvironmentInputKind::Recipe,
            path: "scripts/setup.sh".to_string(),
            sha256: format!("sha256:{}", "a".repeat(64)),
        }];
        valid.setup_steps[0].command = "./scripts/setup.sh".to_string();
        valid.validation_commands = vec![
            "test -f testdata/private_key.pem".to_string(),
            "printf '%s' ssh-keyscan > setup-evidence.txt".to_string(),
        ];
        assert_eq!(
            valid.validate(),
            Ok(()),
            "opaque scripts and application fixture names are not SSH credential operations"
        );
    }

    #[test]
    fn setup_commands_allow_selected_agent_credentials_and_existing_host_verification() {
        let mut valid = definition();
        valid.setup_steps[0].command =
            "test -n \"$SSH_AUTH_SOCK\" && git -c core.sshCommand='ssh -F ~/.ssh/config' fetch"
                .to_string();
        assert_eq!(valid.validate(), Ok(()));
    }
}
