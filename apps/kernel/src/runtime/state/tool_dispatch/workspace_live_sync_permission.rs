use crate::error::DaemonError;

pub(super) fn workspace_live_sync_tool_requires_popup(tool_name: &str) -> bool {
    matches!(
        tool_name,
        crate::transport::runtime_tools::EDIT_ARTIFACT_TOOL
            | crate::transport::runtime_tools::APPLY_PATCH_TOOL
            | crate::transport::runtime_tools::DELETE_ARTIFACT_TOOL
            | crate::transport::runtime_tools::MOVE_ARTIFACT_TOOL
            | crate::transport::runtime_tools::WRITE_ARTIFACT_TOOL
    )
}

pub(super) fn workspace_live_sync_permission_interaction(
    agent_id: &str,
    tool_name: &str,
    arguments: &serde_json::Value,
) -> Result<crate::session::RuntimeInteraction, DaemonError> {
    let (title, message) = workspace_live_sync_permission_message(tool_name, arguments)?;
    Ok(crate::session::RuntimeInteraction::new(
        format!(
            "workspace-live-sync-permission-{agent_id}-{}",
            crate::session::unix_epoch_ms()
        ),
        agent_id,
        crate::session::RuntimeInteractionKind::Permission,
        crate::session::RuntimeInteractionLevel::Warning,
        Some(title),
        message,
        vec![
            crate::session::RuntimeInteractionChoice::new(
                "allow",
                "Allow",
                "allow",
                Some(crate::session::RuntimeInteractionChoiceStyle::Primary),
            ),
            crate::session::RuntimeInteractionChoice::new(
                "deny",
                "Deny",
                "deny",
                Some(crate::session::RuntimeInteractionChoiceStyle::Danger),
            ),
        ],
        None,
        None,
        None,
    ))
}

fn workspace_live_sync_permission_message(
    tool_name: &str,
    arguments: &serde_json::Value,
) -> Result<(String, String), DaemonError> {
    match tool_name {
        crate::transport::runtime_tools::EDIT_ARTIFACT_TOOL => {
            let args = serde_json::from_value::<
                crate::transport::runtime_tools::WorkspaceLiveSyncEditArtifactArgs,
            >(arguments.clone())
            .map_err(|error| DaemonError::LocalTransport {
                operation: "workspace_live_sync_permission_message",
                message: format!("invalid workspace live sync edit arguments: {error}"),
            })?;
            Ok((
                "Workspace live sync edit approval".to_string(),
                format!(
                    "Allow editing `{}` through Chariox workspace live sync?",
                    args.path
                ),
            ))
        }
        crate::transport::runtime_tools::APPLY_PATCH_TOOL => {
            let args = serde_json::from_value::<
                crate::transport::runtime_tools::WorkspaceLiveSyncApplyPatchArgs,
            >(arguments.clone())
            .map_err(|error| DaemonError::LocalTransport {
                operation: "workspace_live_sync_permission_message",
                message: format!("invalid workspace live sync apply_patch arguments: {error}"),
            })?;
            let patch_preview = args
                .patch_text
                .lines()
                .find(|line| !line.trim().is_empty())
                .unwrap_or("patch");
            Ok((
                "Workspace live sync patch approval".to_string(),
                format!(
                    "Allow applying this workspace live sync patch? First patch line: `{}`",
                    patch_preview
                ),
            ))
        }
        crate::transport::runtime_tools::DELETE_ARTIFACT_TOOL => {
            let args = serde_json::from_value::<
                crate::transport::runtime_tools::WorkspaceLiveSyncDeleteArtifactArgs,
            >(arguments.clone())
            .map_err(|error| DaemonError::LocalTransport {
                operation: "workspace_live_sync_permission_message",
                message: format!("invalid managed delete arguments: {error}"),
            })?;
            Ok((
                "Workspace live sync delete approval".to_string(),
                format!(
                    "Allow deleting `{}` through Chariox workspace live sync?",
                    args.path
                ),
            ))
        }
        crate::transport::runtime_tools::MOVE_ARTIFACT_TOOL => {
            let args = serde_json::from_value::<
                crate::transport::runtime_tools::WorkspaceLiveSyncMoveArtifactArgs,
            >(arguments.clone())
            .map_err(|error| DaemonError::LocalTransport {
                operation: "workspace_live_sync_permission_message",
                message: format!("invalid managed move arguments: {error}"),
            })?;
            Ok((
                "Workspace live sync move approval".to_string(),
                format!(
                    "Allow moving `{}` to `{}` through Chariox workspace live sync?",
                    args.from_path, args.to_path
                ),
            ))
        }
        crate::transport::runtime_tools::WRITE_ARTIFACT_TOOL => {
            let args = serde_json::from_value::<
                crate::transport::runtime_tools::WorkspaceLiveSyncWriteArtifactArgs,
            >(arguments.clone())
            .map_err(|error| DaemonError::LocalTransport {
                operation: "workspace_live_sync_permission_message",
                message: format!("invalid workspace live sync write arguments: {error}"),
            })?;
            Ok((
                "Workspace live sync write approval".to_string(),
                format!(
                    "Allow writing `{}` through Chariox workspace live sync?",
                    args.path
                ),
            ))
        }
        other => Err(DaemonError::LocalTransport {
            operation: "workspace_live_sync_permission_message",
            message: format!("unsupported workspace live sync permission tool `{other}`"),
        }),
    }
}
