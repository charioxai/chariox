use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::error::DaemonError;

use super::common::resolve_worktree_scoped_path;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReadFileRequest {
    pub session_id: String,
    pub attachment_id: String,
    pub worktree_root: PathBuf,
    pub path: PathBuf,
}

impl ReadFileRequest {
    pub fn new(
        session_id: impl Into<String>,
        attachment_id: impl Into<String>,
        worktree_root: PathBuf,
        path: PathBuf,
    ) -> Self {
        Self {
            session_id: session_id.into(),
            attachment_id: attachment_id.into(),
            worktree_root,
            path,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReadFileResult {
    pub session_id: String,
    pub path: PathBuf,
    pub contents: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EditFileRequest {
    pub session_id: String,
    pub attachment_id: String,
    pub worktree_root: PathBuf,
    pub path: PathBuf,
    pub contents: String,
}

impl EditFileRequest {
    pub fn new(
        session_id: impl Into<String>,
        attachment_id: impl Into<String>,
        worktree_root: PathBuf,
        path: PathBuf,
        contents: impl Into<String>,
    ) -> Self {
        Self {
            session_id: session_id.into(),
            attachment_id: attachment_id.into(),
            worktree_root,
            path,
            contents: contents.into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EditFileResult {
    pub session_id: String,
    pub path: PathBuf,
    pub created: bool,
    pub old_size: usize,
    pub new_size: usize,
    pub bytes_written: usize,
    pub changed: bool,
}

#[derive(Debug, Clone, Default)]
pub struct FileCapabilityService;

impl FileCapabilityService {
    pub fn new() -> Self {
        Self
    }

    pub fn read_file(&self, request: ReadFileRequest) -> Result<ReadFileResult, DaemonError> {
        let path = resolve_worktree_scoped_path(
            &request.session_id,
            &request.worktree_root,
            Some(&request.path),
        )?;
        let contents = std::fs::read_to_string(&path).map_err(|error| {
            DaemonError::FilesystemCapabilityFailed {
                session_id: request.session_id.clone(),
                path: path.display().to_string(),
                message: error.to_string(),
            }
        })?;

        Ok(ReadFileResult {
            session_id: request.session_id,
            path,
            contents,
        })
    }

    pub fn edit_file(&self, request: EditFileRequest) -> Result<EditFileResult, DaemonError> {
        let path = resolve_worktree_scoped_path(
            &request.session_id,
            &request.worktree_root,
            Some(&request.path),
        )?;
        let previous_bytes = std::fs::read(&path).ok();
        std::fs::write(&path, request.contents.as_bytes()).map_err(|error| {
            DaemonError::FileEditFailed {
                session_id: request.session_id.clone(),
                path: path.display().to_string(),
                message: error.to_string(),
            }
        })?;

        Ok(EditFileResult {
            session_id: request.session_id,
            path,
            created: previous_bytes.is_none(),
            old_size: previous_bytes
                .as_ref()
                .map(|bytes| bytes.len())
                .unwrap_or(0),
            new_size: request.contents.len(),
            bytes_written: request.contents.len(),
            changed: previous_bytes.as_deref() != Some(request.contents.as_bytes()),
        })
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::{EditFileRequest, FileCapabilityService, ReadFileRequest};

    #[test]
    fn reads_and_edits_file_inside_worktree() {
        let root = std::env::temp_dir().join("chariox-file-capability-test");
        fs::create_dir_all(&root).expect("root should exist");
        let file = root.join("notes.txt");
        fs::write(&file, "before").expect("file should exist");
        let service = FileCapabilityService::new();

        let read = service
            .read_file(ReadFileRequest::new(
                "session-1",
                "attachment-1",
                root.clone(),
                file.clone(),
            ))
            .expect("file read should succeed");
        assert_eq!(read.contents, "before");

        let edit = service
            .edit_file(EditFileRequest::new(
                "session-1",
                "attachment-1",
                root,
                file.clone(),
                "after",
            ))
            .expect("file edit should succeed");
        assert!(!edit.created);
        assert_eq!(edit.old_size, 6);
        assert_eq!(edit.new_size, 5);
        assert_eq!(edit.bytes_written, 5);
        assert!(edit.changed);
        assert_eq!(
            fs::read_to_string(file).expect("file should be readable"),
            "after"
        );
    }

    #[test]
    fn reports_existing_binary_file_as_not_created() {
        let root = std::env::temp_dir().join("chariox-file-capability-binary-test");
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).expect("root should exist");
        let file = root.join("binary.bin");
        fs::write(&file, [0xff_u8, 0xfe_u8, 0xfd_u8]).expect("binary file should exist");
        let service = FileCapabilityService::new();

        let edit = service
            .edit_file(EditFileRequest::new(
                "session-1",
                "attachment-1",
                root,
                file,
                "text",
            ))
            .expect("file edit should succeed");

        assert!(!edit.created);
        assert_eq!(edit.old_size, 3);
        assert_eq!(edit.new_size, 4);
        assert!(edit.changed);
    }
}
