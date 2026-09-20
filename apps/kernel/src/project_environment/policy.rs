use std::path::PathBuf;

use thiserror::Error;
use unicode_normalization::UnicodeNormalization;

/// The only policy version understood by this first, data-only contract slice.
pub const POLICY_VERSION: &str = "project-environment-policy-v1";
pub const MANIFEST_SCHEMA_VERSION: u16 = 1;
pub const CONTRACT_SCHEMA_VERSION: u16 = 1;
pub const MANIFEST_RELATIVE_PATH: &str = "project-environment.toml";

pub const MAX_MANIFEST_BYTES: usize = 128 * 1024;
pub const MAX_LOCKFILE_BYTES: usize = 4 * 1024 * 1024;
pub const MAX_STRING_BYTES: usize = 256;
pub const MAX_PATH_BYTES: usize = 512;
pub const MAX_MANIFEST_ENTRIES: usize = 128;
pub const MAX_UTILITIES: usize = 32;
pub const MAX_TOOLCHAINS: usize = 32;
pub const MAX_TOOLCHAIN_COMPONENTS: usize = 16;
pub const MAX_LOCKFILES: usize = 32;
pub const MAX_VALIDATION_PROBES: usize = 64;

#[derive(Debug, Error)]
pub enum ProjectEnvironmentError {
    #[error("manifest is malformed: {message}")]
    MalformedManifest { message: String },

    #[error("manifest schema version {0} is unsupported")]
    UnsupportedManifestVersion(u16),

    #[error("contract policy version `{0}` is unsupported")]
    UnsupportedPolicyVersion(String),

    #[error("manifest {kind} exceeds the bound of {limit}")]
    BoundExceeded { kind: &'static str, limit: usize },

    #[error("manifest field `{field}` is invalid: {message}")]
    InvalidField { field: String, message: String },

    #[error("manifest path `{path}` is invalid: {reason}")]
    InvalidPath { path: String, reason: String },

    #[error("manifest entry `{entry}` is duplicated")]
    DuplicateEntry { entry: String },

    #[error("repository file `{path}` is missing")]
    MissingFile { path: PathBuf },

    #[error("repository file `{path}` is not a regular non-symlink file")]
    UnsafeFile { path: PathBuf },

    #[error("repository path `{path}` contains a symlink")]
    SymlinkPath { path: PathBuf },

    #[error("could not inspect repository path `{path}`: {source}")]
    InspectPath {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("could not read repository file `{path}`: {source}")]
    ReadFile {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

pub(crate) fn normalize_text(
    field: &str,
    value: &str,
    limit: usize,
) -> Result<String, ProjectEnvironmentError> {
    let normalized = value.nfkc().collect::<String>().trim().to_owned();
    if normalized.is_empty() {
        return Err(ProjectEnvironmentError::InvalidField {
            field: field.to_owned(),
            message: "must not be empty".to_owned(),
        });
    }
    if normalized.as_bytes().len() > limit {
        return Err(ProjectEnvironmentError::BoundExceeded {
            kind: "string length",
            limit,
        });
    }
    if normalized.chars().any(char::is_control) {
        return Err(ProjectEnvironmentError::InvalidField {
            field: field.to_owned(),
            message: "must not contain control characters".to_owned(),
        });
    }
    Ok(normalized)
}

pub(crate) fn normalize_relative_path(
    field: &str,
    value: &str,
) -> Result<String, ProjectEnvironmentError> {
    let normalized = normalize_text(field, value, MAX_PATH_BYTES)?;
    if normalized.contains('\\') {
        return Err(ProjectEnvironmentError::InvalidPath {
            path: value.to_owned(),
            reason: "backslash separators are not supported".to_owned(),
        });
    }
    if normalized.starts_with('/') || normalized.starts_with('~') {
        return Err(ProjectEnvironmentError::InvalidPath {
            path: normalized,
            reason: "path must be relative".to_owned(),
        });
    }
    if normalized
        .split('/')
        .any(|component| component.is_empty() || component == "." || component == "..")
    {
        return Err(ProjectEnvironmentError::InvalidPath {
            path: normalized,
            reason: "empty, `.` and `..` path components are not supported".to_owned(),
        });
    }
    if normalized
        .split('/')
        .next()
        .is_some_and(|component| component.contains(':'))
    {
        return Err(ProjectEnvironmentError::InvalidPath {
            path: normalized,
            reason: "platform-specific absolute paths are not supported".to_owned(),
        });
    }
    Ok(normalized)
}

pub(crate) fn normalize_digest(
    field: &str,
    value: &str,
) -> Result<String, ProjectEnvironmentError> {
    let normalized = normalize_text(field, value, 64)?.to_ascii_lowercase();
    if normalized.len() != 64 || !normalized.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(ProjectEnvironmentError::InvalidField {
            field: field.to_owned(),
            message: "must be a 64-character SHA-256 hexadecimal digest".to_owned(),
        });
    }
    Ok(normalized)
}

pub(crate) fn enforce_count(
    kind: &'static str,
    count: usize,
    limit: usize,
) -> Result<(), ProjectEnvironmentError> {
    if count > limit {
        return Err(ProjectEnvironmentError::BoundExceeded { kind, limit });
    }
    Ok(())
}

pub(crate) fn duplicate<T>(entry: impl Into<String>) -> Result<T, ProjectEnvironmentError> {
    Err(ProjectEnvironmentError::DuplicateEntry {
        entry: entry.into(),
    })
}
