//! MP-08 / MP-10: the live rows of a Claude run that their transcript block
//! has not replaced yet. They are kept next to the run's context file, so a
//! kernel restart, which forgets what the terminal reader streamed, still
//! knows which stored rows a block may replace, and a row is replaced once.
use std::fs;
use std::path::PathBuf;

const AWAITING_ROWS_FILE: &str = "live-rows-awaiting";

fn awaiting_file(context_file: &str) -> PathBuf {
    std::path::Path::new(context_file).with_file_name(AWAITING_ROWS_FILE)
}

fn read(context_file: &str) -> Vec<String> {
    fs::read_to_string(awaiting_file(context_file))
        .map(|raw| raw.lines().map(str::to_string).collect())
        .unwrap_or_default()
}

fn write(context_file: &str, rows: &[String]) {
    let path = awaiting_file(context_file);
    if rows.is_empty() {
        let _ = fs::remove_file(path);
    } else {
        let _ = fs::write(path, rows.join("\n") + "\n");
    }
}

/// The row under `merge_key` streams and waits for its transcript block.
pub(super) fn note_streaming(context_file: &str, merge_key: &str) {
    let mut rows = read(context_file);
    if !rows.iter().any(|row| row == merge_key) {
        rows.push(merge_key.to_string());
        write(context_file, &rows);
    }
}

/// The row under `merge_key` got its transcript block.
pub(super) fn note_replaced(context_file: &str, merge_key: &str) {
    let mut rows = read(context_file);
    let before = rows.len();
    rows.retain(|row| row != merge_key);
    if rows.len() != before {
        write(context_file, &rows);
    }
}

/// The newest row still waiting whose merge key starts with `prefix`.
pub(super) fn newest_waiting(context_file: &str, prefix: &str) -> Option<String> {
    read(context_file)
        .into_iter()
        .rev()
        .find(|row| row.starts_with(prefix))
}
