//! Managed-I/O runtime tool argument normalization.

use super::*;

pub(in crate::runtime::state) fn managed_io_edit_operation_from_args(
    args: crate::transport::runtime_tools::ManagedEditArtifactArgs,
) -> Result<crate::io::AgentEditOperation, DaemonError> {
    match (args.range, args.old_text) {
        (Some(range), Some(old_text)) => Ok(crate::io::AgentEditOperation::ReplaceRange {
            range: crate::io::TextRange::new(range.start, range.end),
            old_text,
            new_text: args.new_text,
        }),
        (None, Some(old_text)) => Ok(crate::io::AgentEditOperation::ReplaceText {
            old_text,
            new_text: args.new_text,
        }),
        (Some(_), None) => Err(DaemonError::LocalTransport {
            operation: "runtime_tool_edit_artifact",
            message: "range edits require old_text".to_string(),
        }),
        (None, None) => Err(DaemonError::LocalTransport {
            operation: "runtime_tool_edit_artifact",
            message: "managed text edits require old_text or range+old_text".to_string(),
        }),
    }
}

pub(in crate::runtime::state) fn managed_io_write_content_from_args(
    operation: &'static str,
    domain: crate::io::ArtifactDomainKind,
    args: &crate::transport::runtime_tools::ManagedWriteArtifactArgs,
) -> Result<crate::io::ArtifactContent, DaemonError> {
    match domain {
        crate::io::ArtifactDomainKind::TextDocument
        | crate::io::ArtifactDomainKind::StructuredDocument => {
            let Some(text) = args.content_text.clone() else {
                return Err(DaemonError::LocalTransport {
                    operation,
                    message: "content_text is required for text and structured artifacts"
                        .to_string(),
                });
            };
            Ok(crate::io::ArtifactContent::Text(text))
        }
        crate::io::ArtifactDomainKind::OpaqueBlob => {
            let Some(content_base64) = args.content_base64.as_deref() else {
                return Err(DaemonError::LocalTransport {
                    operation,
                    message: "content_base64 is required for opaque artifacts".to_string(),
                });
            };
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(content_base64)
                .map_err(|error| DaemonError::LocalTransport {
                    operation,
                    message: format!("content_base64 is not valid base64: {error}"),
                })?;
            Ok(crate::io::ArtifactContent::Bytes(bytes))
        }
    }
}

pub(in crate::runtime::state) fn managed_io_snapshot_id_from_arg(
    snapshot_id: Option<String>,
) -> Option<crate::io::ArtifactSnapshotId> {
    snapshot_id
        .filter(|snapshot_id| {
            let snapshot_id = snapshot_id.trim();
            let sentinel = snapshot_id.to_ascii_lowercase();
            !snapshot_id.is_empty()
                && sentinel != "__arroba_create__"
                && sentinel != "create"
                && sentinel != "new"
                && sentinel != "absent"
                && snapshot_id != "*"
        })
        .map(crate::io::ArtifactSnapshotId::new)
}

pub(in crate::runtime::state) fn managed_io_write_snapshot_id_from_arg(
    snapshot_id: Option<String>,
    path: &Path,
) -> Option<crate::io::ArtifactSnapshotId> {
    let snapshot_id = managed_io_snapshot_id_from_arg(snapshot_id)?;
    let snapshot_value = snapshot_id.as_str();
    let path_value = path.to_string_lossy();
    if snapshot_value.starts_with("snap:") && !snapshot_value.contains(path_value.as_ref()) {
        return None;
    }
    Some(snapshot_id)
}
