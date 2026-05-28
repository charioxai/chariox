//! Workspace live sync apply-patch envelope parsing.

use super::*;

#[derive(Debug, Clone)]
pub(in crate::runtime::state) enum ManagedPatchOperation {
    Add {
        path: PathBuf,
        content: String,
    },
    Update {
        path: PathBuf,
        old_text: String,
        new_text: String,
    },
    Delete {
        path: PathBuf,
    },
    Move {
        from_path: PathBuf,
        to_path: PathBuf,
        old_text: Option<String>,
        new_text: Option<String>,
    },
}

pub(in crate::runtime::state) fn parse_workspace_live_sync_apply_patch(
    patch_text: &str,
) -> Result<Vec<ManagedPatchOperation>, DaemonError> {
    let lines = patch_text.lines().collect::<Vec<_>>();
    if lines.first().map(|line| line.trim()) != Some("*** Begin Patch")
        || lines.last().map(|line| line.trim()) != Some("*** End Patch")
    {
        return Err(DaemonError::LocalTransport {
            operation: "runtime_tool_apply_patch",
            message: "patch_text must use the apply_patch envelope".to_string(),
        });
    }
    let mut operations = Vec::new();
    let mut index = 1usize;
    while index + 1 < lines.len() {
        let line = lines[index];
        if let Some(path) = line.strip_prefix("*** Add File: ") {
            index += 1;
            let mut body = Vec::new();
            while index < lines.len() && !lines[index].starts_with("*** ") {
                let line = lines[index];
                body.push(line.strip_prefix('+').unwrap_or(line).to_string());
                index += 1;
            }
            operations.push(ManagedPatchOperation::Add {
                path: PathBuf::from(path.trim()),
                content: join_patch_lines(&body),
            });
            continue;
        }
        if let Some(path) = line.strip_prefix("*** Delete File: ") {
            operations.push(ManagedPatchOperation::Delete {
                path: PathBuf::from(path.trim()),
            });
            index += 1;
            continue;
        }
        if line.starts_with("*** Move to: ") {
            return Err(DaemonError::LocalTransport {
                operation: "runtime_tool_apply_patch",
                message: "move hunks must follow an update file header".to_string(),
            });
        }
        if let Some(path) = line.strip_prefix("*** Update File: ") {
            index += 1;
            let mut move_to = None;
            if index < lines.len() {
                if let Some(target) = lines[index].strip_prefix("*** Move to: ") {
                    move_to = Some(PathBuf::from(target.trim()));
                    index += 1;
                }
            }
            let mut old_lines = Vec::new();
            let mut new_lines = Vec::new();
            while index < lines.len() && !lines[index].starts_with("*** ") {
                let line = lines[index];
                if line.starts_with("@@") || line.starts_with('\\') {
                    index += 1;
                    continue;
                }
                if let Some(rest) = line.strip_prefix('-') {
                    old_lines.push(rest.to_string());
                } else if let Some(rest) = line.strip_prefix('+') {
                    new_lines.push(rest.to_string());
                } else {
                    let rest = line.strip_prefix(' ').unwrap_or(line);
                    old_lines.push(rest.to_string());
                    new_lines.push(rest.to_string());
                }
                index += 1;
            }
            match move_to {
                Some(to_path) => operations.push(ManagedPatchOperation::Move {
                    from_path: PathBuf::from(path.trim()),
                    to_path,
                    old_text: (!old_lines.is_empty()).then(|| join_patch_lines(&old_lines)),
                    new_text: (!new_lines.is_empty()).then(|| join_patch_lines(&new_lines)),
                }),
                None => {
                    if old_lines.is_empty() {
                        return Err(DaemonError::LocalTransport {
                            operation: "runtime_tool_apply_patch",
                            message: format!("update hunk for `{}` has no old text", path.trim()),
                        });
                    }
                    operations.push(ManagedPatchOperation::Update {
                        path: PathBuf::from(path.trim()),
                        old_text: join_patch_lines(&old_lines),
                        new_text: join_patch_lines(&new_lines),
                    });
                }
            }
            continue;
        }
        if line.trim().is_empty() {
            index += 1;
            continue;
        }
        return Err(DaemonError::LocalTransport {
            operation: "runtime_tool_apply_patch",
            message: format!("unsupported patch line `{line}`"),
        });
    }
    if operations.is_empty() {
        return Err(DaemonError::LocalTransport {
            operation: "runtime_tool_apply_patch",
            message: "patch_text did not contain any supported file operations".to_string(),
        });
    }
    Ok(operations)
}

pub(in crate::runtime::state) fn join_patch_lines(lines: &[String]) -> String {
    if lines.is_empty() {
        return String::new();
    }
    let mut text = lines.join("\n");
    text.push('\n');
    text
}
