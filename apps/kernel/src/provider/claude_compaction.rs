//! Claude's own markers for a context compaction, read the same way from a stream-json
//! event and from a transcript entry, so both modes classify one way, and the typed state
//! the kernel sends clients for it.

use std::io::{BufReader, Read, Seek, SeekFrom};
use std::path::Path;

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// What a client draws for one compaction: the text of a `context_compacted` entry, replaced in
/// place as the compaction runs, finishes or fails.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub(crate) enum ContextCompaction {
    /// Claude is writing the summary; `started_at_ms` lets a client show the elapsed time.
    Running { started_at_ms: u64 },
    /// The summary Claude wrote; empty when Claude marked the compaction without one.
    Done { summary: String },
    /// Claude could not compact; `reason` is what it said.
    Failed { reason: String },
}

impl ContextCompaction {
    pub(crate) fn to_text(&self) -> String {
        serde_json::to_string(self).expect("a compaction state is plain data")
    }
}

/// What Claude wrote about one compaction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ClaudeCompaction {
    /// `system/status` "compacting": the summary is being written.
    Started,
    /// `system/status` with `compact_result: "failed"`.
    Failed { reason: String },
    /// `system/compact_boundary`: Claude says it compacted, without the summary.
    Boundary { row_id: String },
    /// The user entry flagged `isCompactSummary`; `summary` is empty when Claude wrote none.
    Summary { row_id: String, summary: String },
}

/// Read by Claude's own markers, never by the summary's wording.
pub(crate) fn claude_compaction(value: &Value) -> Option<ClaudeCompaction> {
    match value.get("type").and_then(Value::as_str)? {
        "system" => match value.get("subtype").and_then(Value::as_str)? {
            "compact_boundary" => {
                let anchor = value
                    .get("compactMetadata")
                    .or_else(|| value.get("compact_metadata"))
                    .and_then(|metadata| {
                        metadata
                            .get("preservedSegment")
                            .or_else(|| metadata.get("preserved_segment"))
                    })
                    .and_then(|segment| {
                        segment
                            .get("anchorUuid")
                            .or_else(|| segment.get("anchor_uuid"))
                    })
                    .and_then(Value::as_str);
                Some(ClaudeCompaction::Boundary {
                    row_id: anchor.or_else(|| entry_id(value))?.to_string(),
                })
            }
            "status" => match (
                value.get("status").and_then(Value::as_str),
                value.get("compact_result").and_then(Value::as_str),
            ) {
                (Some("compacting"), _) => Some(ClaudeCompaction::Started),
                (_, Some("failed")) => Some(ClaudeCompaction::Failed {
                    reason: value
                        .get("compact_error")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string(),
                }),
                _ => None,
            },
            _ => None,
        },
        "user" if value.get("isCompactSummary").and_then(Value::as_bool) == Some(true) => {
            Some(ClaudeCompaction::Summary {
                row_id: entry_id(value)?.to_string(),
                summary: summary_text(value.get("message")?.get("content")?),
            })
        }
        _ => None,
    }
}

/// The summary Claude wrote for the compaction `row_id` names, from the session's transcript file.
/// Claude writes the summary entry before it reports the boundary, so it is in the file's tail.
pub(crate) fn read_summary_from_transcript(
    config_dir: &Path,
    session_id: &str,
    row_id: &str,
) -> Option<String> {
    const TAIL_BYTES: u64 = 8 * 1024 * 1024;
    let file_name = format!("{session_id}.jsonl");
    let transcript = std::fs::read_dir(config_dir.join("projects"))
        .ok()?
        .filter_map(Result::ok)
        .map(|project| project.path().join(&file_name))
        .find(|path| path.is_file())?;
    let mut file = std::fs::File::open(transcript).ok()?;
    let length = file.metadata().ok()?.len();
    file.seek(SeekFrom::Start(length.saturating_sub(TAIL_BYTES)))
        .ok()?;
    let mut tail = String::new();
    BufReader::new(file).read_to_string(&mut tail).ok()?;
    tail.lines().rev().find_map(|line| {
        match claude_compaction(&serde_json::from_str::<Value>(line).ok()?)? {
            ClaudeCompaction::Summary {
                row_id: found,
                summary,
            } if found == row_id => Some(summary),
            _ => None,
        }
    })
}

fn entry_id(value: &Value) -> Option<&str> {
    value
        .get("uuid")
        .or_else(|| value.get("timestamp"))
        .and_then(Value::as_str)
}

