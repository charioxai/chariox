use std::fmt;

use serde::Serialize;

/// Stable machine-readable failure categories; messages are diagnostic only.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ErrorCode {
    InvalidArchive,
    ArchiveLimit,
    InvalidPath,
    DuplicatePath,
    InvalidManifest,
    InvalidSchema,
    IncompatibleProtocol,
    IncompatibleSdk,
    IncompatibleContract,
    IncompatibleResourcePolicy,
    UntrustedPublisher,
    InvalidSignature,
    IntegrityMismatch,
    MissingEntry,
    UnexpectedEntry,
    UnsupportedFeature,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PackageError {
    pub code: ErrorCode,
    pub message: String,
}

impl PackageError {
    pub(crate) fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

impl fmt::Display for PackageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}: {}", self.code, self.message)
    }
}

impl std::error::Error for PackageError {}

pub type Result<T> = std::result::Result<T, PackageError>;
