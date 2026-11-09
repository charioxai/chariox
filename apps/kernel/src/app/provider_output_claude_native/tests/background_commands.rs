//! MP-08 / MP-10: the background-command lifecycle the native Claude bridge
//! reads from Claude's own transcript, replayed from a real Claude Code
//! 2.1.296 session (lines kept verbatim, paths shortened): two background
//! launches and a `Read` poll; a newer turn with a foreground call; the first
//! command's task notification; `TaskStop` of the second and a third launch; a
//! failing fourth launch and its notification; the session stopping the third
//! on exit.

use serde_json::Value;

use super::super::transcript::{drain_claude_transcript_file_at, ClaudeTranscriptCursor};
use crate::terminal::TerminalOutputKind;

const RECORDED: &str = include_str!("fixtures/claude-2.1.296-background-commands.jsonl");

const FIRST: &str = "toolu_01Lo1vSg3ae11aKWY7jR6GXV";
const SECOND: &str = "toolu_01759ZTJf9mHLxVchA5qkCKe";
const THIRD: &str = "toolu_015WbSZim7JjZdS7Dgvo1AbM";
const FAILING: &str = "toolu_01KjeWWvN4yS8SHRJMNwhQGf";
const POLL: &str = "toolu_0111h5aw56YsbAwZqZe3ptMN";
const FOREGROUND: &str = "toolu_01CNXLsrCVACYSAFzusAe1Dm";
const STOP: &str = "toolu_01YcFX8eACMwxX2GUeNaqNzk";

/// Recorded lines, in order, through the end of each step.
const LAUNCHED_AND_POLLED: usize = 6;
const NEWER_TURN: usize = 9;
const FIRST_NOTIFIED: usize = 11;
const SECOND_STOPPED_THIRD_LAUNCHED: usize = 15;
const FAILING_LAUNCHED: usize = 17;
const FAILING_NOTIFIED: usize = 19;
const EXITED: usize = 20;

struct RecordedSession {
    dir: std::path::PathBuf,
    transcript: String,
    cursor: ClaudeTranscriptCursor,
}

impl RecordedSession {
    fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "chariox-claude-background-{name}-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let transcript = dir.join("session.jsonl").display().to_string();
        Self {
            dir,
            transcript,
            cursor: ClaudeTranscriptCursor::default(),
        }
    }

    /// Claude has written the recorded lines up to `lines`; the kernel reads
    /// them at `now_ms`. The tool snapshots it emits.
    fn read(&mut self, lines: usize, now_ms: u64) -> Vec<Value> {
        let written = RECORDED.lines().take(lines).collect::<Vec<_>>().join("\n") + "\n";
        std::fs::write(&self.transcript, written).unwrap();
        drain_claude_transcript_file_at(&self.transcript, &mut self.cursor, None, now_ms)
            .chunks
            .into_iter()
            .filter(|chunk| chunk.kind == TerminalOutputKind::ProviderTool)
            .map(|chunk| {
                let snapshot: Value = serde_json::from_str(&chunk.text).unwrap();
                assert_eq!(
                    chunk.merge_key_suffix,
                    format!("tool:{}", snapshot["id"].as_str().unwrap())
                );
                snapshot
            })
            .collect()
    }

    /// The kernel restarts: the cursor comes back from its file.
    fn restart(&mut self) {
        let saved = serde_json::to_string(&self.cursor).unwrap();
        self.cursor = serde_json::from_str(&saved).unwrap();
    }
}

impl Drop for RecordedSession {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn of<'a>(snapshots: &'a [Value], id: &str) -> Vec<&'a Value> {
    snapshots
        .iter()
        .filter(|snapshot| snapshot["id"] == id)
        .collect()
}

fn running_background(snapshot: &Value, started_at_ms: u64) {
    assert_eq!(snapshot["started_at_ms"], started_at_ms, "{snapshot}");
    assert_eq!(snapshot["background"], true, "{snapshot}");
    assert!(snapshot["finished_at_ms"].is_null(), "{snapshot}");
    assert!(snapshot.get("finished_at_ms").is_some(), "{snapshot}");
}

fn finished_background(snapshot: &Value, started_at_ms: u64, finished_at_ms: u64) {
    assert_eq!(snapshot["started_at_ms"], started_at_ms, "{snapshot}");
    assert_eq!(snapshot["background"], true, "{snapshot}");
    assert_eq!(snapshot["finished_at_ms"], finished_at_ms, "{snapshot}");
    // The launch itself succeeded; its row keeps the launch result.
    assert_eq!(snapshot["status"], "completed", "{snapshot}");
    assert!(snapshot["output"]
        .as_str()
        .unwrap()
        .starts_with("Command running in background with ID: "));
}

fn foreground(snapshot: &Value, started_at_ms: u64) {
    assert_eq!(snapshot["started_at_ms"], started_at_ms, "{snapshot}");
    assert!(snapshot.get("background").is_none(), "{snapshot}");
    assert!(snapshot.get("finished_at_ms").is_none(), "{snapshot}");
}

#[test]
fn native_background_command_runs_past_its_turn_and_a_newer_one_until_its_notification() {
    let mut session = RecordedSession::new("notified");
    let launched = session.read(LAUNCHED_AND_POLLED, 1_000);
    let first = of(&launched, FIRST);
    assert_eq!(first.len(), 2, "{launched:?}");
    assert_eq!(first[0]["status"], "started");
    assert_eq!(first[1]["status"], "completed");
    for snapshot in first {
        running_background(snapshot, 1_000);
    }
    for snapshot in of(&launched, POLL) {
        foreground(snapshot, 1_000);
    }

    let newer = session.read(NEWER_TURN, 2_000);
    assert!(of(&newer, FIRST).is_empty(), "{newer:?}");
    for snapshot in of(&newer, FOREGROUND) {
        foreground(snapshot, 2_000);
    }

    session.restart();
    let notified = session.read(FIRST_NOTIFIED, 3_000);
    // Claude queues the notification, then starts a turn with it: one end.
    assert_eq!(notified.len(), 1, "{notified:?}");
    finished_background(&notified[0], 1_000, 3_000);
    assert_eq!(notified[0]["id"], FIRST);
}

#[test]
fn native_background_commands_end_by_stop_by_failure_and_with_the_session() {
    let mut session = RecordedSession::new("ended");
    session.read(FIRST_NOTIFIED, 1_000);

    let stopped = session.read(SECOND_STOPPED_THIRD_LAUNCHED, 2_000);
    for snapshot in of(&stopped, STOP) {
        foreground(snapshot, 2_000);
    }
    let second = of(&stopped, SECOND);
    assert_eq!(second.len(), 1, "{stopped:?}");
    finished_background(second[0], 1_000, 2_000);
    for snapshot in of(&stopped, THIRD) {
        running_background(snapshot, 2_000);
    }

    let failing = session.read(FAILING_LAUNCHED, 3_000);
    for snapshot in of(&failing, FAILING) {
        running_background(snapshot, 3_000);
    }
    let failed = session.read(FAILING_NOTIFIED, 4_000);
    assert_eq!(failed.len(), 1, "{failed:?}");
    assert_eq!(failed[0]["id"], FAILING);
    finished_background(&failed[0], 3_000, 4_000);

    // On exit the session stops what still runs and only queues the notice.
    let exited = session.read(EXITED, 5_000);
    assert_eq!(exited.len(), 1, "{exited:?}");
    assert_eq!(exited[0]["id"], THIRD);
    finished_background(&exited[0], 2_000, 5_000);
}
