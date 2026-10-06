//! MP-08/MP-10/MP-11: one Room-owned artifact service for tools and attachments.
use super::KernelRuntimeState;
use crate::artifacts::{ArtifactRecord, OperationalArtifactStore, StoreArtifactRequest};
use crate::error::DaemonError;
use crate::runtime::browser_artifact::*;
use crate::transport::room_browser_controller::{
    RoomBrowserControllerCommand as Command, RoomBrowserControllerResult as Response,
};
use crate::transport::runtime_tools::{RuntimeToolResult, SliceBrowserArtifactArgs};
use base64::Engine as _;
use std::collections::BTreeMap;
use std::io::Write;

const SOURCE: &str = "room_browser_artifact";
fn error(message: impl Into<String>) -> DaemonError {
    DaemonError::LocalTransport {
        operation: "room_browser_artifact",
        message: message.into(),
    }
}

impl KernelRuntimeState {
    pub(crate) async fn room_browser_artifact_for_client(
        &self,
        caller: &crate::runtime::command::KernelCaller,
        request: crate::local::RoomBrowserArtifactRequest,
    ) -> Result<RuntimeToolResult, DaemonError> {
        self.authorize_room_observation_attachment(
            caller,
            &request.session_id,
            &request.attachment_id,
        )
        .await?;
        self.execute_room_browser_artifact(&request.session_id, &request.tab_id, request.operation)
            .await
    }

