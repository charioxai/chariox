use std::collections::BTreeMap;
use std::fmt::Write;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::error::DaemonError;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArtifactRecord {
    pub artifact_id: String,
    pub sha256: String,
    pub size_bytes: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub media_type: Option<String>,
    pub display_name: String,
    pub source_kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attachment_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub worktree_path: Option<String>,
    pub operational_path: PathBuf,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub metadata: BTreeMap<String, serde_json::Value>,
    pub created_at_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub archived_at_ms: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoreArtifactRequest {
    pub source_path: PathBuf,
    pub display_name: String,
    pub source_kind: String,
    pub session_id: Option<String>,
    pub attachment_id: Option<String>,
    pub workspace_id: Option<String>,
    pub worktree_path: Option<String>,
    pub metadata: BTreeMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArtifactArchiveOutboxItem {
    pub record: ArtifactRecord,
    pub attempts: u32,
    pub last_error: Option<String>,
}

#[derive(Debug, Clone)]
pub struct OperationalArtifactStore {
    root: PathBuf,
    connection: Arc<Mutex<Connection>>,
}

impl OperationalArtifactStore {
    pub fn open(root: PathBuf, index_path: PathBuf) -> Result<Self, DaemonError> {
        fs::create_dir_all(root.join("blobs"))
            .map_err(|error| artifact_error("create operational artifact blob directory", error))?;
        if let Some(parent) = index_path.parent() {
            fs::create_dir_all(parent).map_err(|error| {
                artifact_error("create operational artifact index directory", error)
            })?;
        }
        let connection =
            Connection::open(&index_path).map_err(|error| artifact_sql_error("open", error))?;
        connection
            .pragma_update(None, "journal_mode", "WAL")
            .map_err(|error| artifact_sql_error("enable WAL mode", error))?;
        connection
            .execute_batch(OPERATIONAL_ARTIFACT_SCHEMA)
            .map_err(|error| artifact_sql_error("migrate schema", error))?;
        Ok(Self {
            root,
            connection: Arc::new(Mutex::new(connection)),
        })
    }

    pub fn store_existing_file(
        &self,
        request: StoreArtifactRequest,
    ) -> Result<ArtifactRecord, DaemonError> {
        let source_path = fs::canonicalize(&request.source_path)
            .map_err(|error| artifact_error("canonicalize source artifact", error))?;
        let (sha256, size_bytes) = hash_file(&source_path)?;
        let created_at_ms = unix_epoch_ms();
        let artifact_id = format!("art_{created_at_ms}_{}", &sha256[..16]);
        let blob_path = self.blob_path(&sha256);
        if !blob_path.exists() {
            if let Some(parent) = blob_path.parent() {
                fs::create_dir_all(parent)
                    .map_err(|error| artifact_error("create artifact blob shard", error))?;
            }
            let tmp_path = blob_path.with_extension(format!("tmp-{}", std::process::id()));
            fs::copy(&source_path, &tmp_path)
                .map_err(|error| artifact_error("copy artifact blob", error))?;
            fs::rename(&tmp_path, &blob_path)
                .map_err(|error| artifact_error("promote artifact blob", error))?;
        }
        let record = ArtifactRecord {
            artifact_id,
            sha256,
            size_bytes,
            media_type: None,
            display_name: request.display_name,
            source_kind: request.source_kind,
            session_id: request.session_id,
            attachment_id: request.attachment_id,
            workspace_id: request.workspace_id,
            worktree_path: request.worktree_path,
            operational_path: blob_path,
            metadata: request.metadata,
            created_at_ms,
            archived_at_ms: None,
        };
        self.insert_record(&record)?;
        Ok(record)
    }

    pub fn load_pending_archive_artifacts(
        &self,
        limit: usize,
    ) -> Result<Vec<ArtifactArchiveOutboxItem>, DaemonError> {
        let connection = self.lock("lock artifact archive outbox")?;
        let mut statement = connection
            .prepare(
                "SELECT record_json, attempts, last_error
                 FROM artifact_archive_outbox
                 WHERE archived_at_ms IS NULL
                 ORDER BY created_at_ms ASC, artifact_id ASC
                 LIMIT ?1",
            )
            .map_err(|error| artifact_sql_error("prepare artifact archive outbox load", error))?;
        let mut rows = statement
            .query(params![limit.clamp(1, 500) as i64])
            .map_err(|error| artifact_sql_error("load artifact archive outbox", error))?;
        let mut items = Vec::new();
        while let Some(row) = rows
            .next()
            .map_err(|error| artifact_sql_error("read artifact archive outbox", error))?
        {
            let record_json = row
                .get::<_, String>(0)
                .map_err(|error| artifact_sql_error("read artifact archive record", error))?;
            let record = serde_json::from_str::<ArtifactRecord>(&record_json).map_err(|error| {
                DaemonError::SessionHistoryFailed {
                    session_id: None,
                    operation: "decode artifact archive record",
                    message: error.to_string(),
                }
            })?;
            items.push(ArtifactArchiveOutboxItem {
                record,
                attempts: row.get::<_, i64>(1).unwrap_or_default().max(0) as u32,
                last_error: row.get::<_, Option<String>>(2).unwrap_or_default(),
            });
        }
        Ok(items)
    }

    pub fn mark_archive_artifacts_accepted(
        &self,
        artifact_ids: &[String],
    ) -> Result<(), DaemonError> {
        if artifact_ids.is_empty() {
            return Ok(());
        }
        let now = unix_epoch_ms();
        let connection = self.lock("lock artifact archive outbox")?;
        for artifact_id in artifact_ids {
            connection
                .execute(
                    "UPDATE artifact_archive_outbox
                     SET archived_at_ms = ?2, updated_at_ms = ?2, last_error = NULL
                     WHERE artifact_id = ?1",
                    params![artifact_id.as_str(), now as i64],
                )
                .map_err(|error| artifact_sql_error("mark artifact archive accepted", error))?;
            connection
                .execute(
                    "UPDATE artifacts SET archived_at_ms = ?2 WHERE artifact_id = ?1",
                    params![artifact_id.as_str(), now as i64],
                )
                .map_err(|error| artifact_sql_error("mark artifact record archived", error))?;
        }
        Ok(())
    }

    pub fn mark_archive_artifacts_failed(
        &self,
        artifact_ids: &[String],
        message: &str,
    ) -> Result<(), DaemonError> {
        if artifact_ids.is_empty() {
            return Ok(());
        }
        let now = unix_epoch_ms();
        let connection = self.lock("lock artifact archive outbox")?;
        for artifact_id in artifact_ids {
            connection
                .execute(
                    "UPDATE artifact_archive_outbox
                     SET attempts = attempts + 1, last_error = ?2, updated_at_ms = ?3
                     WHERE artifact_id = ?1 AND archived_at_ms IS NULL",
                    params![artifact_id.as_str(), message, now as i64],
                )
                .map_err(|error| artifact_sql_error("mark artifact archive failed", error))?;
        }
        Ok(())
    }

    pub fn blob_path_for_record(&self, record: &ArtifactRecord) -> PathBuf {
        self.blob_path(&record.sha256)
    }

    fn blob_path(&self, sha256: &str) -> PathBuf {
        self.root
            .join("blobs")
            .join(&sha256[0..2])
            .join(&sha256[2..4])
            .join(sha256)
    }

    fn insert_record(&self, record: &ArtifactRecord) -> Result<(), DaemonError> {
        let record_json =
            serde_json::to_string(record).map_err(|error| DaemonError::SessionHistoryFailed {
                session_id: record.session_id.clone(),
                operation: "encode artifact record",
                message: error.to_string(),
            })?;
        let metadata_json = serde_json::to_string(&record.metadata).map_err(|error| {
            DaemonError::SessionHistoryFailed {
                session_id: record.session_id.clone(),
                operation: "encode artifact metadata",
                message: error.to_string(),
            }
        })?;
        let connection = self.lock("lock operational artifact store")?;
        connection
            .execute(
                "INSERT OR IGNORE INTO artifacts (
                    artifact_id, sha256, size_bytes, media_type, display_name, source_kind,
                    session_id, attachment_id, workspace_id, worktree_path, operational_path,
                    metadata_json, record_json, created_at_ms, archived_at_ms
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, NULL)",
                params![
                    record.artifact_id.as_str(),
                    record.sha256.as_str(),
                    record.size_bytes as i64,
                    record.media_type.as_deref(),
                    record.display_name.as_str(),
                    record.source_kind.as_str(),
                    record.session_id.as_deref(),
                    record.attachment_id.as_deref(),
                    record.workspace_id.as_deref(),
                    record.worktree_path.as_deref(),
                    record.operational_path.display().to_string(),
                    metadata_json,
                    record_json,
                    record.created_at_ms as i64,
                ],
            )
            .map_err(|error| artifact_sql_error("insert artifact record", error))?;
        connection
            .execute(
                "INSERT OR IGNORE INTO artifact_archive_outbox (
                    artifact_id, record_json, attempts, last_error, archived_at_ms,
                    created_at_ms, updated_at_ms
                 ) VALUES (?1, ?2, 0, NULL, NULL, ?3, ?3)",
                params![
                    record.artifact_id.as_str(),
                    record_json,
                    record.created_at_ms as i64,
                ],
            )
            .map_err(|error| artifact_sql_error("enqueue artifact archive record", error))?;
        Ok(())
    }

    fn lock(
        &self,
        operation: &'static str,
    ) -> Result<std::sync::MutexGuard<'_, Connection>, DaemonError> {
        self.connection
            .lock()
            .map_err(|error| DaemonError::SessionHistoryFailed {
                session_id: None,
                operation,
                message: error.to_string(),
            })
    }
}

