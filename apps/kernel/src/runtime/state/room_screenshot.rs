use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};

use base64::Engine as _;

use super::*;
use crate::artifacts::{OperationalArtifactStore, StoreArtifactRequest};
use crate::local::{
    CaptureRoomEnvironmentScreenshotRequest, ReadRoomEnvironmentScreenshotChunkRequest,
    RoomEnvironmentScreenshotArtifact, RoomEnvironmentScreenshotChunk,
};
use crate::runtime::command::{KernelCaller, KernelCallerKind};
use crate::slice::{SliceRecord, SliceStatus};

const SCREENSHOT_SOURCE_KIND: &str = "room_environment_screenshot";
const SCREENSHOT_MEDIA_TYPE: &str = "image/png";
const SCREENSHOT_MAX_BYTES: u64 = 64 * 1024 * 1024;
const SCREENSHOT_CHUNK_MAX_BYTES: u32 = 128 * 1024;
static SCREENSHOT_SEQUENCE: AtomicU64 = AtomicU64::new(1);

impl KernelRuntimeState {
    pub(crate) async fn capture_room_environment_screenshot(
        &self,
        caller: &KernelCaller,
        request: CaptureRoomEnvironmentScreenshotRequest,
    ) -> Result<RoomEnvironmentScreenshotArtifact, DaemonError> {
        let session_id = screenshot_field(request.session_id, "session_id")?;
        let attachment_id = screenshot_field(request.attachment_id, "attachment_id")?;
        let slice = self
            .authorize_room_screenshot(caller, &session_id, &attachment_id)
            .await?;
        let _guard = self.owned.slice_store.guard_environment_use(
            &slice.id,
            Some(&session_id),
            "environment.screenshot.capture",
        )?;
        let response = self
            .send_room_screenshot_peer_request(
                &slice,
                RelayPeerRequest::CaptureRoomScreenshot {
                    session_id: session_id.clone(),
                    slice_id: slice.id.clone(),
                },
            )
            .await?;
        match response {
            RelayPeerResponse::RoomScreenshotCaptured {
                session_id: returned_session,
                slice_id,
                artifact,
            } if returned_session == session_id
                && slice_id == slice.id
                && screenshot_artifact_is_valid(&artifact) =>
            {
                Ok(artifact)
            }
            _ => Err(screenshot_error(
                "worker returned mismatched screenshot metadata",
            )),
        }
    }

    pub(crate) async fn read_room_environment_screenshot_chunk(
        &self,
        caller: &KernelCaller,
        request: ReadRoomEnvironmentScreenshotChunkRequest,
    ) -> Result<RoomEnvironmentScreenshotChunk, DaemonError> {
        let session_id = screenshot_field(request.session_id, "session_id")?;
        let attachment_id = screenshot_field(request.attachment_id, "attachment_id")?;
        let artifact_id = screenshot_field(request.artifact_id, "artifact_id")?;
        if request.max_bytes == 0 || request.max_bytes > SCREENSHOT_CHUNK_MAX_BYTES {
            return Err(screenshot_error(
                "screenshot chunk size must be between 1 and 131072 bytes",
            ));
        }
        let slice = self
            .authorize_room_screenshot(caller, &session_id, &attachment_id)
            .await?;
        let _guard = self.owned.slice_store.guard_environment_use(
            &slice.id,
            Some(&session_id),
            "environment.screenshot.read",
        )?;
        let response = self
            .send_room_screenshot_peer_request(
                &slice,
                RelayPeerRequest::ReadRoomScreenshotChunk {
                    session_id: session_id.clone(),
                    slice_id: slice.id.clone(),
                    artifact_id: artifact_id.clone(),
                    offset: request.offset,
                    max_bytes: request.max_bytes,
                },
            )
            .await?;
        match response {
            RelayPeerResponse::RoomScreenshotChunk {
                session_id: returned_session,
                slice_id,
                chunk,
            } if returned_session == session_id
                && slice_id == slice.id
                && chunk.artifact_id == artifact_id
                && chunk.offset == request.offset =>
            {
                Ok(chunk)
            }
            _ => Err(screenshot_error(
                "worker returned a mismatched screenshot chunk",
            )),
        }
    }

