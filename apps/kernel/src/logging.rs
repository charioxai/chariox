use std::env;
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde_json::{json, Map, Value};

const DEFAULT_LOG_RETENTION_DAYS: u64 = 7;
const DEFAULT_LOG_ROOT_MAX_BYTES: u64 = 200 * 1024 * 1024;
const DEFAULT_LOG_FILE_MAX_BYTES: u64 = 32 * 1024 * 1024;
const DEFAULT_LOG_FLUSH_INTERVAL_BYTES: u64 = 256 * 1024;
const MAX_LOG_RECORD_BYTES: usize = 64 * 1024;
const MAX_LOG_STRING_BYTES: usize = 16 * 1024;
const MAX_LOG_ARRAY_ITEMS: usize = 64;
const MAX_LOG_OBJECT_FIELDS: usize = 64;

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
    writer: Option<Mutex<BoundedLogWriter>>,
}

struct BoundedLogWriter {
    log_root: PathBuf,
    startup_epoch_ms: u64,
    process_kind: String,
    pid: u32,
    segment: u64,
    current_path: PathBuf,
    file: File,
    current_bytes: u64,
    bytes_since_flush: u64,
    max_file_bytes: u64,
    flush_interval_bytes: u64,
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
        Some(Mutex::new(BoundedLogWriter::new(
            log_root.clone(),
            startup_epoch_ms,
            process_kind,
            std::process::id(),
            log_path.clone(),
        )?))
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
    if let Some(path) = env::var_os("CHARIOX_LOG_DIR")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
    {
        return path;
    }

