use crate::error::DaemonError;

pub(super) fn managed_io_tool_requires_popup(tool_name: &str) -> bool {
    matches!(
        tool_name,
        crate::transport::runtime_tools::EDIT_ARTIFACT_TOOL
            | crate::transport::runtime_tools::APPLY_PATCH_TOOL
            | crate::transport::runtime_tools::DELETE_ARTIFACT_TOOL
            | crate::transport::runtime_tools::MOVE_ARTIFACT_TOOL
            | crate::transport::runtime_tools::WRITE_ARTIFACT_TOOL
    )
}

pub(super) fn managed_io_permission_interaction(
    agent_id: &str,
    tool_name: &str,
    arguments: &serde_json::Value,
) -> Result<crate::session::RuntimeInteraction, DaemonError> {
    let (title, message) = managed_io_permission_message(tool_name, arguments)?;
    Ok(crate::session::RuntimeInteraction::new(
        format!(
            "managed-io-permission-{agent_id}-{}",
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

fn managed_io_permission_message(
    tool_name: &str,
    arguments: &serde_json::Value,
) -> Result<(String, String), DaemonError> {
    match tool_name {
        crate::transport::runtime_tools::EDIT_ARTIFACT_TOOL => {
            let args = serde_json::from_value::<
                crate::transport::runtime_tools::ManagedEditArtifactArgs,
            >(arguments.clone())
            .map_err(|error| DaemonError::LocalTransport {
                operation: "managed_io_permission_message",
                message: format!("invalid managed edit arguments: {error}"),
            })?;
            Ok((
                "Managed I/O edit approval".to_string(),
                format!("Allow editing `{}` through Arroba managed I/O?", args.path),
            ))
        }
        crate::transport::runtime_tools::APPLY_PATCH_TOOL => {
            let args = serde_json::from_value::<
                crate::transport::runtime_tools::ManagedApplyPatchArgs,
            >(arguments.clone())
            .map_err(|error| DaemonError::LocalTransport {
                operation: "managed_io_permission_message",
                message: format!("invalid managed apply_patch arguments: {error}"),
            })?;
            let patch_preview = args
                .patch_text
                .lines()
                .find(|line| !line.trim().is_empty())
                .unwrap_or("patch");
            Ok((
                "Managed I/O patch approval".to_string(),
                format!(
                    "Allow applying this managed I/O patch? First patch line: `{}`",
                    patch_preview
                ),
            ))
        }
        crate::transport::runtime_tools::DELETE_ARTIFACT_TOOL => {
            let args = serde_json::from_value::<
                crate::transport::runtime_tools::ManagedDeleteArtifactArgs,
            >(arguments.clone())
            .map_err(|error| DaemonError::LocalTransport {
                operation: "managed_io_permission_message",
                message: format!("invalid managed delete arguments: {error}"),
            })?;
            Ok((
                "Managed I/O delete approval".to_string(),
                format!("Allow deleting `{}` through Arroba managed I/O?", args.path),
            ))
        }
        crate::transport::runtime_tools::MOVE_ARTIFACT_TOOL => {
            let args = serde_json::from_value::<
                crate::transport::runtime_tools::ManagedMoveArtifactArgs,
            >(arguments.clone())
            .map_err(|error| DaemonError::LocalTransport {
                operation: "managed_io_permission_message",
                message: format!("invalid managed move arguments: {error}"),
            })?;
            Ok((
                "Managed I/O move approval".to_string(),
                format!(
                    "Allow moving `{}` to `{}` through Arroba managed I/O?",
                    args.from_path, args.to_path
                ),
            ))
        }
        crate::transport::runtime_tools::WRITE_ARTIFACT_TOOL => {
            let args = serde_json::from_value::<
                crate::transport::runtime_tools::ManagedWriteArtifactArgs,
            >(arguments.clone())
            .map_err(|error| DaemonError::LocalTransport {
                operation: "managed_io_permission_message",
                message: format!("invalid managed write arguments: {error}"),
            })?;
            Ok((
                "Managed I/O write approval".to_string(),
                format!("Allow writing `{}` through Arroba managed I/O?", args.path),
            ))
        }
        other => Err(DaemonError::LocalTransport {
            operation: "managed_io_permission_message",
            message: format!("unsupported managed I/O permission tool `{other}`"),
        }),
    }
}