    pub(crate) async fn execute_bound_room_screenshot_capture(
        &self,
        authenticated_kernel_id: &str,
        authenticated_public_key: &str,
        session_id: &str,
        slice_id: &str,
    ) -> Result<RoomEnvironmentScreenshotArtifact, DaemonError> {
        let config = self.authorize_bound_room_screenshot(
            authenticated_kernel_id,
            authenticated_public_key,
            session_id,
            slice_id,
        )?;
        let root = config.operational_artifact_root();
        let staging = root.join("staging");
        std::fs::create_dir_all(&staging).map_err(|error| {
            screenshot_error(&format!(
                "failed to create screenshot staging directory: {error}"
            ))
        })?;
        let sequence = SCREENSHOT_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let created_at_ms = crate::session::unix_epoch_ms();
        let display_name = format!("chariox-room-{created_at_ms}-{sequence}.png");
        let staging_path = staging.join(format!(
            ".room-screenshot-{}-{created_at_ms}-{sequence}.png",
            std::process::id()
        ));
        if let Err(error) =
            super::tool_dispatch::capture_room_environment_screenshot(&staging_path).await
        {
            let _ = std::fs::remove_file(&staging_path);
            return Err(error);
        }
        let size_bytes = std::fs::metadata(&staging_path)
            .map_err(|error| screenshot_error(&format!("failed to inspect screenshot: {error}")))?
            .len();
        if size_bytes == 0 || size_bytes > SCREENSHOT_MAX_BYTES {
            let _ = std::fs::remove_file(&staging_path);
            return Err(screenshot_error(
                "captured screenshot exceeds the supported size",
            ));
        }
        let store = OperationalArtifactStore::open(root, config.operational_artifact_index_path())?;
        let stored = store.store_existing_file(StoreArtifactRequest {
            source_path: staging_path.clone(),
            display_name: display_name.clone(),
            source_kind: SCREENSHOT_SOURCE_KIND.to_string(),
            media_type: Some(SCREENSHOT_MEDIA_TYPE.to_string()),
            enqueue_archive: false,
            session_id: Some(session_id.to_string()),
            attachment_id: None,
            workspace_id: None,
            worktree_path: None,
            metadata: BTreeMap::from([(
                "slice_id".to_string(),
                serde_json::Value::String(slice_id.to_string()),
            )]),
        });
        let _ = std::fs::remove_file(&staging_path);
        let stored = stored?;
        Ok(RoomEnvironmentScreenshotArtifact {
            artifact_id: stored.artifact_id,
            sha256: stored.sha256,
            size_bytes: stored.size_bytes,
            media_type: stored
                .media_type
                .unwrap_or_else(|| SCREENSHOT_MEDIA_TYPE.to_string()),
            display_name,
        })
    }

    pub(crate) fn execute_bound_room_screenshot_chunk(
        &self,
        authenticated_kernel_id: &str,
        authenticated_public_key: &str,
        session_id: &str,
        slice_id: &str,
        artifact_id: &str,
        offset: u64,
        max_bytes: u32,
    ) -> Result<RoomEnvironmentScreenshotChunk, DaemonError> {
        let config = self.authorize_bound_room_screenshot(
            authenticated_kernel_id,
            authenticated_public_key,
            session_id,
            slice_id,
        )?;
        if max_bytes == 0 || max_bytes > SCREENSHOT_CHUNK_MAX_BYTES {
            return Err(screenshot_error(
                "screenshot chunk size must be between 1 and 131072 bytes",
            ));
        }
        let store = OperationalArtifactStore::open(
            config.operational_artifact_root(),
            config.operational_artifact_index_path(),
        )?;
        let record = store
            .load_artifact(artifact_id)?
            .ok_or_else(|| screenshot_error("screenshot artifact was not found"))?;
        if record.source_kind != SCREENSHOT_SOURCE_KIND
            || record.media_type.as_deref() != Some(SCREENSHOT_MEDIA_TYPE)
            || record.session_id.as_deref() != Some(session_id)
            || record.size_bytes == 0
            || record.size_bytes > SCREENSHOT_MAX_BYTES
            || record
                .metadata
                .get("slice_id")
                .and_then(|value| value.as_str())
                != Some(slice_id)
        {
            return Err(screenshot_error(
                "screenshot artifact does not belong to this Room Environment",
            ));
        }
        let read = store.read_artifact_chunk(&record, offset, max_bytes as usize)?;
        Ok(RoomEnvironmentScreenshotChunk {
            artifact_id: record.artifact_id,
            offset,
            data_base64: base64::engine::general_purpose::STANDARD.encode(read.data),
            eof: read.eof,
        })
    }

