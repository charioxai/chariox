//! One Environment per Project (schema 2): a Setup script, Variables, Secret names and Files outside
//! Git. Values live in each kernel's Vault; presence is computed on every read.
mod detect;
mod draft;
mod files;
mod launch;
mod operations;
mod presence;
mod proposals;
mod record;
mod registrations;
mod setup;
mod store;
mod transfer;

pub(crate) use detect::{declared_in, detect};
pub(crate) use draft::{draft_facts, draft_prompt, parse_draft};
pub(crate) use files::{environment_file_rules, register_environment_files};
pub(crate) use launch::{
    attach_project_provider_environment, project_launch_environment, project_mcp_environment,
};
pub(crate) use operations::{Begun, EnvironmentOperations, OperationChanges, OperationHandle};
pub(crate) use presence::{file_presence, secret_presence};
pub use presence::{FilePresence, SecretPresence};
pub(crate) use proposals::{
    apply as apply_proposal, check_prompt, deterministic_proposals, parse_proposals, ProposalStore,
};
pub use record::{
    environment_variable_name, extension_name, parse_variables, project_environment_protected_name,
    relative_project_path, ExtensionEntry, ExtensionSource, ProjectEnvironmentRecord,
    PROJECT_ENVIRONMENT_SCHEMA,
};
pub(crate) use registrations::{
    mcp_key_names, ProjectRegistrations, Registration, RegistrationHost, UserRegistrations,
};
pub(crate) use setup::{SetupRuns, SetupSpec};
pub(crate) use store::{remove_schema_1_state, ProjectEnvironmentStore, SaveOutcome};
pub(crate) use transfer::{
    read_file_slice, run_import, source_holdings, EnvironmentSource, FileSlice, ImportTarget,
    SourceError, SourceSnapshot, IMPORT_CHUNK_BYTES, MAX_IMPORT_FILE_BYTES,
};

/// The Vault service that holds a Project's Secret values on this kernel; the key is the name.
pub fn project_environment_vault_service(project_id: &str) -> String {
    use sha2::{Digest, Sha256};
    format!(
        "project-environment:{:x}",
        Sha256::digest(project_id.as_bytes())
    )
}

pub(crate) fn environment_error(message: impl Into<String>) -> crate::error::DaemonError {
    crate::error::DaemonError::LocalTransport {
        operation: "Project environment",
        message: message.into(),
    }
}
