use std::fs;
use std::io::ErrorKind;
use std::path::{Component, Path, PathBuf};

use crate::io::coordinator::{ArtifactEditCoordinator, ArtifactReadRequest, ArtifactWriteRequest};
use crate::io::types::{
    AgentEditIntent, AgentEditOperation, ArtifactContent, ArtifactDomainKind, ArtifactEditError,
    ArtifactReadResult, EditResult, WorkspaceIdentity,
};

#[derive(Debug, Clone)]
pub struct ManagedFileReadRequest {
    pub workspace_identity: WorkspaceIdentity,
    pub workspace_root: PathBuf,
    pub path: PathBuf,
    pub domain: ArtifactDomainKind,
}

#[derive(Debug, Clone)]
pub struct ManagedFileWriteRequest {
    pub workspace_identity: WorkspaceIdentity,
    pub workspace_root: PathBuf,
    pub intent: AgentEditIntent,
    pub domain: ArtifactDomainKind,
}

pub struct ManagedFileIo;

impl ManagedFileIo {
    pub fn read_artifact(
        coordinator: &mut ArtifactEditCoordinator,
        request: ManagedFileReadRequest,
    ) -> Result<ArtifactReadResult, ArtifactEditError> {
        let full_path = resolve_workspace_path(&request.workspace_root, &request.path)?;
        let content = read_content(&full_path, request.domain)?;
        Ok(coordinator.read_artifact(ArtifactReadRequest {
            workspace_identity: request.workspace_identity,
            path: request.path,
            domain: request.domain,
            content,
        }))
    }

    pub fn apply_edit(
        coordinator: &mut ArtifactEditCoordinator,
        request: ManagedFileWriteRequest,
    ) -> EditResult {
        match Self::apply_edit_inner(coordinator, request) {
            Ok(result) => result,
            Err(reason) => EditResult::Rejected { reason },
        }
    }

    fn apply_edit_inner(
        coordinator: &mut ArtifactEditCoordinator,
        request: ManagedFileWriteRequest,
    ) -> Result<EditResult, ArtifactEditError> {
        reject_arroba_owned_write_path(&request.workspace_root, &request.intent.path)?;
        let full_path = resolve_workspace_path(&request.workspace_root, &request.intent.path)?;
        let allow_missing = matches!(
            request.intent.operation,
            AgentEditOperation::WriteArtifact { .. }
        );
        let observed_content = if allow_missing {
            read_content_or_empty(&full_path, request.domain)?
        } else {
            read_content(&full_path, request.domain)?
        };
        coordinator.read_artifact(ArtifactReadRequest {
            workspace_identity: request.workspace_identity.clone(),
            path: request.intent.path.clone(),
            domain: request.domain,
            content: observed_content.clone(),
        });

        let prepared = coordinator.prepare_edit(ArtifactWriteRequest {
            workspace_identity: request.workspace_identity,
            intent: request.intent,
        })?;
        let latest_content = if allow_missing {
            read_content_or_empty(&full_path, request.domain)?
        } else {
            read_content(&full_path, request.domain)?
        };
        if latest_content != observed_content {
            return Err(ArtifactEditError::ExternalChangeDuringApply { path: full_path });
        }
        write_content(&full_path, &prepared.content)?;
        Ok(coordinator.commit_prepared_edit(prepared))
    }
}

fn read_content_or_empty(
    path: &Path,
    domain: ArtifactDomainKind,
) -> Result<ArtifactContent, ArtifactEditError> {
    match fs::read(path) {
        Ok(bytes) => content_from_bytes(path, domain, bytes),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(empty_content(domain)),
        Err(error) => Err(ArtifactEditError::Filesystem {
            path: path.to_path_buf(),
            message: error.to_string(),
        }),
    }
}

fn read_content(
    path: &Path,
    domain: ArtifactDomainKind,
) -> Result<ArtifactContent, ArtifactEditError> {
    let bytes = fs::read(path).map_err(|error| ArtifactEditError::Filesystem {
        path: path.to_path_buf(),
        message: error.to_string(),
    })?;
    content_from_bytes(path, domain, bytes)
}

fn content_from_bytes(
    _path: &Path,
    domain: ArtifactDomainKind,
    bytes: Vec<u8>,
) -> Result<ArtifactContent, ArtifactEditError> {
    match domain {
        ArtifactDomainKind::TextDocument | ArtifactDomainKind::StructuredDocument => {
            let text =
                String::from_utf8(bytes).map_err(|error| ArtifactEditError::InvalidOperation {
                    message: format!("artifact is not valid UTF-8: {error}"),
                })?;
            Ok(ArtifactContent::Text(text))
        }
        ArtifactDomainKind::OpaqueBlob => Ok(ArtifactContent::Bytes(bytes)),
    }
}