    async fn authorize_room_screenshot(
        &self,
        caller: &KernelCaller,
        session_id: &str,
        attachment_id: &str,
    ) -> Result<SliceRecord, DaemonError> {
        if !matches!(
            caller.caller_kind,
            KernelCallerKind::LocalClient | KernelCallerKind::RemoteClient
        ) {
            return Err(screenshot_error(
                "caller cannot capture or read a Room screenshot",
            ));
        }
        self.ensure_attachment_in_session(session_id, attachment_id)
            .await?;
        let caller_user_id = caller
            .user_id
            .as_deref()
            .unwrap_or(crate::session::DEFAULT_LOCAL_USER_ID);
        if self.attachment_owner_user_id(attachment_id).await? != caller_user_id {
            return Err(DaemonError::SessionAccessDenied {
                session_id: session_id.to_string(),
                user_id: caller_user_id.to_string(),
            });
        }
        let binding = self
            .room_environment_slice(session_id)?
            .ok_or_else(|| screenshot_error("Room has no bound Environment slice"))?;
        let slice = self.resolve_slice(&binding.slice_id)?;
        if slice.status != SliceStatus::Running {
            return Err(screenshot_error("Room Environment slice is not running"));
        }
        Ok(slice)
    }

    fn authorize_bound_room_screenshot(
        &self,
        authenticated_kernel_id: &str,
        authenticated_public_key: &str,
        session_id: &str,
        slice_id: &str,
    ) -> Result<crate::config::DaemonConfig, DaemonError> {
        let config = self.owned.config_projection.snapshot();
        if !config
            .room_environment_worker_binding
            .as_ref()
            .is_some_and(|binding| {
                binding.permits(
                    authenticated_kernel_id,
                    authenticated_public_key,
                    session_id,
                    slice_id,
                )
            })
        {
            return Err(screenshot_error(
                "Room screenshot peer or binding scope was denied",
            ));
        }
        Ok(config)
    }

    async fn send_room_screenshot_peer_request(
        &self,
        slice: &SliceRecord,
        request: RelayPeerRequest,
    ) -> Result<RelayPeerResponse, DaemonError> {
        let config = self.owned.config_projection.snapshot();
        let config = config.slice_relay_override(slice).unwrap_or(config);
        let target = ClientTarget {
            daemon_id: slice.worker_kernel_id.clone(),
            daemon_alias: slice
                .worker_kernel_id
                .is_none()
                .then(|| slice.worker_kernel_ref.clone()),
        };
        crate::transport::relay_client::send_peer_request_via_temporary_connection_with_timeout(
            &config,
            target,
            request,
            Duration::from_secs(15),
        )
        .await
    }
}

fn screenshot_field(value: String, field: &str) -> Result<String, DaemonError> {
    if value.is_empty() || value.trim() != value {
        return Err(screenshot_error(&format!(
            "Room screenshot requires {field}"
        )));
    }
    Ok(value)
}

fn screenshot_artifact_is_valid(artifact: &RoomEnvironmentScreenshotArtifact) -> bool {
    !artifact.artifact_id.is_empty()
        && artifact.sha256.len() == 64
        && artifact
            .sha256
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        && artifact.size_bytes > 0
        && artifact.size_bytes <= SCREENSHOT_MAX_BYTES
        && artifact.media_type == SCREENSHOT_MEDIA_TYPE
        && !artifact.display_name.is_empty()
        && !matches!(artifact.display_name.as_str(), "." | "..")
        && !artifact.display_name.contains(['/', '\\'])
}

fn screenshot_error(message: &str) -> DaemonError {
    DaemonError::LocalTransport {
        operation: "environment.screenshot",
        message: message.to_string(),
    }
}
