use std::env;
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde_json::{json, Map, Value};

const DEFAULT_LOG_RETENTION_DAYS: u64 = 7;
const DEFAULT_LOG_ROOT_MAX_BYTES: u64 = 200 * 1024 * 1024;

static LOGGER: OnceLock<ProcessLogger> = OnceLock::new();

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum LogLevel {
    Debug,
    Info,
    Warn,
    Error,
    Off,
}

impl LogLevel {
    fn as_str(self) -> &'static str {
        match self {
            Self::Debug => "debug",
            Self::Info => "info",
            Self::Warn => "warn",
            Self::Error => "error",
            Self::Off => "off",
        }
    }
}

struct ProcessLogger {
    process_kind: String,
    pid: u32,
    level: LogLevel,
    log_path: Option<PathBuf>,
    writer: Option<Mutex<File>>,
}

pub fn init_process_logger(process_kind: &str) -> std::io::Result<PathBuf> {
    let log_root = default_log_root();
    fs::create_dir_all(&log_root)?;
    let level = configured_log_level();

    let startup_epoch_ms = unix_epoch_ms();
    let log_path = log_root.join(format!(
        "{}-{}-{}.ndjson",
        startup_epoch_ms,
        process_kind,
        std::process::id()
    ));
    cleanup_log_root(&log_root, &log_path)?;

    let writer = if level == LogLevel::Off {
        None
    } else {
        Some(Mutex::new(
            OpenOptions::new()
                .create(true)
                .append(true)
                .open(&log_path)?,
        ))
    };

    let logger = ProcessLogger {
        process_kind: process_kind.to_string(),
        pid: std::process::id(),
        level,
        log_path: if level == LogLevel::Off {
            None
        } else {
            Some(log_path.clone())
        },
        writer,
    };

    let _ = LOGGER.set(logger);
    if level != LogLevel::Off {
        info_with_fields(
            "logging",
            "initialized process logger",
            json!({
                "log_root": log_root.display().to_string(),
                "log_path": log_path.display().to_string(),
            }),
        );
    }

    Ok(log_path)
}

pub fn default_log_root() -> PathBuf {
    if let Some(path) = env::var_os("ARROBA_LOG_DIR")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
    {
        return path;
    }

    if let Some(path) = env::var_os("XDG_STATE_HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
    {
        return path.join("arroba").join("logs");
    }

    if let Some(path) = env::var_os("HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
    {
        return path.join(".local").join("state").join("arroba").join("logs");
    }

    std::env::current_dir()
        .unwrap_or_else(|_| std::env::temp_dir())
        .join(".arroba")
        .join("logs")
}

pub fn debug(component: &str, message: impl Into<String>) {
    log(LogLevel::Debug, component, message.into(), Value::Null);
}

pub fn debug_with_fields(component: &str, message: impl Into<String>, fields: Value) {
    log(LogLevel::Debug, component, message.into(), fields);
}

pub fn info(component: &str, message: impl Into<String>) {
    log(LogLevel::Info, component, message.into(), Value::Null);
}

pub fn info_with_fields(component: &str, message: impl Into<String>, fields: Value) {
    log(LogLevel::Info, component, message.into(), fields);
}

pub fn warn_with_fields(component: &str, message: impl Into<String>, fields: Value) {
    log(LogLevel::Warn, component, message.into(), fields);
}

pub fn error_with_fields(component: &str, message: impl Into<String>, fields: Value) {
    log(LogLevel::Error, component, message.into(), fields);
}

fn log(level: LogLevel, component: &str, message: String, fields: Value) {
    let Some(logger) = LOGGER.get() else {
        return;
    };

    if level < logger.level {
        return;
    }

    let mut record = Map::new();
    record.insert("timestamp_ms".to_string(), Value::from(unix_epoch_ms()));
    record.insert("level".to_string(), Value::from(level.as_str()));
    record.insert("process_kind".to_string(), Value::from(logger.process_kind.clone()));
    record.insert("pid".to_string(), Value::from(logger.pid));
    record.insert("component".to_string(), Value::from(component.to_string()));
    record.insert("message".to_string(), Value::from(message));
    if let Some(log_path) = &logger.log_path {
        record.insert(
            "log_path".to_string(),
            Value::from(log_path.display().to_string()),
        );
    }

    if let Some(object) = fields.as_object() {
        for (key, value) in object {
            record.insert(key.clone(), value.clone());
        }
    }

    let Some(writer) = &logger.writer else {
        return;
    };

    let mut writer = match writer.lock() {
        Ok(guard) => guard,
        Err(_) => return,
    };

    let _ = serde_json::to_writer(&mut *writer, &Value::Object(record));
    let _ = writer.write_all(b"\n");
    let _ = writer.flush();
}

fn configured_log_level() -> LogLevel {
    match env::var("ARROBA_LOG_LEVEL")
        .ok()
        .as_deref()
        .unwrap_or("info")
        .to_ascii_lowercase()
        .as_str()
    {
        "debug" => LogLevel::Debug,
        "warn" => LogLevel::Warn,
        "error" => LogLevel::Error,
        "off" => LogLevel::Off,
        _ => LogLevel::Info,
    }
}

fn cleanup_log_root(log_root: &Path, current_log_path: &Path) -> std::io::Result<()> {
    let now = SystemTime::now();
    let retention = Duration::from_secs(DEFAULT_LOG_RETENTION_DAYS * 24 * 60 * 60);
    let mut files = Vec::new();

    for entry in fs::read_dir(log_root)? {
        let entry = entry?;
        let path = entry.path();
        if path == current_log_path || path.extension().and_then(|ext| ext.to_str()) != Some("ndjson") {
            continue;
        }

        let metadata = entry.metadata()?;
        let modified = metadata.modified().unwrap_or(now);
        if now.duration_since(modified).unwrap_or_default() > retention {
            let _ = fs::remove_file(&path);
            continue;
        }

        files.push((path, metadata.len(), modified));
    }

    let mut total_bytes: u64 = files.iter().map(|(_, size, _)| *size).sum();
    if total_bytes <= DEFAULT_LOG_ROOT_MAX_BYTES {
        return Ok(());
    }

    files.sort_by_key(|(_, _, modified)| *modified);
    for (path, size, _) in files {
        if total_bytes <= DEFAULT_LOG_ROOT_MAX_BYTES {
            break;
        }
        let _ = fs::remove_file(path);
        total_bytes = total_bytes.saturating_sub(size);
    }

    Ok(())
}

fn unix_epoch_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .try_into()
        .unwrap_or(u64::MAX)
}
