use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::PathBuf;

use serde::Serialize;

const DEFAULT_TERMINAL_TURN_TRACE_MAX_MB: u64 = 100;

pub(crate) fn record_terminal_turn(session_id: &str, source: &str, payload: impl Serialize) {
    if std::env::var_os("ARROBA_DISABLE_TERMINAL_TURN_TRACE").is_some() {
        return;
    }
    let Some(path) = terminal_turn_trace_path(session_id) else {
        return;
    };
    if let Some(parent) = path.parent() {
        if fs::create_dir_all(parent).is_err() {
            return;
        }
    }
    rotate_if_needed(&path, terminal_turn_trace_max_bytes());
    let record = serde_json::json!({
        "at_ms": crate::session::unix_epoch_ms(),
        "source": source,
        "session_id": session_id,
        "payload": payload,
    });
    let Ok(line) = serde_json::to_string(&record) else {
        return;
    };
    let Ok(mut file) = OpenOptions::new().create(true).append(true).open(&path) else {
        return;
    };
    let _ = writeln!(file, "{line}");
}

fn terminal_turn_trace_path(session_id: &str) -> Option<PathBuf> {
    let base = std::env::var_os("ARROBA_TERMINAL_TURN_TRACE_DIR")
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME")
                .map(PathBuf::from)
                .map(|home| home.join(".arroba").join("debug").join("terminal-turns"))
        })?;
    Some(base.join(format!(
        "{}.jsonl",
        sanitize_trace_file_component(session_id)
    )))
}

fn sanitize_trace_file_component(value: &str) -> String {
    value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
                ch
            } else {
                '_'
            }
        })
        .collect()
}

fn terminal_turn_trace_max_bytes() -> u64 {
    let max_mb = std::env::var("ARROBA_TERMINAL_TURN_TRACE_MAX_MB")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(DEFAULT_TERMINAL_TURN_TRACE_MAX_MB)
        .min(500);
    max_mb * 1024 * 1024
}

fn rotate_if_needed(path: &PathBuf, max_bytes: u64) {
    let Ok(metadata) = fs::metadata(path) else {
        return;
    };
    if metadata.len() < max_bytes {
        return;
    }
    let rotated = path.with_extension("jsonl.1");
    let _ = fs::remove_file(&rotated);
    let _ = fs::rename(path, rotated);
}