fn hash_file(path: &Path) -> Result<(String, u64), DaemonError> {
    let mut file =
        fs::File::open(path).map_err(|error| artifact_error("open artifact source", error))?;
    let mut hasher = Sha256::new();
    let mut size = 0u64;
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|error| artifact_error("read artifact source", error))?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
        size += read as u64;
    }
    Ok((hex_lower(&hasher.finalize()), size))
}

fn hex_lower(bytes: &[u8]) -> String {
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        write!(&mut output, "{byte:02x}").expect("writing hex to string should not fail");
    }
    output
}

fn unix_epoch_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

fn artifact_error(operation: &'static str, error: std::io::Error) -> DaemonError {
    DaemonError::SessionHistoryFailed {
        session_id: None,
        operation,
        message: error.to_string(),
    }
}

fn artifact_sql_error(operation: &'static str, error: rusqlite::Error) -> DaemonError {
    DaemonError::SessionHistoryFailed {
        session_id: None,
        operation,
        message: error.to_string(),
    }
}

const OPERATIONAL_ARTIFACT_SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS artifacts (
    artifact_id TEXT PRIMARY KEY,
    sha256 TEXT NOT NULL,
    size_bytes INTEGER NOT NULL,
    media_type TEXT,
    display_name TEXT NOT NULL,
    source_kind TEXT NOT NULL,
    session_id TEXT,
    attachment_id TEXT,
    workspace_id TEXT,
    worktree_path TEXT,
    operational_path TEXT NOT NULL,
    metadata_json TEXT NOT NULL,
    record_json TEXT NOT NULL,
    created_at_ms INTEGER NOT NULL,
    archived_at_ms INTEGER
);

