//! Bounded artifact pages shared by ordinary and forwarded workspace reads.

use super::*;

pub(in crate::runtime::state) fn workspace_live_sync_read_payload(
    read: crate::io::ArtifactReadResult,
    offset_bytes: usize,
    max_bytes: Option<usize>,
) -> Result<serde_json::Value, DaemonError> {
    // Even control characters can expand sixfold in JSON. Leave room for
    // metadata below the MCP 64 KiB text limit so no page loses its content.
    const MAX_PAGE_BYTES: usize = 8 * 1024;
    let max_bytes = max_bytes.unwrap_or(MAX_PAGE_BYTES);
    let invalid_range = |message: &str| DaemonError::LocalTransport {
        operation: "runtime_tool_read_artifact",
        message: message.to_string(),
    };
    if !(4..=MAX_PAGE_BYTES).contains(&max_bytes) {
        return Err(invalid_range("max_bytes must be between 4 and 8192"));
    }
    let total_bytes = match &read.content {
        crate::io::ArtifactContent::Text(text) => text.len(),
        crate::io::ArtifactContent::Bytes(bytes) => bytes.len(),
    };
    if offset_bytes > total_bytes {
        return Err(invalid_range(
            "offset_bytes exceeds the artifact size; reread from offset 0",
        ));
    }
    let mut end = offset_bytes.saturating_add(max_bytes).min(total_bytes);
    let mut payload = serde_json::json!({
        "artifact_id": read.artifact_id.as_str(),
        "path": read.path.to_string_lossy(),
        "domain": workspace_live_sync_domain_name(read.domain),
        "version": read.version.value(),
        "snapshot_id": read.snapshot_id.as_str(),
        "offset_bytes": offset_bytes,
        "total_bytes": total_bytes,
    });
    match read.content {
        crate::io::ArtifactContent::Text(text) => {
            if !text.is_char_boundary(offset_bytes) {
                return Err(invalid_range("offset_bytes must be a UTF-8 character boundary; use next_offset_bytes from the previous page"));
            }
            while !text.is_char_boundary(end) {
                end -= 1;
            }
            payload["content_text"] =
                serde_json::Value::String(text[offset_bytes..end].to_string());
        }
        crate::io::ArtifactContent::Bytes(bytes) => {
            payload["byte_count"] = serde_json::json!(total_bytes);
            payload["content_base64"] = serde_json::Value::String(
                base64::engine::general_purpose::STANDARD.encode(&bytes[offset_bytes..end]),
            );
        }
    }
    payload["next_offset_bytes"] = if end < total_bytes {
        serde_json::json!(end)
    } else {
        serde_json::Value::Null
    };
    Ok(payload)
}

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::{Digest, Sha256};

    #[test]
    fn workspace_artifact_paging_protocol_snapshot_and_hash() {
        let snapshot: serde_json::Value = serde_json::from_str(include_str!(
            "../../../../../../fixtures/workspace-artifact-paging-protocol.json"
        ))
        .expect("paging protocol fixture");
        assert_eq!(
            snapshot["local"],
            crate::local::LOCAL_DAEMON_PROTOCOL_VERSION
        );
        assert_eq!(
            snapshot["relayPeer"],
            crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION
        );
        assert_eq!(
            format!(
                "{:x}",
                Sha256::digest(serde_json::to_vec(&snapshot).unwrap())
            ),
            "6c4dffc65e1e96014e5dd4510b7d3c1ea04dcbb4639cf51ff338abb8ebff8668"
        );
        let args: crate::transport::runtime_tools::WorkspaceLiveSyncReadArtifactArgs =
            serde_json::from_value(snapshot["arguments"].clone()).unwrap();
        assert_eq!(serde_json::to_value(&args).unwrap(), snapshot["arguments"]);
        let payload = workspace_live_sync_read_payload(
            crate::io::ArtifactReadResult {
                artifact_id: crate::io::ArtifactId::new("artifact:opaque"),
                path: PathBuf::from(&args.path),
                domain: crate::io::ArtifactDomainKind::OpaqueBlob,
                version: crate::io::ArtifactVersion::initial(),
                snapshot_id: crate::io::ArtifactSnapshotId::new("snap:opaque:1"),
                content: crate::io::ArtifactContent::Bytes(
                    (0..9000).map(|index| (index % 256) as u8).collect(),
                ),
            },
            args.offset_bytes,
            args.max_bytes,
        )
        .unwrap();
        assert_eq!(payload, snapshot["payload"]);
        let spec = crate::transport::runtime_tools::workspace_live_sync_runtime_tool_specs()
            .into_iter()
            .find(|spec| spec.name == snapshot["tool"])
            .unwrap();
        assert_eq!(
            spec.input_schema["properties"]["offset_bytes"]["type"],
            "integer"
        );
        assert_eq!(
            spec.input_schema["properties"]["max_bytes"]["maximum"],
            8192
        );
    }
}
