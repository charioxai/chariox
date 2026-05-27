//! Workspace live sync workspace file state access.

use super::*;

pub(in crate::runtime::state) fn managed_patch_state(
    workspace_root: &PathBuf,
    path: &PathBuf,
    before_states: &mut BTreeMap<PathBuf, Option<String>>,
    final_states: &mut BTreeMap<PathBuf, Option<String>>,
) -> Result<Option<String>, DaemonError> {
    if let Some(current) = final_states.get(path) {
        return Ok(current.clone());
    }
    let current = workspace_live_sync_read_optional_text(workspace_root, path)?;
    before_states
        .entry(path.clone())
        .or_insert_with(|| current.clone());
    final_states.insert(path.clone(), current.clone());
    Ok(current)
}

pub(in crate::runtime::state) fn managed_whole_file_state(
    workspace_root: &PathBuf,
    path: &PathBuf,
    domain: crate::io::ArtifactDomainKind,
    before_states: &mut BTreeMap<PathBuf, Option<crate::io::ArtifactContent>>,
    final_states: &mut BTreeMap<PathBuf, Option<crate::io::ArtifactContent>>,
) -> Result<Option<crate::io::ArtifactContent>, DaemonError> {
    if let Some(current) = final_states.get(path) {
        return Ok(current.clone());
    }
    let current = workspace_live_sync_read_optional_content(workspace_root, path, domain)?;
    before_states
        .entry(path.clone())
        .or_insert_with(|| current.clone());
    final_states.insert(path.clone(), current.clone());
    Ok(current)
}

pub(in crate::runtime::state) fn workspace_live_sync_validate_patch_path(
    workspace_root: &PathBuf,
    path: &PathBuf,
) -> Result<(), DaemonError> {
    let _ = workspace_live_sync_diff_workspace_path(workspace_root, path).ok_or_else(|| DaemonError::LocalTransport {
        operation: "runtime_tool_apply_patch",
        message: "managed patch paths must be workspace-relative and cannot escape the workspace root".to_string(),
    })?;
    if path == std::path::Path::new(crate::provider::WORKSPACE_LIVE_SYNC_INSTRUCTIONS_SOURCE_PATH)
        && workspace_live_sync_is_arroba_source_workspace(workspace_root)
    {
        return Err(DaemonError::LocalTransport {
            operation: "runtime_tool_apply_patch",
            message: format!(
                "the Arroba workspace live sync instruction policy `{}` is owned by Arroba and cannot be edited through managed artifact I/O",
                crate::provider::WORKSPACE_LIVE_SYNC_INSTRUCTIONS_SOURCE_PATH
            ),
        });
    }
    Ok(())
}

