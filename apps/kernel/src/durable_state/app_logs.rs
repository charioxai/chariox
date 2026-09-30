//! Bounded per-installation App log (SDK `log.write`), read by `app logs`.
//! Entries are App-authored text: stored and returned as data, never
//! interpreted, and never written to the kernel's own log. The kernel adds
//! its own notices about the App here too, marked by the `kernel` field an
//! App cannot write.
use super::{DurableKernelStateStore, DurableWriterRequest};
use rusqlite::{params, Connection, OptionalExtension};
use std::sync::mpsc;

/// Entries kept per installation; older ones are dropped on write.
const KEEP: i64 = 1000;
const MAX_MESSAGE_BYTES: usize = 4096;
const MAX_FIELDS_BYTES: usize = 8192;
pub(crate) const MAX_READ: usize = 200;
/// Marks an entry the kernel wrote; refused in App-written fields.
const KERNEL_FIELD: &str = "kernel";

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct AppLogEntry {
    pub(crate) sequence: u64,
    pub(crate) at_ms: u64,
    pub(crate) level: String,
    pub(crate) message: String,
    pub(crate) fields: serde_json::Value,
}

pub(super) struct AppLogRequest {
    owner: String,
    installation: String,
    level: String,
    message: String,
    fields: String,
    /// Writes refused since the last admitted one; noted before this entry.
    dropped: u32,
    response: mpsc::Sender<Result<(), &'static str>>,
}
impl std::fmt::Debug for AppLogRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("AppLogRequest(..)")
    }
}

pub(super) fn initialize(connection: &Connection) -> rusqlite::Result<()> {
    connection.execute_batch(
        "CREATE TABLE IF NOT EXISTS app_logs (
            sequence INTEGER PRIMARY KEY AUTOINCREMENT,
            owner_id TEXT NOT NULL, installation_id TEXT NOT NULL,
            at_ms INTEGER NOT NULL CHECK(at_ms >= 0),
            level TEXT NOT NULL CHECK(level IN ('debug','info','warn','error')),
            message TEXT NOT NULL, fields_json TEXT NOT NULL);
         CREATE INDEX IF NOT EXISTS app_logs_installation
            ON app_logs(owner_id, installation_id, sequence);",
    )
}

/// Validates an App's `log.write` request; the error is a stable code.
pub(crate) fn validate(
    level: &str,
    message: &str,
    fields: &serde_json::Value,
) -> Result<String, &'static str> {
    if !matches!(level, "debug" | "info" | "warn" | "error") {
        return Err("INVALID_ARGUMENT");
    }
    if !fields.is_object() || fields.get(KERNEL_FIELD).is_some() {
        return Err("INVALID_ARGUMENT");
    }
    if message.len() > MAX_MESSAGE_BYTES {
        return Err("LIMIT_EXCEEDED");
    }
    let fields = serde_json::to_string(fields).map_err(|_| "INVALID_ARGUMENT")?;
    if fields.len() > MAX_FIELDS_BYTES {
        return Err("LIMIT_EXCEEDED");
    }
    Ok(fields)
}

impl DurableKernelStateStore {
    pub(crate) fn append_app_log(
        &self,
        owner: &str,
        installation: &str,
        level: &str,
        message: &str,
        fields: &serde_json::Value,
    ) -> Result<(), &'static str> {
        self.append_app_log_after_drops(owner, installation, level, message, fields, 0)
    }

    /// Like `append_app_log`, first noting how many earlier writes were
    /// refused (rate limit or busy), so the owner sees the gap.
    pub(crate) fn append_app_log_after_drops(
        &self,
        owner: &str,
        installation: &str,
        level: &str,
        message: &str,
        fields: &serde_json::Value,
        dropped: u32,
    ) -> Result<(), &'static str> {
        let fields = validate(level, message, fields)?;
        let (response, receiver) = mpsc::channel();
        self.writer
            .enqueue(DurableWriterRequest::AppLog(Box::new(AppLogRequest {
                owner: owner.into(),
                installation: installation.into(),
                level: level.into(),
                message: message.into(),
                fields,
                dropped,
                response,
            })))
            .map_err(|_| "STORAGE_UNAVAILABLE")?;
        receiver.recv().map_err(|_| "STORAGE_UNAVAILABLE")?
    }

    /// Entries after `after` (oldest first), at most `limit`.
    pub(crate) fn app_logs(
        &self,
        owner: &str,
        installation: &str,
        after: u64,
        limit: usize,
    ) -> Result<Vec<AppLogEntry>, &'static str> {
        let connection = self
            .lock_connection("durable_state.app_logs")
            .map_err(|_| "STORAGE_UNAVAILABLE")?;
        let mut statement = connection
            .prepare(
                "SELECT sequence, at_ms, level, message, fields_json FROM app_logs
                 WHERE owner_id=?1 AND installation_id=?2 AND sequence>?3
                 ORDER BY sequence LIMIT ?4",
            )
            .map_err(|_| "STORAGE_UNAVAILABLE")?;
        let rows = statement
            .query_map(
                params![
                    owner,
                    installation,
                    i64::try_from(after).unwrap_or(i64::MAX),
                    limit.min(MAX_READ) as i64
                ],
                |row| {
                    let fields: String = row.get(4)?;
                    Ok(AppLogEntry {
                        sequence: row.get::<_, i64>(0)? as u64,
                        at_ms: row.get::<_, i64>(1)? as u64,
                        level: row.get(2)?,
                        message: row.get(3)?,
                        fields: serde_json::from_str(&fields).unwrap_or(serde_json::Value::Null),
                    })
                },
            )
            .map_err(|_| "STORAGE_UNAVAILABLE")?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|_| "STORAGE_UNAVAILABLE")
    }
}