fn empty_content(domain: ArtifactDomainKind) -> ArtifactContent {
    match domain {
        ArtifactDomainKind::TextDocument | ArtifactDomainKind::StructuredDocument => {
            ArtifactContent::Text(String::new())
        }
        ArtifactDomainKind::OpaqueBlob => ArtifactContent::Bytes(Vec::new()),
    }
}

fn write_content(path: &Path, content: &ArtifactContent) -> Result<(), ArtifactEditError> {
    let bytes = match content {
        ArtifactContent::Text(text) => text.as_bytes().to_vec(),
        ArtifactContent::Bytes(bytes) => bytes.clone(),
    };
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| ArtifactEditError::Filesystem {
            path: parent.to_path_buf(),
            message: error.to_string(),
        })?;
    }
    fs::write(path, bytes).map_err(|error| ArtifactEditError::Filesystem {
        path: path.to_path_buf(),
        message: error.to_string(),
    })
}

fn resolve_workspace_path(root: &Path, path: &Path) -> Result<PathBuf, ArtifactEditError> {
    Ok(root.join(normalize_workspace_relative_path(path)?))
}

fn normalize_workspace_relative_path(path: &Path) -> Result<PathBuf, ArtifactEditError> {
    if path.is_absolute() {
        return Err(ArtifactEditError::InvalidOperation {
            message: "managed file paths must be relative to the workspace root".to_string(),
        });
    }
    let mut relative = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Normal(part) => relative.push(part),
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(ArtifactEditError::InvalidOperation {
                    message: "managed file path escapes the workspace root".to_string(),
                });
            }
        }
    }
    Ok(relative)
}

fn reject_arroba_owned_write_path(root: &Path, path: &Path) -> Result<(), ArtifactEditError> {
    let relative = normalize_workspace_relative_path(path)?;
    if relative == Path::new(crate::provider::MANAGED_IO_INSTRUCTIONS_SOURCE_PATH)
        && is_arroba_source_workspace(root)
    {
        return Err(ArtifactEditError::InvalidOperation {
            message: format!(
                "the Arroba managed-I/O instruction policy `{}` is owned by Arroba and cannot be edited through managed artifact I/O",
                crate::provider::MANAGED_IO_INSTRUCTIONS_SOURCE_PATH
            ),
        });
    }
    Ok(())
}

