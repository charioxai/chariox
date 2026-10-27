use std::fs::{self, File};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{json, Value};

use crate::error::DaemonError;
use crate::local::{ExportDebugBundleRequest, LocalDaemonResponse};

const DEFAULT_DEBUG_BUNDLE_LIMIT: usize = 1000;
const MAX_DEBUG_BUNDLE_LIMIT: usize = 10_000;

pub(crate) fn execute_export_debug_bundle_request(
    request: ExportDebugBundleRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    let log_root = crate::logging::default_log_root();
    let export = export_debug_bundle_from_log_root(&log_root, request)?;
    Ok(LocalDaemonResponse::DebugBundleExported {
        bundle_dir: export.bundle_dir.display().to_string(),
        manifest_path: export.manifest_path.display().to_string(),
        logs_path: export.logs_path.display().to_string(),
        log_root: log_root.display().to_string(),
        record_count: export.record_count,
        limit: export.limit,
    })
}

struct DebugBundleExport {
    bundle_dir: PathBuf,
    manifest_path: PathBuf,
    logs_path: PathBuf,
    record_count: usize,
    limit: usize,
}

fn export_debug_bundle_from_log_root(
    log_root: &Path,
    request: ExportDebugBundleRequest,
) -> Result<DebugBundleExport, DaemonError> {
    let limit = request
        .limit
        .unwrap_or(DEFAULT_DEBUG_BUNDLE_LIMIT)
        .clamp(1, MAX_DEBUG_BUNDLE_LIMIT);
    let records = matching_log_records(log_root, &request.session_id, limit)?;
    let bundle_dir = default_debug_bundle_dir(
        log_root,
        &request.session_id,
        request.bundle_label.as_deref(),
    );
    fs::create_dir_all(&bundle_dir).map_err(|error| DaemonError::LocalTransport {
        operation: "debug bundle export",
        message: format!(
            "failed to create debug bundle directory `{}`: {error}",
            bundle_dir.display()
        ),
    })?;
    let manifest_path = bundle_dir.join("manifest.json");
    let logs_path = bundle_dir.join("logs.ndjson");

    let manifest = json!({
        "schema": "chariox.debug_bundle.v1",
        "created_at_ms": unix_epoch_ms(),
        "log_root": log_root.display().to_string(),
        "filters": {
            "session_id": request.session_id,
            "limit": limit,
        },
        "record_count": records.len(),
        "files": ["logs.ndjson"],
    });
    write_text_file(
        &manifest_path,
        &format!(
            "{}\n",
            serde_json::to_string_pretty(&manifest).map_err(|error| {
                DaemonError::LocalTransport {
                    operation: "debug bundle export",
                    message: format!("failed to serialize debug bundle manifest: {error}"),
                }
            })?
        ),
    )?;
    let log_body = if records.is_empty() {
        String::new()
    } else {
        format!(
            "{}\n",
            records
                .iter()
                .map(|record| record.line.as_str())
                .collect::<Vec<_>>()
                .join("\n")
        )
    };
    write_text_file(&logs_path, &log_body)?;

    crate::logging::info_with_fields(
        "daemon.debug_bundle",
        "exported debug bundle",
        json!({
            "session_id": manifest["filters"]["session_id"].as_str(),
            "bundle_dir": bundle_dir.display().to_string(),
            "record_count": records.len(),
            "limit": limit,
        }),
    );

    Ok(DebugBundleExport {
        bundle_dir,
        manifest_path,
        logs_path,
        record_count: records.len(),
        limit,
    })
}

#[derive(Debug)]
struct LogLine {
    timestamp_ms: u64,
    sequence: usize,
    line: String,
}

fn matching_log_records(
    log_root: &Path,
    session_id: &str,
    limit: usize,
) -> Result<Vec<LogLine>, DaemonError> {
    let mut records = Vec::new();
    let entries = match fs::read_dir(log_root) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(records),
        Err(error) => {
            return Err(DaemonError::LocalTransport {
                operation: "debug bundle export",
                message: format!("failed to read log root `{}`: {error}", log_root.display()),
            });
        }
    };

    let mut sequence = 0usize;
    for entry in entries {
        let entry = entry.map_err(|error| DaemonError::LocalTransport {
            operation: "debug bundle export",
            message: format!("failed to read log root entry: {error}"),
        })?;
        let path = entry.path();
        if path.extension().and_then(|extension| extension.to_str()) != Some("ndjson") {
            continue;
        }
        read_matching_log_file(&path, session_id, &mut sequence, &mut records)?;
    }

    records.sort_by_key(|record| (record.timestamp_ms, record.sequence));
    if records.len() > limit {
        records.drain(0..records.len() - limit);
    }
    Ok(records)
}