    pub(in crate::runtime) async fn execute_room_browser_artifact(
        &self,
        session_id: &str,
        tab_id: &str,
        args: SliceBrowserArtifactArgs,
    ) -> Result<RuntimeToolResult, DaemonError> {
        self.owned
            .room_secret_observations
            .require(session_id, true)?;
        self.ensure_browser_controller_process_started(session_id)
            .await?;
        self.reconcile_browser_controller_environment(session_id)
            .await?;
        let environment = self
            .room_environment_snapshot(session_id)
            .map_err(|e| error(e.code()))?;
        let binding = self
            .room_environment_controller_tab_binding(session_id, tab_id)
            .map_err(|e| error(e.code()))?;
        let config = self.owned.config_projection.snapshot();
        let store = OperationalArtifactStore::open(
            config.operational_artifact_root(),
            config.operational_artifact_index_path(),
        )?;
        let revision = self.owned.room_secret_observations.revision(session_id)?;
        match args {
            SliceBrowserArtifactArgs::Capture {
                kind,
                browser_generation,
                guid,
                return_image_base64,
            } => {
                if kind == BrowserArtifactKind::Identity
                    || (return_image_base64 && kind != BrowserArtifactKind::Image)
                    || browser_generation == 0
                    || (kind == BrowserArtifactKind::Download) != guid.is_some()
                {
                    return Err(error("capture requires the observed browser generation and a GUID only for downloads"));
                }
                let request = BrowserArtifactRequest {
                    target_id: binding.runtime_target_id.clone(),
                    document_id: binding.document_id.clone(),
                    viewport: environment.viewport.clone(),
                    kind,
                    guid,
                    browser_generation,
                };
                let Response::Artifact {
                    capture: Some(capture),
                } = self
                    .room_browser_controller_command(
                        session_id,
                        Command::Artifact {
                            request: request.clone(),
                        },
                    )
                    .await?
                else {
                    return Err(error("Browser controller did not return an artifact"));
                };
                let _guard = self
                    .owned
                    .room_secret_observations
                    .barrier(session_id)?
                    .read_owned()
                    .await;
                if revision != self.owned.room_secret_observations.revision(session_id)? {
                    return Err(error("observation changed during Browser capture"));
                }
                let bytes = capture.validate(&request).map_err(error)?;
                let current = self
                    .room_environment_snapshot(session_id)
                    .map_err(|e| error(e.code()))?;
                let current_binding = self
                    .room_environment_controller_tab_binding(session_id, tab_id)
                    .map_err(|e| error(e.code()))?;
                if current.runtime_generation != environment.runtime_generation
                    || current.viewport != environment.viewport
                    || current_binding.document_id != binding.document_id
                    || current_binding.document_revision != binding.document_revision
                {
                    return Err(error("Browser artifact became stale before publication"));
                }
                let root = config.operational_artifact_root().join("browser-staging");
                std::fs::create_dir_all(&root)
                    .map_err(|_| error("artifact staging unavailable"))?;
                let path = root.join(format!("{:032x}", rand::random::<u128>()));
                let mut options = std::fs::OpenOptions::new();
                options.write(true).create_new(true);
                #[cfg(unix)]
                {
                    use std::os::unix::fs::OpenOptionsExt;
                    options.mode(0o600);
                }
                let stored = (|| {
                    let mut file = options
                        .open(&path)
                        .map_err(|_| error("artifact staging unavailable"))?;
                    file.write_all(&bytes)
                        .map_err(|_| error("artifact staging write failed"))?;
                    store.store_existing_file(StoreArtifactRequest {
                        source_path: path.clone(),
                        display_name: capture.bytes.display_name.clone(),
                        media_type: Some(capture.bytes.mime_type.clone()),
                        source_kind: SOURCE.into(),
                        enqueue_archive: false,
                        session_id: Some(session_id.into()),
                        attachment_id: None,
                        workspace_id: None,
                        worktree_path: None,
                        metadata: BTreeMap::from_iter([
                            (
                                "environment_id".into(),
                                environment.environment_id.clone().into(),
                            ),
                            (
                                "runtime_generation".into(),
                                environment.runtime_generation.into(),
                            ),
                            ("tab_id".into(), tab_id.into()),
                            ("document_revision".into(), binding.document_revision.into()),
                            ("document_id".into(), binding.document_id.clone().into()),
                            ("browser_id".into(), capture.browser_id.clone().into()),
                            (
                                "browser_generation".into(),
                                capture.browser_generation.into(),
                            ),
                            (
                                "viewport".into(),
                                serde_json::to_value(&environment.viewport)
                                    .map_err(|_| error("viewport encoding failed"))?,
                            ),
                            ("geometry".into(), capture.geometry.clone()),
                            ("kind".into(), serde_json::to_value(kind).unwrap()),
                            ("redaction".into(), capture.redaction.clone().into()),
                            (
                                "observation_epoch".into(),
                                self.owned.room_secret_observations.epoch.into(),
                            ),
                            ("observation_revision".into(), revision.into()),
                        ]),
                    })
                })();
                let _ = std::fs::remove_file(&path);
                let record = stored?;
                let mut payload = public_artifact(&record);
                if return_image_base64 {
                    if kind != BrowserArtifactKind::Image {
                        return Err(error("native image bytes require an image capture"));
                    }
                    payload["image_base64"] = capture.bytes.data_base64.into();
                }
                Ok(RuntimeToolResult { ok: true, payload })
            }
            SliceBrowserArtifactArgs::Read {
                artifact_id,
                offset,
                max_bytes,
            } => {
                self.validate_browser_artifact_identity(session_id, tab_id, &store, &artifact_id)
                    .await?;
                let _guard = self
                    .owned
                    .room_secret_observations
                    .barrier(session_id)?
                    .read_owned()
                    .await;
                if max_bytes == 0 || max_bytes > 128 * 1024 {
                    return Err(error("Browser artifact read is bounded to 131072 bytes"));
                }
                let record =
                    self.load_browser_artifact(&store, session_id, tab_id, &artifact_id)?;
                let bytes = verified_bytes(&store, &record)?;
                if offset > bytes.len() as u64 {
                    return Err(error("invalid artifact offset"));
                }
                let end = bytes
                    .len()
                    .min((offset as usize).saturating_add(max_bytes as usize));
                let chunk = crate::artifacts::ArtifactChunkRead {
                    data: bytes[offset as usize..end].to_vec(),
                    eof: end == bytes.len(),
                };
                Ok(RuntimeToolResult {
                    ok: true,
                    payload: serde_json::json!({ "artifact": public_artifact(&record),
                    "offset": offset, "data_base64": base64::engine::general_purpose::STANDARD.encode(chunk.data), "eof": chunk.eof }),
                })
            }
            SliceBrowserArtifactArgs::Inspect { artifact_id } => {
                self.validate_browser_artifact_identity(session_id, tab_id, &store, &artifact_id)
                    .await?;
                let _guard = self
                    .owned
                    .room_secret_observations
                    .barrier(session_id)?
                    .read_owned()
                    .await;
                let record =
                    self.load_browser_artifact(&store, session_id, tab_id, &artifact_id)?;
                if record.size_bytes > 256 * 1024 {
                    return Err(error(
                        "text inspection is bounded to 262144 bytes; use chunk reads",
                    ));
                }
                let chunk = crate::artifacts::ArtifactChunkRead {
                    data: verified_bytes(&store, &record)?,
                    eof: true,
                };
                let text = if record.media_type.as_deref() == Some("application/pdf") {
                    crate::artifacts::inspect::pdf_text(&chunk.data).map_err(error)?
                } else {
                    String::from_utf8(chunk.data)
                        .map_err(|_| error("artifact is not UTF-8 text; use bounded chunk reads"))?
                };
                let text = self.owned.room_secret_observations.scrub(
                    session_id,
                    RuntimeToolResult {
                        ok: true,
                        payload: serde_json::json!({ "text": text }),
                    },
                )?;
                Ok(RuntimeToolResult {
                    ok: true,
                    payload: serde_json::json!({ "artifact": public_artifact(&record), "text": text.payload["text"] }),
                })
            }
        }
    }

