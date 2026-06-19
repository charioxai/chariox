use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::PathBuf;

use serde::Serialize;

const DEFAULT_TERMINAL_TURN_TRACE_MAX_MB: u64 = 100;
const DEFAULT_TERMINAL_TURN_TRACE_DIR_MAX_MB: u64 = 500;

pub(crate) fn record_terminal_turn(session_id: &str, source: &str, payload: impl Serialize) {
    if std::env::var_os("ARROBA_DISABLE_TERMINAL_TURN_TRACE").is_some() {
        return;
    }
    if std::env::var("ARROBA_TERMINAL_TURN_TRACE").ok().as_deref() != Some("1") {
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
    if let Some(parent) = path.parent() {
        enforce_trace_dir_budget(parent, terminal_turn_trace_dir_max_bytes());
    }
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

fn terminal_turn_trace_dir_max_bytes() -> u64 {
    let max_mb = std::env::var("ARROBA_TERMINAL_TURN_TRACE_DIR_MAX_MB")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(DEFAULT_TERMINAL_TURN_TRACE_DIR_MAX_MB)
        .min(DEFAULT_TERMINAL_TURN_TRACE_DIR_MAX_MB);
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

fn enforce_trace_dir_budget(dir: &std::path::Path, max_bytes: u64) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    let mut files = entries
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let metadata = entry.metadata().ok()?;
            if !metadata.is_file() {
                return None;
            }
            let modified = metadata.modified().ok();
            Some((entry.path(), metadata.len(), modified))
        })
        .collect::<Vec<_>>();
    let mut total = files.iter().map(|(_, size, _)| *size).sum::<u64>();
    if total <= max_bytes {
        return;
    }
    files.sort_by_key(|(_, _, modified)| *modified);
    for (path, size, _) in files {
        if total <= max_bytes {
            break;
        }
        if fs::remove_file(path).is_ok() {
            total = total.saturating_sub(size);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Mutex, OnceLock};

    use super::*;

    fn env_lock() -> std::sync::MutexGuard<'static, ()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(())).lock().unwrap()
    }

    #[test]
    fn terminal_turn_trace_is_disabled_by_default() {
        let _guard = env_lock();
        let dir = std::env::temp_dir().join(format!(
            "arroba-terminal-turn-trace-disabled-{}",
            crate::session::unix_epoch_ms()
        ));
        let _ = fs::remove_dir_all(&dir);
        std::env::remove_var("ARROBA_DISABLE_TERMINAL_TURN_TRACE");
        std::env::remove_var("ARROBA_TERMINAL_TURN_TRACE");
        std::env::set_var("ARROBA_TERMINAL_TURN_TRACE_DIR", &dir);

        record_terminal_turn("session-1", "test", serde_json::json!({"ok": true}));

        assert!(
            !dir.exists(),
            "terminal turn traces should not be created unless explicitly enabled"
        );
        std::env::remove_var("ARROBA_TERMINAL_TURN_TRACE_DIR");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn terminal_turn_trace_writes_when_enabled() {
        let _guard = env_lock();
        let dir = std::env::temp_dir().join(format!(
            "arroba-terminal-turn-trace-enabled-{}",
            crate::session::unix_epoch_ms()
        ));
        let _ = fs::remove_dir_all(&dir);
        std::env::remove_var("ARROBA_DISABLE_TERMINAL_TURN_TRACE");
        std::env::set_var("ARROBA_TERMINAL_TURN_TRACE", "1");
        std::env::set_var("ARROBA_TERMINAL_TURN_TRACE_DIR", &dir);

        record_terminal_turn("session-1", "test", serde_json::json!({"ok": true}));

        assert!(
            dir.join("session-1.jsonl").exists(),
            "enabled terminal turn tracing should write a session trace"
        );
        std::env::remove_var("ARROBA_TERMINAL_TURN_TRACE");
        std::env::remove_var("ARROBA_TERMINAL_TURN_TRACE_DIR");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn terminal_turn_trace_dir_budget_removes_old_files() {
        let dir = std::env::temp_dir().join(format!(
            "arroba-terminal-turn-trace-budget-{}",
            crate::session::unix_epoch_ms()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("trace dir should be created");
        fs::write(dir.join("old.jsonl"), vec![b'a'; 128]).expect("old trace should write");
        fs::write(dir.join("new.jsonl"), vec![b'b'; 128]).expect("new trace should write");

        enforce_trace_dir_budget(&dir, 128);

        let remaining = fs::read_dir(&dir)
            .expect("trace dir should list")
            .filter_map(Result::ok)
            .count();
        assert_eq!(remaining, 1);
        let _ = fs::remove_dir_all(&dir);
    }
}
