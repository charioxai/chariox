//! The watcher contract: what an agent may declare, the one JSON line the script reads and the
//! JSON lines it writes. Checked here once; the rest of the kernel trusts it.
use crate::durable_state::workflow_notifications::watcher::MAX_CURSOR_BYTES;
use crate::local::{WatcherEvent, WatcherManifest, WatcherMode, WatcherScript};

pub(crate) const MIN_POLL_S: u64 = 60;
const MAX_POLL_S: u64 = 7 * 24 * 60 * 60;
const MAX_DEBOUNCE_S: u64 = 60 * 60;
const MAX_SCRIPT_BYTES: usize = 256 * 1024;
const MAX_TEXT_CHARS: usize = 2000;
const MAX_READING_CHARS: usize = 300;

/// A watcher's name: what the person calls it in the Notification center.
pub(crate) fn name(name: &str) -> Result<String, String> {
    let trimmed = name.trim();
    if trimmed.is_empty() || trimmed.chars().count() > 80 || trimmed != name {
        return Err("name: 1–80 characters, no surrounding spaces".into());
    }
    Ok(name.to_owned())
}

pub(crate) fn validate(manifest: &WatcherManifest, script: &WatcherScript) -> Result<(), String> {
    name(&manifest.name)?;
    let sentence = manifest.sentence.trim();
    if sentence.is_empty() || sentence.chars().count() > 300 || sentence.contains('\n') {
        return Err("sentence: one line of 1–300 characters".into());
    }
    if let WatcherMode::Poll { every_s } = manifest.mode {
        if !(MIN_POLL_S..=MAX_POLL_S).contains(&every_s) {
            return Err(format!(
                "mode.poll.every_s: {MIN_POLL_S}–{MAX_POLL_S} seconds"
            ));
        }
    }
    if manifest.command.is_empty()
        || manifest.command.len() > 64
        || manifest
            .command
            .iter()
            .any(|arg| arg.is_empty() || arg.contains('\0') || arg.len() > 4096)
    {
        return Err("command: 1–64 non-empty arguments".into());
    }
    if manifest.events.is_empty() || manifest.events.len() > 32 {
        return Err("events: declare 1–32 event names".into());
    }
    for (n, event) in manifest.events.iter().enumerate() {
        if !event_name(&event.name) {
            return Err(format!(
                "events[{n}].name: 1–64 of a-z, 0-9, '_', '-' and '.'"
            ));
        }
        if manifest.events[..n].iter().any(|e| e.name == event.name) {
            return Err(format!("events[{n}].name: declared twice"));
        }
    }
    if manifest.debounce_s > MAX_DEBOUNCE_S {
        return Err(format!("debounce_s: 0–{MAX_DEBOUNCE_S} seconds"));
    }
    let file = script.file_name.as_str();
    if file.is_empty()
        || file.len() > 128
        || file == "."
        || file == ".."
        || file.contains(['/', '\\', '\0'])
    {
        return Err("script.file_name: one plain file name".into());
    }
    if script.text.len() > MAX_SCRIPT_BYTES {
        return Err(format!("script.text: at most {MAX_SCRIPT_BYTES} bytes"));
    }
    Ok(())
}

fn event_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"_-.".contains(&b))
}

/// The one line the script reads on its standard input.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub(crate) struct Input {
    pub cursor: Option<serde_json::Value>,
    pub now_ms: u64,
    pub dry_run: bool,
    pub since_ms: Option<u64>,
}

/// One line the script wrote.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Report {
    Reading(String),
    Event(WatcherEvent),
    History(bool),
    Cursor(serde_json::Value),
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct ReadingLine {
    reading: String,
}
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct HistoryLine {
    history: bool,
}
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct CursorLine {
    cursor: serde_json::Value,
}

/// Reads one output line. A line the contract does not know, or an event the manifest did not
/// declare, fails the run: a watcher never loses what it reports without saying so.
pub(crate) fn parse_line(manifest: &WatcherManifest, line: &str) -> Result<Report, String> {
    let value: serde_json::Value = serde_json::from_str(line)
        .map_err(|_| format!("output is not a JSON line: {}", excerpt(line)))?;
    let object = value
        .as_object()
        .ok_or_else(|| format!("output is not a JSON object: {}", excerpt(line)))?;
    let kind = ["event", "reading", "history", "cursor", "candidate"]
        .into_iter()
        .find(|kind| object.contains_key(*kind));
    let parsed = match kind {
        Some("event") => serde_json::from_value::<WatcherEvent>(value).map(Report::Event),
        Some("reading") => {
            serde_json::from_value::<ReadingLine>(value).map(|l| Report::Reading(l.reading))
        }
        Some("history") => {
            serde_json::from_value::<HistoryLine>(value).map(|l| Report::History(l.history))
        }
        Some("cursor") => {
            serde_json::from_value::<CursorLine>(value).map(|l| Report::Cursor(l.cursor))
        }
        Some(_) => {
            return Err("a candidate line needs a judge, which this watcher does not have".into())
        }
        None => return Err(format!("output line of no known kind: {}", excerpt(line))),
    }
    .map_err(|e| format!("{e}: {}", excerpt(line)))?;
    match &parsed {
        Report::Event(event) => {
            let declared = manifest
                .events
                .iter()
                .find(|declared| declared.name == event.event)
                .ok_or_else(|| format!("event {:?} is not declared", event.event))?;
            if declared.keyed != event.key.is_some() {
                return Err(format!(
                    "event {:?} {} a key",
                    event.event,
                    if declared.keyed { "needs" } else { "has no" }
                ));
            }
            if event.text.trim().is_empty() || event.text.chars().count() > MAX_TEXT_CHARS {
                return Err(format!(
                    "event {:?}: text of 1–{MAX_TEXT_CHARS} characters",
                    event.event
                ));
            }
            if event.key.as_ref().is_some_and(|key| key.len() > 256) {
                return Err(format!("event {:?}: key over 256 bytes", event.event));
            }
        }
        Report::Reading(reading) if reading.chars().count() > MAX_READING_CHARS => {
            return Err(format!("reading over {MAX_READING_CHARS} characters"));
        }
        Report::Cursor(cursor) if cursor.to_string().len() > MAX_CURSOR_BYTES => {
            return Err(format!("cursor over {MAX_CURSOR_BYTES} bytes"));
        }
        _ => {}
    }
    Ok(parsed)
}

fn excerpt(line: &str) -> String {
    let mut excerpt: String = line.chars().take(120).collect();
    if excerpt.len() < line.len() {
        excerpt.push('…');
    }
    excerpt
}
