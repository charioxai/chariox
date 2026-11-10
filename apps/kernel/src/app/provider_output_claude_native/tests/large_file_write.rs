//! MP-08 / MP-10: a Write of a whole page is larger than the input the bridge keeps for a pending call. Its result record
//! still says which file was written and starts with the content, so the row opened in the pane is not empty.

use serde_json::{json, Value};

use super::super::transcript::{drain_claude_transcript_file_at, ClaudeTranscriptCursor};
use crate::terminal::TerminalOutputKind;

#[test]
fn a_large_write_result_keeps_its_path_and_the_start_of_its_content() {
    let dir =
        std::env::temp_dir().join(format!("chariox-claude-large-write-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let transcript = dir.join("session.jsonl").display().to_string();
    let page = format!(
        "<!doctype html>\n<html>{}</html>\n",
        "<p>row</p>\n".repeat(2000)
    );
    let call = json!({"type":"assistant","timestamp":"2026-11-09T20:29:17.281Z","message":{"role":"assistant","content":[
        {"type":"tool_use","id":"toolu_write","name":"Write","input":{"file_path":"/w/chart.html","content":page}}]}});
    let result = json!({"type":"user","timestamp":"2026-11-09T20:29:17.339Z","message":{"role":"user","content":[
        {"tool_use_id":"toolu_write","type":"tool_result","content":"File created successfully at: /w/chart.html"}]}});
    std::fs::write(&transcript, format!("{call}\n{result}\n")).unwrap();
    let mut cursor = ClaudeTranscriptCursor::default();
    let chunks = drain_claude_transcript_file_at(&transcript, &mut cursor, None, 1_000).chunks;
    let _ = std::fs::remove_dir_all(&dir);
    let completed = chunks
        .iter()
        .filter(|chunk| chunk.kind == TerminalOutputKind::ProviderTool)
        .map(|chunk| serde_json::from_str::<Value>(&chunk.text).unwrap())
        .find(|snapshot| snapshot["status"] == "completed")
        .expect("the result record");
    assert_eq!(completed["input"]["file_path"], "/w/chart.html");
    assert!(completed["input"]["content"]
        .as_str()
        .unwrap()
        .starts_with("<!doctype html>"));
}
