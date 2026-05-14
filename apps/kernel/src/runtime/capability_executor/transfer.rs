//! Transferred-file capability execution and artifact history recording.

use std::collections::BTreeMap;

use crate::artifacts::{OperationalArtifactStore, StoreArtifactRequest};
use crate::capability::{FileTransferService, StoreTransferredFileRequest};
use crate::error::DaemonError;
use crate::history::{HistoryEvent, HistoryEventKind, HistoryEventRole, HistoryEventTurnContext};
use crate::local::{LocalDaemonResponse, StoreTransferredFileCapabilityRequest};

use super::context::CapabilityContext;

pub(super) fn store_transferred_file(
    context: CapabilityContext,
    request: StoreTransferredFileCapabilityRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    let _claim = context.workspace_coordinator.acquire_worktree_write_claim(
        context.workspace_id.clone(),
        context.worktree_root.display().to_string(),
        request.session_id.clone(),
        Some(request.attachment_id.clone()),
        "transfer_store",
    )?;
    let artifact_root = context.artifact_root("transfers");
    let result = FileTransferService::new().store_file(StoreTransferredFileRequest::new(
        request.session_id.clone(),
        request.attachment_id.clone(),
        context.worktree_root.clone(),
        artifact_root,
        request.source_path,
        request.display_name,
    ))?;

    let artifact_store = OperationalArtifactStore::open(
        context.operational_artifact_root,
        context.operational_artifact_index_path,
    )?;
    let mut metadata = transfer_metadata(&result);
    let artifact_record = artifact_store.store_existing_file(StoreArtifactRequest {
        source_path: result.stored_path.clone(),
        display_name: result.display_name.clone(),
        source_kind: "transfer".to_string(),
        session_id: Some(request.session_id.clone()),
        attachment_id: Some(request.attachment_id.clone()),
        workspace_id: Some(context.workspace_id.clone()),
        worktree_path: Some(context.worktree_root.display().to_string()),
        metadata: metadata.clone(),
    })?;
    metadata.insert(
        "artifact_id".to_string(),
        serde_json::Value::String(artifact_record.artifact_id.clone()),
    );
    metadata.insert(
        "sha256".to_string(),
        serde_json::Value::String(artifact_record.sha256.clone()),
    );
    metadata.insert(
        "size_bytes".to_string(),
        serde_json::Value::Number(serde_json::Number::from(artifact_record.size_bytes)),
    );

    let sequence = context.operational_history_store.reserve_sequence();
    let mut event = HistoryEvent::operational(
        sequence,
        HistoryEventKind::ArtifactStored,
        Some(HistoryEventRole::System),
        Some(format!(
            "stored artifact `{}` ({})",
            artifact_record.display_name, artifact_record.artifact_id
        )),
        metadata,
        HistoryEventTurnContext {
            workspace_id: Some(context.workspace_id.clone()),
            session_id: Some(request.session_id.clone()),
            worktree_path: Some(context.worktree_root.display().to_string()),
            ..HistoryEventTurnContext::default()
        },
    );
    event.content_ref = Some(format!("artifact://sha256/{}", artifact_record.sha256));
    context.operational_history_store.append(&event)?;
    if context.history_archive_enabled {
        context
            .operational_history_store
            .enqueue_archive_events(&[event])?;
    }

    Ok(LocalDaemonResponse::FileTransferred { result })
}

fn transfer_metadata(
    result: &crate::capability::StoredTransferArtifact,
) -> BTreeMap<String, serde_json::Value> {
    let mut metadata = BTreeMap::new();
    metadata.insert(
        "transfer_artifact_id".to_string(),
        serde_json::Value::String(result.artifact_id.clone()),
    );
    metadata.insert(
        "stored_path".to_string(),
        serde_json::Value::String(result.stored_path.display().to_string()),
    );
    metadata.insert(
        "stored_name".to_string(),
        serde_json::Value::String(result.stored_name.clone()),
    );
    metadata
}