CREATE INDEX IF NOT EXISTS idx_artifacts_sha256 ON artifacts(sha256);
CREATE INDEX IF NOT EXISTS idx_artifacts_session ON artifacts(session_id, created_at_ms);
CREATE INDEX IF NOT EXISTS idx_artifacts_source_kind ON artifacts(source_kind, created_at_ms);

CREATE TABLE IF NOT EXISTS artifact_archive_outbox (
    artifact_id TEXT PRIMARY KEY,
    record_json TEXT NOT NULL,
    attempts INTEGER NOT NULL DEFAULT 0,
    last_error TEXT,
    archived_at_ms INTEGER,
    created_at_ms INTEGER NOT NULL,
    updated_at_ms INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_artifact_archive_outbox_pending
    ON artifact_archive_outbox(archived_at_ms, created_at_ms);
"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stores_artifact_blob_and_loads_archive_outbox() {
        let root = std::env::temp_dir().join(format!(
            "chariox-artifact-store-{}-{}",
            std::process::id(),
            unix_epoch_ms()
        ));
        let source = root.join("source.txt");
        fs::create_dir_all(&root).expect("root should exist");
        fs::write(&source, "hello artifact").expect("source should write");
        let store = OperationalArtifactStore::open(root.join("store"), root.join("index.db"))
            .expect("store should open");

        let record = store
            .store_existing_file(StoreArtifactRequest {
                source_path: source,
                display_name: "source.txt".to_string(),
                source_kind: "transfer".to_string(),
                session_id: Some("session-1".to_string()),
                attachment_id: Some("attachment-1".to_string()),
                workspace_id: Some("workspace-1".to_string()),
                worktree_path: Some("/tmp/worktree".to_string()),
                metadata: BTreeMap::new(),
            })
            .expect("artifact should store");

        assert!(record.operational_path.exists());
        assert_eq!(record.size_bytes, 14);
        let pending = store
            .load_pending_archive_artifacts(10)
            .expect("pending should load");
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].record.artifact_id, record.artifact_id);
        store
            .mark_archive_artifacts_accepted(&[record.artifact_id])
            .expect("accept should mark");
        assert!(store
            .load_pending_archive_artifacts(10)
            .expect("pending should reload")
            .is_empty());
        let _ = fs::remove_dir_all(&root);
    }
}