fn read_matching_log_file(
    path: &Path,
    session_id: &str,
    sequence: &mut usize,
    records: &mut Vec<LogLine>,
) -> Result<(), DaemonError> {
    let file = File::open(path).map_err(|error| DaemonError::LocalTransport {
        operation: "debug bundle export",
        message: format!("failed to open log file `{}`: {error}", path.display()),
    })?;
    for line in BufReader::new(file).lines() {
        let line = line.map_err(|error| DaemonError::LocalTransport {
            operation: "debug bundle export",
            message: format!("failed to read log file `{}`: {error}", path.display()),
        })?;
        if !log_line_matches_session(&line, session_id) {
            *sequence += 1;
            continue;
        }
        let timestamp_ms = serde_json::from_str::<Value>(&line)
            .ok()
            .and_then(|value| value.get("timestamp_ms").and_then(Value::as_u64))
            .unwrap_or(0);
        records.push(LogLine {
            timestamp_ms,
            sequence: *sequence,
            line,
        });
        *sequence += 1;
    }
    Ok(())
}

fn log_line_matches_session(line: &str, session_id: &str) -> bool {
    serde_json::from_str::<Value>(line)
        .ok()
        .and_then(|value| {
            value
                .get("session_id")
                .and_then(Value::as_str)
                .map(str::to_string)
        })
        .as_deref()
        == Some(session_id)
}

fn default_debug_bundle_dir(log_root: &Path, session_id: &str, label: Option<&str>) -> PathBuf {
    let root = log_root
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| log_root.to_path_buf())
        .join("debug-bundles");
    let safe_session = safe_path_segment(session_id);
    let safe_label = label
        .map(safe_path_segment)
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| unix_epoch_ms().to_string());
    root.join(format!("{safe_session}-{safe_label}"))
}

fn safe_path_segment(value: &str) -> String {
    value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-') {
                ch
            } else {
                '_'
            }
        })
        .collect::<String>()
        .trim_matches('_')
        .to_string()
}

fn write_text_file(path: &Path, content: &str) -> Result<(), DaemonError> {
    let mut file = File::create(path).map_err(|error| DaemonError::LocalTransport {
        operation: "debug bundle export",
        message: format!("failed to create `{}`: {error}", path.display()),
    })?;
    file.write_all(content.as_bytes())
        .map_err(|error| DaemonError::LocalTransport {
            operation: "debug bundle export",
            message: format!("failed to write `{}`: {error}", path.display()),
        })
}

fn unix_epoch_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .try_into()
        .unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debug_bundle_exports_session_filtered_logs_under_kernel_root() {
        let root = test_root("debug-bundle-session-filter");
        let log_root = root.join("logs");
        fs::create_dir_all(&log_root).expect("log root should be created");
        write_text_file(
            &log_root.join("daemon.ndjson"),
            r#"{"timestamp_ms":1,"session_id":"session-1","message":"old"}"#
                .to_string()
                .as_str(),
        )
        .expect("log should be written");
        write_text_file(
            &log_root.join("worker.ndjson"),
            [
                r#"{"timestamp_ms":3,"session_id":"session-2","message":"other"}"#,
                r#"{"timestamp_ms":2,"session_id":"session-1","message":"new"}"#,
            ]
            .join("\n")
            .as_str(),
        )
        .expect("log should be written");

        let export = export_debug_bundle_from_log_root(
            &log_root,
            ExportDebugBundleRequest {
                session_id: "session-1".to_string(),
                bundle_label: Some("../../support bundle".to_string()),
                limit: Some(10),
            },
        )
        .expect("debug bundle should export");

        assert_eq!(export.record_count, 2);
        assert!(export.bundle_dir.starts_with(root.join("debug-bundles")));
        assert!(export.bundle_dir.ends_with("session-1-support_bundle"));
        let logs = fs::read_to_string(export.logs_path).expect("logs should be readable");
        assert!(logs.contains(r#""message":"old""#));
        assert!(logs.contains(r#""message":"new""#));
        assert!(!logs.contains("other"));
        let manifest =
            fs::read_to_string(export.manifest_path).expect("manifest should be readable");
        assert!(manifest.contains(r#""schema": "chariox.debug_bundle.v1""#));
        assert!(manifest.contains(r#""session_id": "session-1""#));

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn debug_bundle_applies_limit_after_timestamp_sort() {
        let root = test_root("debug-bundle-limit");
        let log_root = root.join("logs");
        fs::create_dir_all(&log_root).expect("log root should be created");
        write_text_file(
            &log_root.join("daemon.ndjson"),
            [
                r#"{"timestamp_ms":1,"session_id":"session-1","message":"first"}"#,
                r#"{"timestamp_ms":2,"session_id":"session-1","message":"second"}"#,
            ]
            .join("\n")
            .as_str(),
        )
        .expect("log should be written");

        let export = export_debug_bundle_from_log_root(
            &log_root,
            ExportDebugBundleRequest {
                session_id: "session-1".to_string(),
                bundle_label: Some("limit".to_string()),
                limit: Some(1),
            },
        )
        .expect("debug bundle should export");

        assert_eq!(export.record_count, 1);
        let logs = fs::read_to_string(export.logs_path).expect("logs should be readable");
        assert!(!logs.contains("first"));
        assert!(logs.contains("second"));

        let _ = fs::remove_dir_all(root);
    }

    fn test_root(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "chariox-{name}-{}-{}",
            std::process::id(),
            unix_epoch_ms()
        ));
        let _ = fs::remove_dir_all(&root);
        root
    }
}
