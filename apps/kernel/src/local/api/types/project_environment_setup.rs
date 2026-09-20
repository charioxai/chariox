use super::*;

fn serialize_bounded_progress_percent<S>(
    progress_percent: &u8,
    serializer: S,
) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    if *progress_percent > 100 {
        return Err(serde::ser::Error::custom(
            "project environment setup progress must be between 0 and 100",
        ));
    }
    progress_percent.serialize(serializer)
}

fn deserialize_bounded_progress_percent<'de, D>(deserializer: D) -> Result<u8, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let progress_percent = u8::deserialize(deserializer)?;
    if progress_percent > 100 {
        return Err(serde::de::Error::custom(
            "project environment setup progress must be between 0 and 100",
        ));
    }
    Ok(progress_percent)
}

pub use crate::session::{
    ProjectEnvironmentDefinition, ProjectEnvironmentDefinitionOrigin,
    ProjectEnvironmentDefinitionSource, ProjectEnvironmentInput, ProjectEnvironmentInputKind,
    ProjectEnvironmentSetupStep, ProjectEnvironmentSetupStepKind,
};

pub use crate::session::PROJECT_ENVIRONMENT_DEFINITION_SCHEMA_VERSION;

#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProjectEnvironmentSetupPhase {
    Requested,
    Preparing,
    Validating,
    Ready,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub struct ProjectEnvironmentCommandResult {
    pub command_digest: String,
    pub exit_code: i32,
    pub stdout_bytes: usize,
    pub stderr_bytes: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub struct ProjectEnvironmentValidation {
    pub worker_id: String,
    pub platform: String,
    pub commands: Vec<ProjectEnvironmentCommandResult>,
}

impl ProjectEnvironmentValidation {
    pub fn passed(&self) -> bool {
        !self.commands.is_empty() && self.commands.iter().all(|result| result.exit_code == 0)
    }

    pub fn failed_commands(&self) -> usize {
        self.commands
            .iter()
            .filter(|result| result.exit_code != 0)
            .count()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub struct ProjectEnvironmentSetupStatus {
    pub operation_id: String,
    pub project_id: String,
    pub session_id: String,
    pub agent_id: String,
    pub worker_id: String,
    pub platform: String,
    pub phase: ProjectEnvironmentSetupPhase,
    pub attempt: u32,
    #[serde(
        serialize_with = "serialize_bounded_progress_percent",
        deserialize_with = "deserialize_bounded_progress_percent"
    )]
    pub progress_percent: u8,
    pub definition_digest: Option<String>,
    pub validation: Option<ProjectEnvironmentValidation>,
    pub message: Option<String>,
    pub failure_code: Option<String>,
    pub failure_message: Option<String>,
    pub retryable: bool,
    pub created_at_ms: u64,
    pub updated_at_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StartProjectEnvironmentSetupRequest {
    pub operation_id: String,
    pub project_id: String,
    pub session_id: String,
    pub agent_id: String,
    pub target_worker_id: String,
    pub target_platform: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub definition: Option<ProjectEnvironmentDefinition>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub validation_commands: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GetProjectEnvironmentSetupStatusRequest {
    pub operation_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CancelProjectEnvironmentSetupRequest {
    pub operation_id: String,
    pub session_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RetryProjectEnvironmentSetupRequest {
    pub operation_id: String,
    pub session_id: String,
}