pub(super) fn execute(connection: &mut Connection, request: AppLogRequest) {
    let result = (|| -> rusqlite::Result<()> {
        let transaction = connection.transaction()?;
        let now = crate::session::unix_epoch_ms();
        if request.dropped > 0 {
            let mut fields = serde_json::Map::new();
            fields.insert("dropped".into(), request.dropped.into());
            append_kernel_notice_in(
                &transaction,
                &request.owner,
                &request.installation,
                now,
                &match request.dropped {
                    1 => "1 log write was dropped: the App wrote faster than its log rate limit or while busy.".into(),
                    count => format!("{count} log writes were dropped: the App wrote faster than its log rate limit or while busy."),
                },
                fields,
            )?;
        }
        insert(
            &transaction,
            &request.owner,
            &request.installation,
            now,
            &request.level,
            &request.message,
            &request.fields,
        )?;
        transaction.commit()
    })()
    .map_err(|_| "STORAGE_UNAVAILABLE");
    let _ = request.response.send(result);
}

/// A kernel notice about the installation, in the caller's transaction.
pub(super) fn append_kernel_notice_in(
    transaction: &Connection,
    owner: &str,
    installation: &str,
    at_ms: u64,
    message: &str,
    mut fields: serde_json::Map<String, serde_json::Value>,
) -> rusqlite::Result<()> {
    fields.insert(KERNEL_FIELD.into(), true.into());
    let fields = serde_json::Value::Object(fields).to_string();
    insert(
        transaction,
        owner,
        installation,
        at_ms,
        "warn",
        message,
        &fields,
    )
}

/// When the installation's latest kernel notice carrying `marker` was
/// written, if one is still kept.
pub(super) fn latest_kernel_notice_at_in(
    transaction: &Connection,
    owner: &str,
    installation: &str,
    marker: &str,
) -> rusqlite::Result<Option<u64>> {
    transaction
        .query_row(
            "SELECT at_ms FROM app_logs WHERE owner_id=?1 AND installation_id=?2
               AND json_extract(fields_json, '$.kernel') = 1
               AND json_extract(fields_json, '$.' || ?3) = 1
             ORDER BY sequence DESC LIMIT 1",
            params![owner, installation, marker],
            |row| row.get::<_, i64>(0),
        )
        .optional()
        .map(|at| at.map(|at| at.max(0) as u64))
}

/// Appends one entry and drops the installation's oldest beyond `KEEP`.
fn insert(
    transaction: &Connection,
    owner: &str,
    installation: &str,
    at_ms: u64,
    level: &str,
    message: &str,
    fields: &str,
) -> rusqlite::Result<()> {
    transaction.execute(
        "INSERT INTO app_logs(owner_id, installation_id, at_ms, level, message, fields_json)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![owner, installation, at_ms as i64, level, message, fields],
    )?;
    transaction.execute(
        "DELETE FROM app_logs WHERE owner_id=?1 AND installation_id=?2 AND sequence <= (
           SELECT sequence FROM app_logs WHERE owner_id=?1 AND installation_id=?2
           ORDER BY sequence DESC LIMIT 1 OFFSET ?3)",
        params![owner, installation, KEEP],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn logs_are_validated_owner_scoped_ordered_and_bounded() {
        let root =
            std::env::temp_dir().join(format!("chariox-app-logs-{:016x}", rand::random::<u64>()));
        std::fs::create_dir(&root).unwrap();
        let store = DurableKernelStateStore::open_owned(root.join("kernel.sqlite")).unwrap();
        let fields = serde_json::json!({"todo": "1"});
        assert_eq!(
            store.append_app_log("alice", "todo", "trace", "x", &fields),
            Err("INVALID_ARGUMENT")
        );
        assert_eq!(
            store.append_app_log("alice", "todo", "info", "x", &serde_json::json!([1])),
            Err("INVALID_ARGUMENT")
        );
        // Only the kernel marks its own notices.
        assert_eq!(
            store.append_app_log(
                "alice",
                "todo",
                "warn",
                "x",
                &serde_json::json!({"kernel": true})
            ),
            Err("INVALID_ARGUMENT")
        );
        assert_eq!(
            store.append_app_log(
                "alice",
                "todo",
                "info",
                "x",
                &serde_json::json!({"big": "x".repeat(9000)})
            ),
            Err("LIMIT_EXCEEDED")
        );
        // UTF-8 bytes, as the SDK counts: 1100 emoji are 4400 bytes.
        assert_eq!(
            store.append_app_log("alice", "todo", "info", &"\u{1F600}".repeat(1100), &fields),
            Err("LIMIT_EXCEEDED")
        );
        for index in 0..1005 {
            store
                .append_app_log("alice", "todo", "info", &format!("m{index}"), &fields)
                .unwrap();
        }
        store
            .append_app_log("bob", "todo", "warn", "bob's", &fields)
            .unwrap();
        let first = store.app_logs("alice", "todo", 0, 3).unwrap();
        assert_eq!(
            first.iter().map(|e| e.message.as_str()).collect::<Vec<_>>(),
            ["m5", "m6", "m7"]
        );
        assert_eq!(first[0].fields, fields);
        let rest = store
            .app_logs("alice", "todo", first[2].sequence, 10_000)
            .unwrap();
        assert_eq!(rest.len(), MAX_READ);
        assert_eq!(store.app_logs("bob", "todo", 0, 10).unwrap().len(), 1);
        drop(store);
        let _ = std::fs::remove_dir_all(root);
    }
}