    async fn validate_browser_artifact_identity(
        &self,
        session_id: &str,
        tab_id: &str,
        store: &OperationalArtifactStore,
        artifact_id: &str,
    ) -> Result<(), DaemonError> {
        let record = self.load_browser_artifact(store, session_id, tab_id, artifact_id)?;
        let binding = self
            .room_environment_controller_tab_binding(session_id, tab_id)
            .map_err(|e| error(e.code()))?;
        let environment = self
            .room_environment_snapshot(session_id)
            .map_err(|e| error(e.code()))?;
        let request = BrowserArtifactRequest {
            target_id: binding.runtime_target_id,
            document_id: binding.document_id,
            browser_generation: record
                .metadata
                .get("browser_generation")
                .and_then(serde_json::Value::as_u64)
                .ok_or_else(|| error("artifact browser identity is absent"))?,
            viewport: environment.viewport,
            kind: BrowserArtifactKind::Identity,
            guid: None,
        };
        let Response::Artifact {
            capture: Some(capture),
        } = self
            .room_browser_controller_command(
                session_id,
                Command::Artifact {
                    request: request.clone(),
                },
            )
            .await?
        else {
            return Err(error("Browser identity is unavailable"));
        };
        capture.validate(&request).map_err(error)?;
        if record.metadata.get("browser_id") != Some(&capture.browser_id.into())
            || (record
                .metadata
                .get("kind")
                .and_then(serde_json::Value::as_str)
                == Some("image")
                && record.metadata.get("geometry") != Some(&capture.geometry))
        {
            return Err(error("Browser artifact browser/viewport identity is stale"));
        }
        Ok(())
    }

    fn load_browser_artifact(
        &self,
        store: &OperationalArtifactStore,
        session_id: &str,
        tab_id: &str,
        artifact_id: &str,
    ) -> Result<ArtifactRecord, DaemonError> {
        let record = store
            .load_artifact(artifact_id)?
            .ok_or_else(|| error("Browser artifact is unavailable"))?;
        let environment = self
            .room_environment_snapshot(session_id)
            .map_err(|e| error(e.code()))?;
        let binding = self
            .room_environment_controller_tab_binding(session_id, tab_id)
            .map_err(|e| error(e.code()))?;
        if record.source_kind != SOURCE
            || record.session_id.as_deref() != Some(session_id)
            || record
                .metadata
                .get("environment_id")
                .and_then(serde_json::Value::as_str)
                != Some(environment.environment_id.as_str())
            || record
                .metadata
                .get("document_revision")
                .and_then(serde_json::Value::as_u64)
                != Some(binding.document_revision)
            || record
                .metadata
                .get("tab_id")
                .and_then(serde_json::Value::as_str)
                != Some(tab_id)
            || record
                .metadata
                .get("runtime_generation")
                .and_then(serde_json::Value::as_u64)
                != Some(environment.runtime_generation)
            || record
                .metadata
                .get("document_id")
                .and_then(serde_json::Value::as_str)
                != Some(binding.document_id.as_str())
            || record.metadata.get("viewport")
                != Some(&serde_json::to_value(environment.viewport).unwrap())
            || record
                .metadata
                .get("observation_epoch")
                .and_then(serde_json::Value::as_u64)
                != Some(self.owned.room_secret_observations.epoch)
            || record
                .metadata
                .get("observation_revision")
                .and_then(serde_json::Value::as_u64)
                != Some(self.owned.room_secret_observations.revision(session_id)?)
        {
            return Err(error(
                "Browser artifact is stale or belongs to another Room/tab",
            ));
        }
        Ok(record)
    }