pub(in crate::runtime::state) fn workspace_live_sync_read_optional_text(
    workspace_root: &PathBuf,
    path: &PathBuf,
) -> Result<Option<String>, DaemonError> {
    let full_path = workspace_live_sync_diff_workspace_path(workspace_root, path).ok_or_else(|| {
        DaemonError::LocalTransport {
            operation: "runtime_tool_apply_patch",
            message: "managed patch paths must be workspace-relative and cannot escape the workspace root".to_string(),
        }
    })?;
    match std::fs::read_to_string(&full_path) {
        Ok(text) => Ok(Some(text)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(DaemonError::LocalTransport {
            operation: "runtime_tool_apply_patch",
            message: format!("failed to read `{}`: {error}", path.to_string_lossy()),
        }),
    }
}

pub(in crate::runtime::state) fn workspace_live_sync_read_optional_content(
    workspace_root: &PathBuf,
    path: &PathBuf,
    domain: crate::io::ArtifactDomainKind,
) -> Result<Option<crate::io::ArtifactContent>, DaemonError> {
    let full_path = workspace_live_sync_diff_workspace_path(workspace_root, path).ok_or_else(|| {
        DaemonError::LocalTransport {
            operation: "runtime_tool_workspace_live_sync_state",
            message:
                "managed paths must be workspace-relative and cannot escape the workspace root"
                    .to_string(),
        }
    })?;
    match std::fs::read(&full_path) {
        Ok(bytes) => match domain {
            crate::io::ArtifactDomainKind::TextDocument
            | crate::io::ArtifactDomainKind::StructuredDocument => {
                let text =
                    String::from_utf8(bytes).map_err(|error| DaemonError::LocalTransport {
                        operation: "runtime_tool_workspace_live_sync_state",
                        message: format!(
                            "failed to decode `{}` as UTF-8: {error}",
                            path.to_string_lossy()
                        ),
                    })?;
                Ok(Some(crate::io::ArtifactContent::Text(text)))
            }
            crate::io::ArtifactDomainKind::OpaqueBlob => {
                Ok(Some(crate::io::ArtifactContent::Bytes(bytes)))
            }
        },
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(DaemonError::LocalTransport {
            operation: "runtime_tool_workspace_live_sync_state",
            message: format!("failed to read `{}`: {error}", path.to_string_lossy()),
        }),
    }
}

pub(in crate::runtime::state) fn workspace_live_sync_write_final_states(
    workspace_root: &PathBuf,
    states: &BTreeMap<PathBuf, Option<String>>,
) -> Result<(), DaemonError> {
    for (path, text) in states {
        let full_path = workspace_live_sync_diff_workspace_path(workspace_root, path).ok_or_else(|| {
            DaemonError::LocalTransport {
                operation: "runtime_tool_apply_patch",
                message: "managed patch paths must be workspace-relative and cannot escape the workspace root".to_string(),
            }
        })?;
        match text {
            Some(text) => {
                if let Some(parent) = full_path.parent() {
                    std::fs::create_dir_all(parent).map_err(|error| {
                        DaemonError::LocalTransport {
                            operation: "runtime_tool_apply_patch",
                            message: format!(
                                "failed to create `{}`: {error}",
                                parent.to_string_lossy()
                            ),
                        }
                    })?;
                }
                std::fs::write(&full_path, text).map_err(|error| DaemonError::LocalTransport {
                    operation: "runtime_tool_apply_patch",
                    message: format!("failed to write `{}`: {error}", path.to_string_lossy()),
                })?;
            }
            None => match std::fs::remove_file(&full_path) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => {
                    return Err(DaemonError::LocalTransport {
                        operation: "runtime_tool_apply_patch",
                        message: format!("failed to delete `{}`: {error}", path.to_string_lossy()),
                    });
                }
            },
        }
    }
    Ok(())
}

pub(in crate::runtime::state) fn workspace_live_sync_write_final_content_states(
    workspace_root: &PathBuf,
    states: &BTreeMap<PathBuf, Option<crate::io::ArtifactContent>>,
) -> Result<(), DaemonError> {
    for (path, content) in states {
        let full_path = workspace_live_sync_diff_workspace_path(workspace_root, path).ok_or_else(|| {
            DaemonError::LocalTransport {
                operation: "runtime_tool_workspace_live_sync_state",
                message:
                    "managed paths must be workspace-relative and cannot escape the workspace root"
                        .to_string(),
            }
        })?;
        match content {
            Some(crate::io::ArtifactContent::Text(text)) => {
                if let Some(parent) = full_path.parent() {
                    std::fs::create_dir_all(parent).map_err(|error| {
                        DaemonError::LocalTransport {
                            operation: "runtime_tool_workspace_live_sync_state",
                            message: format!(
                                "failed to create `{}`: {error}",
                                parent.to_string_lossy()
                            ),
                        }
                    })?;
                }
                std::fs::write(&full_path, text).map_err(|error| DaemonError::LocalTransport {
                    operation: "runtime_tool_workspace_live_sync_state",
                    message: format!("failed to write `{}`: {error}", path.to_string_lossy()),
                })?;
            }
            Some(crate::io::ArtifactContent::Bytes(bytes)) => {
                if let Some(parent) = full_path.parent() {
                    std::fs::create_dir_all(parent).map_err(|error| {
                        DaemonError::LocalTransport {
                            operation: "runtime_tool_workspace_live_sync_state",
                            message: format!(
                                "failed to create `{}`: {error}",
                                parent.to_string_lossy()
                            ),
                        }
                    })?;
                }
                std::fs::write(&full_path, bytes).map_err(|error| DaemonError::LocalTransport {
                    operation: "runtime_tool_workspace_live_sync_state",
                    message: format!("failed to write `{}`: {error}", path.to_string_lossy()),
                })?;
            }
            None => match std::fs::remove_file(&full_path) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => {
                    return Err(DaemonError::LocalTransport {
                        operation: "runtime_tool_workspace_live_sync_state",
                        message: format!("failed to delete `{}`: {error}", path.to_string_lossy()),
                    });
                }
            },
        }
    }
    Ok(())
}
