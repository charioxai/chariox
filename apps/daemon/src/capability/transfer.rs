use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::error::DaemonError;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoreTransferredFileRequest {
    pub session_id: String,
    pub attachment_id: String,
    pub worktree_root: PathBuf,
    pub source_path: PathBuf,
    pub display_name: Option<String>,
}

impl StoreTransferredFileRequest {
    pub fn new(
        session_id: impl Into<String>,
        attachment_id: impl Into<String>,
        worktree_root: PathBuf,
        source_path: PathBuf,
        display_name: Option<String>,
    ) -> Self {
        Self {
            session_id: session_id.into(),
            attachment_id: attachment_id.into(),
            worktree_root,
            source_path,
            display_name,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoredTransferArtifact {
    pub session_id: String,
    pub attachment_id: String,
    pub artifact_id: String,
    pub stored_path: PathBuf,
    pub display_name: String,
    pub stored_name: String,
    pub bytes: usize,
}

#[derive(Debug, Clone, Default)]
pub struct FileTransferService;

impl FileTransferService {
    pub fn new() -> Self {
        Self
    }

    pub fn store_file(
        &self,
        request: StoreTransferredFileRequest,
    ) -> Result<StoredTransferArtifact, DaemonError> {
        let source_path = resolve_transfer_path(&request)?;
        let display_name = request.display_name.unwrap_or_else(|| {
            source_path
                .file_name()
                .map(|value| value.to_string_lossy().into_owned())
                .unwrap_or_else(|| "transfer.bin".to_string())
        });
        let stored_name = sanitize_stored_name(&display_name).ok_or_else(|| {
            DaemonError::InvalidTransferDisplayName {
                session_id: request.session_id.clone(),
                display_name: display_name.clone(),
            }
        })?;

        let artifact_id = format!("transfer-{}-{}", timestamp_ms(), std::process::id());
        let artifact_root = std::env::temp_dir()
            .join("arroba-session-artifacts")
            .join(&request.session_id)
            .join("transfers");
        std::fs::create_dir_all(&artifact_root).map_err(|error| {
            DaemonError::TransferCapabilityFailed {
                session_id: request.session_id.clone(),
                message: error.to_string(),
            }
        })?;
        let stored_path = artifact_root.join(format!("{}-{}", artifact_id, stored_name));
        let bytes = std::fs::copy(&source_path, &stored_path).map_err(|error| {
            DaemonError::TransferCapabilityFailed {
                session_id: request.session_id.clone(),
                message: error.to_string(),
            }
        })? as usize;

        Ok(StoredTransferArtifact {
            session_id: request.session_id,
            attachment_id: request.attachment_id,
            artifact_id,
            stored_path,
            display_name,
            stored_name,
            bytes,
        })
    }
}

fn resolve_transfer_path(request: &StoreTransferredFileRequest) -> Result<PathBuf, DaemonError> {
    let base = if request.source_path.is_absolute() {
        request.source_path.clone()
    } else {
        request.worktree_root.join(&request.source_path)
    };
    std::fs::canonicalize(&base).map_err(|error| DaemonError::TransferCapabilityFailed {
        session_id: request.session_id.clone(),
        message: error.to_string(),
    })
}

fn sanitize_stored_name(name: &str) -> Option<String> {
    let sanitized: String = name
        .chars()
        .map(|ch| match ch {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '_',
            ch if ch.is_control() => '_',
            ch => ch,
        })
        .collect();
    let trimmed = sanitized.trim_matches('.').trim();

    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

fn timestamp_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::{FileTransferService, StoreTransferredFileRequest};
    use crate::DaemonError;

    #[test]
    fn stores_transfer_artifact_under_session_artifact_root() {
        let root = std::env::temp_dir().join("arroba-transfer-capability-test");
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).expect("root should exist");
        let source = root.join("sample.txt");
        fs::write(&source, "artifact").expect("source should exist");

        let result = FileTransferService::new()
            .store_file(StoreTransferredFileRequest::new(
                "session-1",
                "attachment-1",
                root,
                source,
                None,
            ))
            .expect("transfer should succeed");

        assert!(result
            .stored_path
            .to_string_lossy()
            .contains("arroba-session-artifacts"));
        assert_eq!(result.bytes, 8);
        assert_eq!(
            fs::read_to_string(result.stored_path).expect("stored artifact should exist"),
            "artifact"
        );
    }

    #[test]
    fn sanitizes_display_name_for_storage_path() {
        let root = std::env::temp_dir().join("arroba-transfer-display-name-test");
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).expect("root should exist");
        let source = root.join("sample.txt");
        fs::write(&source, "artifact").expect("source should exist");

        let result = FileTransferService::new()
            .store_file(StoreTransferredFileRequest::new(
                "session-1",
                "attachment-1",
                root,
                source,
                Some("foo/bar.txt".to_string()),
            ))
            .expect("transfer should succeed");

        assert_eq!(result.display_name, "foo/bar.txt");
        assert_eq!(result.stored_name, "foo_bar.txt");
        assert!(!result.stored_path.to_string_lossy().contains("foo/bar.txt"));
    }

    #[test]
    fn rejects_empty_sanitized_display_name() {
        let root = std::env::temp_dir().join("arroba-transfer-invalid-name-test");
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).expect("root should exist");
        let source = root.join("sample.txt");
        fs::write(&source, "artifact").expect("source should exist");

        let error = FileTransferService::new()
            .store_file(StoreTransferredFileRequest::new(
                "session-1",
                "attachment-1",
                root,
                source,
                Some("...".to_string()),
            ))
            .expect_err("invalid display name should be rejected");

        match error {
            DaemonError::InvalidTransferDisplayName { .. } => {}
            other => panic!("unexpected error: {other}"),
        }
    }
}