    pub(in crate::runtime::state) fn browser_upload_artifacts(
        &self,
        session_id: &str,
        ids: &[String],
    ) -> Result<crate::runtime::browser_controller_file_transfer::BrowserUploadFiles, DaemonError>
    {
        if ids.is_empty() || ids.len() > 20 {
            return Err(error("upload requires 1 through 20 opaque artifacts"));
        }
        let config = self.owned.config_projection.snapshot();
        let store = OperationalArtifactStore::open(
            config.operational_artifact_root(),
            config.operational_artifact_index_path(),
        )?;
        let mut artifacts = Vec::new();
        let mut total = 0;
        for id in ids {
            let record = store
                .load_artifact(id)?
                .ok_or_else(|| error("upload artifact is unavailable"))?;
            if record.session_id.as_deref() != Some(session_id)
                || !matches!(record.source_kind.as_str(), SOURCE | "transfer")
                || !safe_filename(&record.display_name)
            {
                return Err(error(
                    "upload artifact belongs to another Room or has an unsafe filename",
                ));
            }
            if record.source_kind == SOURCE {
                let tab_id = record
                    .metadata
                    .get("tab_id")
                    .and_then(serde_json::Value::as_str)
                    .ok_or_else(|| error("artifact tab binding is absent"))?;
                self.load_browser_artifact(&store, session_id, tab_id, id)?;
            }
            total += record.size_bytes;
            if total > MAX_BROWSER_ARTIFACT_BYTES as u64 {
                return Err(error("upload artifact byte bound exceeded"));
            }
            let chunk = store.read_artifact_chunk(&record, 0, MAX_BROWSER_ARTIFACT_BYTES)?;
            let artifact = BrowserArtifactBytes {
                display_name: record.display_name,
                mime_type: record
                    .media_type
                    .unwrap_or_else(|| "application/octet-stream".into()),
                size_bytes: record.size_bytes,
                sha256: record.sha256,
                data_base64: base64::engine::general_purpose::STANDARD.encode(chunk.data),
            };
            artifact.decode().map_err(error)?;
            artifacts.push(artifact);
        }
        crate::runtime::browser_controller_file_transfer::BrowserUploadFiles::from_artifacts(
            artifacts,
        )
        .map_err(error)
    }
}
fn public_artifact(record: &ArtifactRecord) -> serde_json::Value {
    serde_json::json!({ "source": "browser_controller", "session_id": record.session_id, "artifact_id": record.artifact_id,
        "sha256": record.sha256, "size_bytes": record.size_bytes, "mime_type": record.media_type,
        "display_name": record.display_name, "identity": record.metadata })
}

fn verified_bytes(
    store: &OperationalArtifactStore,
    record: &ArtifactRecord,
) -> Result<Vec<u8>, DaemonError> {
    if record.size_bytes > MAX_BROWSER_ARTIFACT_BYTES as u64 {
        return Err(error("artifact exceeds Browser byte bound"));
    }
    let bytes = store
        .read_artifact_chunk(record, 0, MAX_BROWSER_ARTIFACT_BYTES)?
        .data;
    use sha2::{Digest, Sha256};
    if bytes.len() as u64 != record.size_bytes
        || format!("{:x}", Sha256::digest(&bytes)) != record.sha256
    {
        return Err(error("Browser artifact bytes changed"));
    }
    Ok(bytes)
}