    if let Some(path) = env::var_os("XDG_STATE_HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
    {
        return path.join("chariox").join("logs");
    }

    if let Some(path) = env::var_os("HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
    {
        return path
            .join(".local")
            .join("state")
            .join("chariox")
            .join("logs");
    }

    std::env::current_dir()
        .unwrap_or_else(|_| std::env::temp_dir())
        .join(".chariox")
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
    record.insert(
        "process_kind".to_string(),
        Value::from(logger.process_kind.clone()),
    );
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

    let line = encode_log_record(record);
    let flush_immediately = level >= LogLevel::Warn;
    let _ = writer.write_line(&line, flush_immediately);
}

fn configured_log_level() -> LogLevel {
    match env::var("CHARIOX_LOG_LEVEL")
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

impl BoundedLogWriter {
    fn new(
        log_root: PathBuf,
        startup_epoch_ms: u64,
        process_kind: &str,
        pid: u32,
        initial_path: PathBuf,
    ) -> std::io::Result<Self> {
        Self::new_with_limits(
            log_root,
            startup_epoch_ms,
            process_kind,
            pid,
            initial_path,
            DEFAULT_LOG_FILE_MAX_BYTES,
            DEFAULT_LOG_FLUSH_INTERVAL_BYTES,
        )
    }

    fn new_with_limits(
        log_root: PathBuf,
        startup_epoch_ms: u64,
        process_kind: &str,
        pid: u32,
        initial_path: PathBuf,
        max_file_bytes: u64,
        flush_interval_bytes: u64,
    ) -> std::io::Result<Self> {
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&initial_path)?;
        let current_bytes = file.metadata().map(|metadata| metadata.len()).unwrap_or(0);
        Ok(Self {
            log_root,
            startup_epoch_ms,
            process_kind: process_kind.to_string(),
            pid,
            segment: 0,
            current_path: initial_path,
            file,
            current_bytes,
            bytes_since_flush: 0,
            max_file_bytes: max_file_bytes.max(1),
            flush_interval_bytes: flush_interval_bytes.max(1),
        })
    }

    fn write_line(&mut self, line: &str, flush_immediately: bool) -> std::io::Result<()> {
        let line_bytes = line.len() as u64 + 1;
        if self.current_bytes > 0
            && self.current_bytes.saturating_add(line_bytes) > self.max_file_bytes
        {
            self.rotate()?;
        }

        self.file.write_all(line.as_bytes())?;
        self.file.write_all(b"\n")?;
        self.current_bytes = self.current_bytes.saturating_add(line_bytes);
        self.bytes_since_flush = self.bytes_since_flush.saturating_add(line_bytes);

        if flush_immediately || self.bytes_since_flush >= self.flush_interval_bytes {
            self.file.flush()?;
            self.bytes_since_flush = 0;
        }

        Ok(())
    }

    fn rotate(&mut self) -> std::io::Result<()> {
        let _ = self.file.flush();
        self.segment = self.segment.saturating_add(1);
        self.current_path = self.log_root.join(format!(
            "{}-{}-{}-{}.ndjson",
            self.startup_epoch_ms, self.process_kind, self.pid, self.segment
        ));
        self.file = OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .open(&self.current_path)?;
        self.current_bytes = 0;
        self.bytes_since_flush = 0;
        cleanup_log_root(&self.log_root, &self.current_path)
    }
}

fn encode_log_record(mut record: Map<String, Value>) -> String {
    if let Some(fields) = record.get_mut("fields") {
        compact_log_value(fields);
    }
    for value in record.values_mut() {
        compact_log_value(value);
    }

    let value = Value::Object(record);
    match serde_json::to_string(&value) {
        Ok(line) if line.len() <= MAX_LOG_RECORD_BYTES => line,
        Ok(line) => encode_truncated_log_record(value, line.len()),
        Err(_) => fallback_log_record("failed to serialize log record"),
    }
}

fn encode_truncated_log_record(value: Value, original_bytes: usize) -> String {
    let object = value.as_object();
    let mut record = Map::new();
    for key in [
        "timestamp_ms",
        "level",
        "process_kind",
        "pid",
        "component",
        "message",
        "log_path",
    ] {
        if let Some(value) = object.and_then(|object| object.get(key)) {
            record.insert(key.to_string(), value.clone());
        }
    }
    record.insert(
        "chariox_log_record_truncated".to_string(),
        Value::from(true),
    );
    record.insert(
        "chariox_original_record_bytes".to_string(),
        Value::from(original_bytes as u64),
    );
    let encoded = serde_json::to_string(&Value::Object(record))
        .unwrap_or_else(|_| fallback_log_record("failed to serialize truncated log record"));
    if encoded.len() <= MAX_LOG_RECORD_BYTES {
        return encoded;
    }
    fallback_log_record("log record exceeded maximum size")
}

fn fallback_log_record(message: &str) -> String {
    serde_json::to_string(&json!({
        "timestamp_ms": unix_epoch_ms(),
        "level": "warn",
        "component": "logging",
        "message": message,
        "chariox_log_record_truncated": true,
    }))
    .unwrap_or_else(|_| "{\"level\":\"warn\",\"component\":\"logging\"}".to_string())
}

fn compact_log_value(value: &mut Value) {
    match value {
        Value::String(text) => compact_log_string(text),
        Value::Array(items) => {
            let original_len = items.len();
            for item in items.iter_mut().take(MAX_LOG_ARRAY_ITEMS) {
                compact_log_value(item);
            }
            if original_len > MAX_LOG_ARRAY_ITEMS {
                items.truncate(MAX_LOG_ARRAY_ITEMS);
                items.push(json!({
                    "chariox_truncated": true,
                    "chariox_original_items": original_len,
                }));
            }
        }
        Value::Object(object) => {
            let original_len = object.len();
            let keys = object.keys().cloned().collect::<Vec<_>>();
            for key in keys.iter().take(MAX_LOG_OBJECT_FIELDS) {
                if let Some(value) = object.get_mut(key) {
                    compact_log_value(value);
                }
            }
            if original_len > MAX_LOG_OBJECT_FIELDS {
                for key in keys.into_iter().skip(MAX_LOG_OBJECT_FIELDS) {
                    object.remove(&key);
                }
                object.insert(
                    "chariox_truncated_fields".to_string(),
                    Value::from(original_len.saturating_sub(MAX_LOG_OBJECT_FIELDS) as u64),
                );
            }
        }
        _ => {}
    }
}

fn compact_log_string(text: &mut String) {
    if text.len() <= MAX_LOG_STRING_BYTES {
        return;
    }
    let original_bytes = text.len();
    let mut end = MAX_LOG_STRING_BYTES;
    while !text.is_char_boundary(end) {
        end = end.saturating_sub(1);
    }
    text.truncate(end);
    text.push_str(&format!(
        "\n[chariox log value truncated: original_bytes={original_bytes}, retained_bytes={end}]"
    ));
}

fn cleanup_log_root(log_root: &Path, current_log_path: &Path) -> std::io::Result<()> {
    cleanup_log_root_with_limit(log_root, current_log_path, DEFAULT_LOG_ROOT_MAX_BYTES)
}

fn cleanup_log_root_with_limit(
    log_root: &Path,
    current_log_path: &Path,
    max_root_bytes: u64,
) -> std::io::Result<()> {
    let now = SystemTime::now();
    let retention = Duration::from_secs(DEFAULT_LOG_RETENTION_DAYS * 24 * 60 * 60);
    let mut files = Vec::new();

    for entry in fs::read_dir(log_root)? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("ndjson") {
            continue;
        }

        let metadata = entry.metadata()?;
        let modified = metadata.modified().unwrap_or(now);
        if path != current_log_path && now.duration_since(modified).unwrap_or_default() > retention
        {
            let _ = fs::remove_file(&path);
            continue;
        }

        files.push((path, metadata.len(), modified));
    }

    let mut total_bytes: u64 = files.iter().map(|(_, size, _)| *size).sum();
    if total_bytes <= max_root_bytes {
        return Ok(());
    }

    files.sort_by_key(|(_, _, modified)| *modified);
    for (path, size, _) in files {
        if total_bytes <= max_root_bytes {
            break;
        }
        if path == current_log_path {
            continue;
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

#[cfg(test)]
mod tests {
    use std::fs;
    use std::sync::atomic::{AtomicU64, Ordering};

    use serde_json::Value;

    use super::*;

    fn temp_log_dir(name: &str) -> PathBuf {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        let path = std::env::temp_dir().join(format!(
            "chariox-logging-{name}-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::SeqCst)
        ));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).expect("create temp log dir");
        path
    }

    fn write_sized_file(path: &Path, size: usize) {
        fs::write(path, "x".repeat(size)).expect("write sized file");
    }

    fn ndjson_files(dir: &Path) -> Vec<PathBuf> {
        let mut files = fs::read_dir(dir)
            .expect("read log dir")
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| path.extension().and_then(|ext| ext.to_str()) == Some("ndjson"))
            .collect::<Vec<_>>();
        files.sort();
        files
    }

    #[test]
    fn bounded_writer_rotates_active_file_before_it_exceeds_limit() {
        let dir = temp_log_dir("rotate");
        let initial = dir.join("100-daemon-7.ndjson");
        let mut writer =
            BoundedLogWriter::new_with_limits(dir.clone(), 100, "daemon", 7, initial, 80, 1)
                .expect("create bounded writer");

        writer
            .write_line(&"a".repeat(50), true)
            .expect("write first line");
        writer
            .write_line(&"b".repeat(50), true)
            .expect("write second line");
        drop(writer);

        let files = ndjson_files(&dir);
        assert_eq!(files.len(), 2);
        for path in files {
            let size = fs::metadata(path).expect("read metadata").len();
            assert!(size <= 80, "rotated log segment exceeded limit: {size}");
        }

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn cleanup_counts_active_file_against_root_budget() {
        let dir = temp_log_dir("cleanup");
        let old_a = dir.join("1-daemon-7.ndjson");
        let old_b = dir.join("2-daemon-7.ndjson");
        let current = dir.join("3-daemon-7.ndjson");
        write_sized_file(&old_a, 80);
        write_sized_file(&old_b, 80);
        write_sized_file(&current, 80);

        cleanup_log_root_with_limit(&dir, &current, 120).expect("cleanup log root");

        assert!(current.exists(), "cleanup must preserve active log file");
        let total = ndjson_files(&dir)
            .iter()
            .map(|path| fs::metadata(path).expect("metadata").len())
            .sum::<u64>();
        assert!(
            total <= 120,
            "cleanup did not account for the active file: {total}"
        );

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn log_record_compacts_large_fields_before_writing() {
        let mut record = Map::new();
        record.insert("timestamp_ms".to_string(), Value::from(1));
        record.insert("level".to_string(), Value::from("info"));
        record.insert("process_kind".to_string(), Value::from("daemon"));
        record.insert("pid".to_string(), Value::from(7));
        record.insert("component".to_string(), Value::from("test"));
        record.insert("message".to_string(), Value::from("large field"));
        record.insert(
            "provider_output".to_string(),
            Value::from("x".repeat(200_000)),
        );

        let line = encode_log_record(record);

        assert!(line.len() <= MAX_LOG_RECORD_BYTES);
        assert!(line.contains("chariox log value truncated"));
        assert!(!line.contains(&"x".repeat(MAX_LOG_STRING_BYTES + 1)));
        let parsed: Value = serde_json::from_str(&line).expect("valid compacted log json");
        assert_eq!(
            parsed
                .get("provider_output")
                .and_then(Value::as_str)
                .map(|value| value.len() < 200_000),
            Some(true)
        );
    }

    #[test]
    fn log_record_replaces_oversized_records_with_metadata() {
        let mut record = Map::new();
        record.insert("timestamp_ms".to_string(), Value::from(1));
        record.insert("level".to_string(), Value::from("info"));
        record.insert("process_kind".to_string(), Value::from("daemon"));
        record.insert("pid".to_string(), Value::from(7));
        record.insert("component".to_string(), Value::from("test"));
        record.insert("message".to_string(), Value::from("large record"));
        for index in 0..80 {
            record.insert(format!("field_{index}"), Value::from("x".repeat(2_000)));
        }

        let line = encode_log_record(record);

        assert!(line.len() <= MAX_LOG_RECORD_BYTES);
        let parsed: Value = serde_json::from_str(&line).expect("valid truncated log json");
        assert_eq!(
            parsed.get("chariox_log_record_truncated"),
            Some(&Value::Bool(true))
        );
        assert_eq!(
            parsed
                .get("chariox_original_record_bytes")
                .and_then(Value::as_u64)
                .is_some(),
            true
        );
    }
}