fn is_arroba_source_workspace(root: &Path) -> bool {
    root.join("apps/kernel/Cargo.toml").is_file()
        && root
            .join(crate::provider::MANAGED_IO_INSTRUCTIONS_SOURCE_PATH)
            .is_file()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::io::types::{AgentEditOperation, ArtifactEditWarning, TextRange};
    use std::time::{SystemTime, UNIX_EPOCH};

    fn workspace() -> WorkspaceIdentity {
        WorkspaceIdentity::local("repo-a")
    }

    fn test_root(name: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock before unix epoch")
            .as_nanos();
        let root = std::env::temp_dir().join(format!("arroba-managed-file-io-{name}-{nanos}"));
        fs::create_dir_all(&root).expect("create test root");
        root
    }

    fn text_range_of(haystack: &str, needle: &str) -> TextRange {
        let start = haystack.find(needle).expect("needle should exist");
        TextRange::new(start, start + needle.len())
    }

    #[test]
    fn managed_file_read_tracks_snapshot() {
        let root = test_root("read");
        let path = root.join("src.txt");
        fs::write(&path, "alpha\n").expect("write fixture");
        let mut coordinator = ArtifactEditCoordinator::new();

        let read = ManagedFileIo::read_artifact(
            &mut coordinator,
            ManagedFileReadRequest {
                workspace_identity: workspace(),
                workspace_root: root,
                path: PathBuf::from("src.txt"),
                domain: ArtifactDomainKind::TextDocument,
            },
        )
        .expect("read artifact");

        assert_eq!(read.content, ArtifactContent::Text("alpha\n".to_string()));
        assert!(coordinator.current_content(&read.artifact_id).is_some());
    }

    #[test]
    fn managed_file_apply_rebases_external_non_overlap_before_write() {
        let root = test_root("rebase");
        let path = root.join("src.txt");
        fs::write(&path, "one\ntwo\nthree\n").expect("write fixture");
        let mut coordinator = ArtifactEditCoordinator::new();
        let first_read = ManagedFileIo::read_artifact(
            &mut coordinator,
            ManagedFileReadRequest {
                workspace_identity: workspace(),
                workspace_root: root.clone(),
                path: PathBuf::from("src.txt"),
                domain: ArtifactDomainKind::TextDocument,
            },
        )
        .expect("read artifact");
        fs::write(&path, "zero\none\ntwo\nthree\n").expect("external write");

        let result = ManagedFileIo::apply_edit(
            &mut coordinator,
            ManagedFileWriteRequest {
                workspace_identity: workspace(),
                workspace_root: root,
                domain: ArtifactDomainKind::TextDocument,
                intent: AgentEditIntent {
                    path: PathBuf::from("src.txt"),
                    snapshot_id: Some(first_read.snapshot_id),
                    operation: AgentEditOperation::ReplaceText {
                        old_text: "three".to_string(),
                        new_text: "four".to_string(),
                    },
                },
            },
        );

        assert!(matches!(
            result,
            EditResult::AppliedWithWarning {
                warning: ArtifactEditWarning::RebasedOverNonOverlappingChange { .. },
                ..
            }
        ));
        assert_eq!(
            fs::read_to_string(&path).expect("read result"),
            "zero\none\ntwo\nfour\n"
        );
    }

    #[test]
    fn managed_file_apply_places_rebased_range_edit_exactly() {
        let root = test_root("exact-rebase");
        let path = root.join("src.txt");
        let base = "header\nalpha\nTARGET\nomega\nfooter\n";
        fs::write(&path, base).expect("write fixture");
        let mut coordinator = ArtifactEditCoordinator::new();
        let first_read = ManagedFileIo::read_artifact(
            &mut coordinator,
            ManagedFileReadRequest {
                workspace_identity: workspace(),
                workspace_root: root.clone(),
                path: PathBuf::from("src.txt"),
                domain: ArtifactDomainKind::TextDocument,
            },
        )
        .expect("read artifact");
        fs::write(
            &path,
            "intro\nheader\nalpha\nTARGET\nomega\nfooter\noutro\n",
        )
        .expect("external write");

        let result = ManagedFileIo::apply_edit(
            &mut coordinator,
            ManagedFileWriteRequest {
                workspace_identity: workspace(),
                workspace_root: root,
                domain: ArtifactDomainKind::TextDocument,
                intent: AgentEditIntent {
                    path: PathBuf::from("src.txt"),
                    snapshot_id: Some(first_read.snapshot_id),
                    operation: AgentEditOperation::ReplaceRange {
                        range: text_range_of(base, "TARGET"),
                        old_text: "TARGET".to_string(),
                        new_text: "REPLACED".to_string(),
                    },
                },
            },
        );

        assert!(matches!(
            result,
            EditResult::AppliedWithWarning {
                warning: ArtifactEditWarning::RebasedOverNonOverlappingChange { .. },
                ..
            }
        ));
        assert_eq!(
            fs::read_to_string(&path).expect("read result"),
            "intro\nheader\nalpha\nREPLACED\nomega\nfooter\noutro\n"
        );
    }

    #[test]
    fn managed_file_apply_rejects_external_overlap() {
        let root = test_root("conflict");
        let path = root.join("src.txt");
        fs::write(&path, "one\ntwo\nthree\n").expect("write fixture");
        let mut coordinator = ArtifactEditCoordinator::new();
        let first_read = ManagedFileIo::read_artifact(
            &mut coordinator,
            ManagedFileReadRequest {
                workspace_identity: workspace(),
                workspace_root: root.clone(),
                path: PathBuf::from("src.txt"),
                domain: ArtifactDomainKind::TextDocument,
            },
        )
        .expect("read artifact");
        fs::write(&path, "one\nTWO\nthree\n").expect("external write");

        let result = ManagedFileIo::apply_edit(
            &mut coordinator,
            ManagedFileWriteRequest {
                workspace_identity: workspace(),
                workspace_root: root,
                domain: ArtifactDomainKind::TextDocument,
                intent: AgentEditIntent {
                    path: PathBuf::from("src.txt"),
                    snapshot_id: Some(first_read.snapshot_id),
                    operation: AgentEditOperation::ReplaceText {
                        old_text: "two".to_string(),
                        new_text: "deux".to_string(),
                    },
                },
            },
        );

        assert!(matches!(
            result,
            EditResult::Rejected {
                reason: ArtifactEditError::Conflict { .. }
            }
        ));
        assert_eq!(
            fs::read_to_string(&path).expect("read result"),
            "one\nTWO\nthree\n"
        );
    }

    #[test]
    fn managed_file_read_rejects_path_escape() {
        let root = test_root("escape");
        let mut coordinator = ArtifactEditCoordinator::new();
        let result = ManagedFileIo::read_artifact(
            &mut coordinator,
            ManagedFileReadRequest {
                workspace_identity: workspace(),
                workspace_root: root,
                path: PathBuf::from("../outside.txt"),
                domain: ArtifactDomainKind::TextDocument,
            },
        );

        assert!(matches!(
            result,
            Err(ArtifactEditError::InvalidOperation { .. })
        ));
    }

    #[test]
    fn managed_file_write_creates_new_text_file() {
        let root = test_root("create");
        let path = root.join("nested").join("created.txt");
        let mut coordinator = ArtifactEditCoordinator::new();

        let result = ManagedFileIo::apply_edit(
            &mut coordinator,
            ManagedFileWriteRequest {
                workspace_identity: workspace(),
                workspace_root: root,
                domain: ArtifactDomainKind::TextDocument,
                intent: AgentEditIntent {
                    path: PathBuf::from("nested/created.txt"),
                    snapshot_id: None,
                    operation: AgentEditOperation::WriteArtifact {
                        content: ArtifactContent::Text("created through arroba\n".to_string()),
                    },
                },
            },
        );

        assert!(matches!(result, EditResult::Applied { .. }));
        assert_eq!(
            fs::read_to_string(&path).expect("created file should read"),
            "created through arroba\n"
        );
    }

    #[test]
    fn managed_file_write_creates_new_opaque_file() {
        let root = test_root("create-opaque");
        let path = root.join("assets").join("image.bin");
        let mut coordinator = ArtifactEditCoordinator::new();

        let result = ManagedFileIo::apply_edit(
            &mut coordinator,
            ManagedFileWriteRequest {
                workspace_identity: workspace(),
                workspace_root: root,
                domain: ArtifactDomainKind::OpaqueBlob,
                intent: AgentEditIntent {
                    path: PathBuf::from("assets/image.bin"),
                    snapshot_id: None,
                    operation: AgentEditOperation::WriteArtifact {
                        content: ArtifactContent::Bytes(vec![0, 159, 255, 10]),
                    },
                },
            },
        );

        assert!(matches!(result, EditResult::Applied { .. }));
        assert_eq!(
            fs::read(&path).expect("created opaque file should read"),
            vec![0, 159, 255, 10]
        );
    }

    #[test]
    fn managed_file_opaque_stale_write_rejects_and_preserves_external_bytes() {
        let root = test_root("opaque-conflict");
        let path = root.join("asset.bin");
        fs::write(&path, [1, 2, 3]).expect("write fixture");
        let mut coordinator = ArtifactEditCoordinator::new();
        let first_read = ManagedFileIo::read_artifact(
            &mut coordinator,
            ManagedFileReadRequest {
                workspace_identity: workspace(),
                workspace_root: root.clone(),
                path: PathBuf::from("asset.bin"),
                domain: ArtifactDomainKind::OpaqueBlob,
            },
        )
        .expect("read opaque artifact");
        fs::write(&path, [1, 2, 9]).expect("external write");

        let result = ManagedFileIo::apply_edit(
            &mut coordinator,
            ManagedFileWriteRequest {
                workspace_identity: workspace(),
                workspace_root: root,
                domain: ArtifactDomainKind::OpaqueBlob,
                intent: AgentEditIntent {
                    path: PathBuf::from("asset.bin"),
                    snapshot_id: Some(first_read.snapshot_id),
                    operation: AgentEditOperation::WriteArtifact {
                        content: ArtifactContent::Bytes(vec![4, 5, 6]),
                    },
                },
            },
        );

        assert!(matches!(
            result,
            EditResult::Rejected {
                reason: ArtifactEditError::Conflict { .. }
            }
        ));
        assert_eq!(fs::read(path).expect("read result"), vec![1, 2, 9]);
    }

    #[test]
    fn managed_file_write_rejects_arroba_owned_instruction_policy() {
        let root = test_root("reject-policy");
        let policy_path = root.join(crate::provider::MANAGED_IO_INSTRUCTIONS_SOURCE_PATH);
        fs::create_dir_all(policy_path.parent().unwrap()).expect("create policy parent");
        fs::write(
            root.join("apps/kernel/Cargo.toml"),
            "[package]\nname = \"arroba-kernel\"\n",
        )
        .expect("write daemon manifest marker");
        fs::write(&policy_path, "original policy\n").expect("write policy marker");
        let mut coordinator = ArtifactEditCoordinator::new();

        let result = ManagedFileIo::apply_edit(
            &mut coordinator,
            ManagedFileWriteRequest {
                workspace_identity: workspace(),
                workspace_root: root.clone(),
                domain: ArtifactDomainKind::TextDocument,
                intent: AgentEditIntent {
                    path: PathBuf::from(crate::provider::MANAGED_IO_INSTRUCTIONS_SOURCE_PATH),
                    snapshot_id: None,
                    operation: AgentEditOperation::WriteArtifact {
                        content: ArtifactContent::Text("agent override\n".to_string()),
                    },
                },
            },
        );

        assert!(matches!(
            result,
            EditResult::Rejected {
                reason: ArtifactEditError::InvalidOperation { .. }
            }
        ));
        assert_eq!(
            fs::read_to_string(policy_path).unwrap(),
            "original policy\n"
        );
    }
}