fn summary_text(content: &Value) -> String {
    let text = match content {
        Value::String(text) => text.clone(),
        Value::Array(blocks) => blocks
            .iter()
            .filter_map(|block| block.get("text").and_then(Value::as_str))
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    };
    text.trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn the_summary_entry_is_marked_by_its_flag_not_its_wording() {
        let flagged = json!({"type":"user","isCompactSummary":true,"uuid":"s1","message":{"content":"Anything at all"}});
        assert_eq!(
            claude_compaction(&flagged),
            Some(ClaudeCompaction::Summary {
                row_id: "s1".into(),
                summary: "Anything at all".into()
            })
        );
        let imitation = json!({"type":"user","uuid":"u1","message":{"content":"This session is being continued from a previous conversation that ran out of context. The summary below covers the earlier portion of the conversation.\n\nSummary:\nx"}});
        assert_eq!(claude_compaction(&imitation), None);
    }

    #[test]
    fn the_boundary_names_the_summary_it_precedes() {
        let transcript = json!({"type":"system","subtype":"compact_boundary","uuid":"b1","compactMetadata":{"trigger":"auto","preservedSegment":{"anchorUuid":"s1"}}});
        assert_eq!(
            claude_compaction(&transcript),
            Some(ClaudeCompaction::Boundary {
                row_id: "s1".into()
            })
        );
        let stream = json!({"type":"system","subtype":"compact_boundary","uuid":"b2","compact_metadata":{"trigger":"auto","pre_tokens":9}});
        assert_eq!(
            claude_compaction(&stream),
            Some(ClaudeCompaction::Boundary {
                row_id: "b2".into()
            })
        );
    }

    #[test]
    fn a_summary_entry_without_text_has_an_empty_summary() {
        let cut =
            json!({"type":"user","isCompactSummary":true,"uuid":"s2","message":{"content":[]}});
        assert_eq!(
            claude_compaction(&cut),
            Some(ClaudeCompaction::Summary {
                row_id: "s2".into(),
                summary: String::new()
            })
        );
    }

    #[test]
    fn the_status_event_marks_the_start_and_the_failure() {
        let started = json!({"type":"system","subtype":"status","status":"compacting","compact_output_chars":0});
        assert_eq!(claude_compaction(&started), Some(ClaudeCompaction::Started));
        let failed = json!({"type":"system","subtype":"status","status":null,"compact_result":"failed","compact_error":"prompt too long"});
        assert_eq!(
            claude_compaction(&failed),
            Some(ClaudeCompaction::Failed {
                reason: "prompt too long".into()
            })
        );
        let requesting = json!({"type":"system","subtype":"status","status":"requesting"});
        assert_eq!(claude_compaction(&requesting), None);
        let succeeded =
            json!({"type":"system","subtype":"status","status":null,"compact_result":"success"});
        assert_eq!(claude_compaction(&succeeded), None);
    }

    #[test]
    fn the_summary_is_read_from_the_transcript_by_the_boundary_s_row() {
        let temp = crate::test_temp_dir::TestTempDir::new("claude-compaction-summary-read");
        let project = temp.path().join("projects").join("-repo");
        std::fs::create_dir_all(&project).unwrap();
        let lines = [
            json!({"type":"user","isCompactSummary":true,"uuid":"old","message":{"content":"first summary"}}),
            json!({"type":"user","uuid":"u","message":{"content":"a question"}}),
            json!({"type":"user","isCompactSummary":true,"uuid":"s9","message":{"content":"second summary"}}),
        ]
        .map(|line| line.to_string())
        .join("\n");
        std::fs::write(project.join("session-1.jsonl"), lines + "\n").unwrap();

        assert_eq!(
            read_summary_from_transcript(temp.path(), "session-1", "s9").as_deref(),
            Some("second summary")
        );
        assert_eq!(
            read_summary_from_transcript(temp.path(), "session-1", "none"),
            None
        );
        assert_eq!(
            read_summary_from_transcript(temp.path(), "other", "s9"),
            None
        );
    }

    #[test]
    fn a_state_is_the_text_a_client_parses() {
        let text = ContextCompaction::Done {
            summary: "x".into(),
        }
        .to_text();
        assert_eq!(text, r#"{"state":"done","summary":"x"}"#);
        assert_eq!(
            serde_json::from_str::<ContextCompaction>(&text).unwrap(),
            ContextCompaction::Done {
                summary: "x".into()
            }
        );
    }
}
