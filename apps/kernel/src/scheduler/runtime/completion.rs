//! Workflow completion snapshot projection from provider output and artifacts.

use std::time::UNIX_EPOCH;

use serde::Deserialize;
use serde_json::Value;

use crate::app::{attachment_artifact_roots, DaemonApp};
use crate::history::{SessionHistoryEntry, SessionHistoryEntryKind};
use crate::session::{
    RuntimeSession, WorkflowArtifactRef, WorkflowCompletionSnapshot, WorkflowOutputPayload,
};

const WORKFLOW_COMPLETION_SUMMARY_LIMIT: usize = 160;

#[derive(Debug, Clone, Deserialize)]
struct WorkflowStructuredOutputEnvelope {
    summary: Option<String>,
    output: Option<WorkflowStructuredOutputValue>,
    #[serde(default)]
    workflow_handoffs: Vec<Value>,
}

impl WorkflowStructuredOutputEnvelope {
    fn output_message(&self) -> Option<String> {
        if !self.workflow_handoffs.is_empty() {
            return Some(
                serde_json::json!({ "workflow_handoffs": self.workflow_handoffs }).to_string(),
            );
        }
        self.output
            .clone()
            .and_then(WorkflowStructuredOutputValue::into_output_message)
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
enum WorkflowStructuredOutputValue {
    Text(String),
    Object { message: Value },
}

impl WorkflowStructuredOutputValue {
    fn into_output_message(self) -> Option<String> {
        match self {
            WorkflowStructuredOutputValue::Text(message) => {
                let trimmed = message.trim().to_string();
                (!trimmed.is_empty()).then_some(trimmed)
            }
            WorkflowStructuredOutputValue::Object { message } => match message {
                Value::String(message) => {
                    let trimmed = message.trim().to_string();
                    (!trimmed.is_empty()).then_some(trimmed)
                }
                other => Some(other.to_string()),
            },
        }
    }
}

pub(super) fn build_workflow_completion_snapshot(
    app: &DaemonApp,
    session_id: &str,
    workflow_run_id: &str,
    workflow_node_run_id: &str,
    provider_run_id: Option<&str>,
) -> Option<WorkflowCompletionSnapshot> {
    let provider_run_id = provider_run_id
        .map(str::trim)
        .filter(|value| !value.is_empty())?
        .to_string();
    let session = match app.sessions().get_session(session_id) {
        Ok(session) => session,
        Err(error) => {
            crate::logging::warn_with_fields(
                "daemon.workflow",
                "failed to load session while building workflow completion snapshot",
                serde_json::json!({
                    "session_id": session_id,
                    "workflow_run_id": workflow_run_id,
                    "workflow_node_run_id": workflow_node_run_id,
                    "provider_run_id": provider_run_id,
                    "error": error.to_string(),
                }),
            );
            return None;
        }
    };
    let Some(workflow_run) = session.workflow_run(workflow_run_id) else {
        crate::logging::warn_with_fields(
            "daemon.workflow",
            "workflow run disappeared before completion snapshot could be built",
            serde_json::json!({
                "session_id": session_id,
                "workflow_run_id": workflow_run_id,
                "workflow_node_run_id": workflow_node_run_id,
                "provider_run_id": provider_run_id,
            }),
        );
        return None;
    };
    let Some(_node_run) = workflow_run
        .node_runs()
        .iter()
        .find(|node_run| node_run.id() == workflow_node_run_id)
    else {
        crate::logging::warn_with_fields(
            "daemon.workflow",
            "workflow node run disappeared before completion snapshot could be built",
            serde_json::json!({
                "session_id": session_id,
                "workflow_run_id": workflow_run_id,
                "workflow_node_run_id": workflow_node_run_id,
                "provider_run_id": provider_run_id,
            }),
        );
        return None;
    };
    let history = match crate::app::KernelSessionReadService::new(app).session_history(session_id) {
        Ok(history) => history,
        Err(error) => {
            crate::logging::warn_with_fields(
                "daemon.workflow",
                "failed to load session history for workflow completion snapshot",
                serde_json::json!({
                    "session_id": session_id,
                    "workflow_run_id": workflow_run_id,
                    "workflow_node_run_id": workflow_node_run_id,
                    "provider_run_id": provider_run_id,
                    "error": error.to_string(),
                }),
            );
            return None;
        }
    };
    build_workflow_completion_snapshot_from_history(
        &session,
        history,
        session_id,
        workflow_run_id,
        workflow_node_run_id,
        &provider_run_id,
    )
}

pub(crate) fn build_workflow_completion_snapshot_from_history(
    session: &RuntimeSession,
    history: Vec<SessionHistoryEntry>,
    session_id: &str,
    workflow_run_id: &str,
    workflow_node_run_id: &str,
    provider_run_id: &str,
) -> Option<WorkflowCompletionSnapshot> {
    let Some(workflow_run) = session.workflow_run(workflow_run_id) else {
        crate::logging::warn_with_fields(
            "daemon.workflow",
            "workflow run disappeared before completion snapshot could be built",
            serde_json::json!({
                "session_id": session_id,
                "workflow_run_id": workflow_run_id,
                "workflow_node_run_id": workflow_node_run_id,
                "provider_run_id": provider_run_id,
            }),
        );
        return None;
    };
    let Some(node_run) = workflow_run
        .node_runs()
        .iter()
        .find(|node_run| node_run.id() == workflow_node_run_id)
    else {
        crate::logging::warn_with_fields(
            "daemon.workflow",
            "workflow node run disappeared before completion snapshot could be built",
            serde_json::json!({
                "session_id": session_id,
                "workflow_run_id": workflow_run_id,
                "workflow_node_run_id": workflow_node_run_id,
                "provider_run_id": provider_run_id,
            }),
        );
        return None;
    };
    let started_at_ms = node_run
        .started_at_ms()
        .unwrap_or_else(|| node_run.created_at_ms());
    let output_started_at_ms = node_run
        .turn_envelope()
        .and_then(|envelope| {
            envelope
                .runtime_tool_calls()
                .iter()
                .rev()
                .find(|call| {
                    call.ok()
                        && call.tool_name()
                            == crate::transport::runtime_tools::ACK_WORKFLOW_TURN_TOOL
                })
                .map(|call| call.timestamp_ms())
        })
        .unwrap_or(started_at_ms);
    let provider_output_entries = history
        .into_iter()
        .filter(|entry| {
            entry.provider_run_id.as_deref() == Some(provider_run_id)
                && entry.timestamp_ms >= output_started_at_ms
                && entry.kind == SessionHistoryEntryKind::ProviderOutput
        })
        .map(|entry| entry.text)
        .collect::<Vec<_>>();
    let provider_output = provider_output_entries.join("");
    let structured_output = parse_workflow_structured_output(&provider_output);
    if structured_output.is_none() {
        if let Some(snapshot) =
            workflow_completion_snapshot_from_validated_tool_output(node_run, &provider_output)
        {
            return Some(snapshot);
        }
        crate::logging::warn_with_fields(
            "daemon.workflow",
            "ignoring workflow turn completion without structured output block",
            serde_json::json!({
                "session_id": session_id,
                "workflow_run_id": workflow_run_id,
                "workflow_node_run_id": workflow_node_run_id,
                "provider_run_id": provider_run_id,
                "output_started_at_ms": output_started_at_ms,
                "provider_output_entry_count": provider_output_entries.len(),
                "provider_output_char_count": provider_output.chars().count(),
            }),
        );
        return None;
    }
    let summary = structured_output
        .as_ref()
        .and_then(|value| value.summary.as_deref())
        .map(workflow_completion_summary)
        .unwrap_or_else(|| workflow_completion_summary(&provider_output));
    let artifacts = collect_workflow_artifact_refs(session_id, workflow_run_id, started_at_ms);
    let output_message = structured_output
        .as_ref()
        .and_then(WorkflowStructuredOutputEnvelope::output_message);
    let output = match (output_message, artifacts) {
        (Some(message), artifacts) => Some(WorkflowOutputPayload::new(message, artifacts)),
        (None, artifacts) if !artifacts.is_empty() => {
            Some(WorkflowOutputPayload::new("artifacts attached", artifacts))
        }
        _ => None,
    };
    if summary == "completed" && output.is_none() {
        return None;
    }

    Some(WorkflowCompletionSnapshot::new(summary, output))
}

fn workflow_completion_snapshot_from_validated_tool_output(
    node_run: &crate::session::WorkflowNodeRun,
    provider_output: &str,
) -> Option<WorkflowCompletionSnapshot> {
    let call = node_run
        .turn_envelope()?
        .runtime_tool_calls()
        .iter()
        .rev()
        .find(|call| {
            call.ok()
                && call.tool_name()
                    == crate::transport::runtime_tools::VALIDATE_WORKFLOW_HANDOFF_TOOL
                && call
                    .result_json()
                    .and_then(|result| serde_json::from_str::<serde_json::Value>(result).ok())
                    .and_then(|value| value.get("valid").and_then(|valid| valid.as_bool()))
                    == Some(true)
        })?;
    let args =
        serde_json::from_str::<crate::transport::runtime_tools::ValidateWorkflowHandoffArgs>(
            call.arguments_json(),
        )
        .ok()?;
    let summary = workflow_completion_summary(provider_output);
    Some(WorkflowCompletionSnapshot::new(
        summary,
        Some(WorkflowOutputPayload::new(args.handoff_json, Vec::new())),
    ))
}

fn collect_workflow_artifact_refs(
    session_id: &str,
    workflow_run_id: &str,
    started_at_ms: u64,
) -> Vec<WorkflowArtifactRef> {
    let attachment_id = super::workflow_prompt_source_attachment_id(workflow_run_id);
    let mut artifacts = Vec::new();
    for root in attachment_artifact_roots(session_id, &attachment_id) {
        let kind = root
            .parent()
            .and_then(|parent| parent.file_name())
            .and_then(|value| value.to_str())
            .unwrap_or("artifact")
            .trim_end_matches('s')
            .to_string();
        collect_workflow_artifacts_from_dir(&root, &kind, started_at_ms, &mut artifacts);
    }
    artifacts.sort_by(|left, right| left.id().cmp(right.id()));
    artifacts
}

fn workflow_completion_summary(source: &str) -> String {
    if source.trim().is_empty() {
        return "completed".to_string();
    }
    let normalized = source
        .split_whitespace()
        .filter(|segment| !segment.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    if normalized.is_empty() {
        return "completed".to_string();
    }
    if normalized.chars().count() <= WORKFLOW_COMPLETION_SUMMARY_LIMIT {
        return normalized;
    }

    let truncated = normalized
        .chars()
        .take(WORKFLOW_COMPLETION_SUMMARY_LIMIT)
        .collect::<String>();
    format!("{truncated}...")
}

fn parse_workflow_structured_output(text: &str) -> Option<WorkflowStructuredOutputEnvelope> {
    let mut cursor = 0usize;
    let mut parsed = None;
    while let Some(start) = text[cursor..].find("```json") {
        let block_start = cursor + start + "```json".len();
        let candidate = text[block_start..].trim_start();
        let mut values = serde_json::Deserializer::from_str(candidate)
            .into_iter::<WorkflowStructuredOutputEnvelope>();
        if let Some(Ok(value)) = values.next() {
            parsed = Some(value);
        }
        cursor = block_start;
    }
    parsed.or_else(|| serde_json::from_str::<WorkflowStructuredOutputEnvelope>(text.trim()).ok())
}

fn collect_workflow_artifacts_from_dir(
    root: &std::path::Path,
    kind: &str,
    started_at_ms: u64,
    artifacts: &mut Vec<WorkflowArtifactRef>,
) {
    let entries = match std::fs::read_dir(root) {
        Ok(entries) => entries,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_workflow_artifacts_from_dir(&path, kind, started_at_ms, artifacts);
            continue;
        }
        let Ok(metadata) = entry.metadata() else {
            continue;
        };
        let Ok(modified) = metadata.modified() else {
            continue;
        };
        let modified_at_ms = modified
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_millis() as u64)
            .unwrap_or(0);
        if modified_at_ms < started_at_ms {
            continue;
        }
        let display_name = path
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or("artifact")
            .to_string();
        let path_string = path.to_string_lossy().into_owned();
        artifacts.push(WorkflowArtifactRef::new(
            format!("{kind}:{display_name}"),
            kind.to_string(),
            path_string,
            display_name,
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::parse_workflow_structured_output;

    #[test]
    fn workflow_structured_output_preserves_top_level_routing_over_plain_output() {
        let parsed = parse_workflow_structured_output(
            r#"
```json
{"summary":"classified","workflow_handoffs":[{"edge_id":"code-edge","output":{"message":{"task":"fix routing"}}}],"output":{"message":"plain classifier note"}}
```
"#,
        )
        .expect("structured output should parse");

        let output: serde_json::Value = serde_json::from_str(
            &parsed
                .output_message()
                .expect("top-level routing should become the output message"),
        )
        .expect("routing output should stay valid JSON");
        assert_eq!(
            output["workflow_handoffs"].as_array().map(Vec::len),
            Some(1)
        );
        assert_eq!(
            output["workflow_handoffs"][0]["output"]["message"]["task"],
            "fix routing"
        );
    }

    #[test]
    fn workflow_structured_output_accepts_json_message_values() {
        let parsed = parse_workflow_structured_output(
            r#"
```json
{"summary":"fixed","output":{"message":{"ok":true,"source":"mailbox-fixed"}}}
```
"#,
        )
        .expect("structured output should parse");

        let output = parsed
            .output
            .expect("structured output should contain output")
            .into_output_message()
            .expect("message should serialize");
        assert_eq!(output, r#"{"ok":true,"source":"mailbox-fixed"}"#);
    }

    #[test]
    fn workflow_structured_output_accepts_trailing_unclosed_fence() {
        let parsed = parse_workflow_structured_output(
            r#"
The provider forgot to close the fence.
```json
{"summary":"fixed","output":{"message":"{\"ok\":true}"}}
"#,
        )
        .expect("trailing structured output should parse");

        let output = parsed
            .output
            .expect("structured output should contain output")
            .into_output_message()
            .expect("message should serialize");
        assert_eq!(output, r#"{"ok":true}"#);
    }

    #[test]
    fn workflow_structured_output_accepts_bare_json_envelope() {
        let parsed = parse_workflow_structured_output(
            r#"
{"summary":"fixed","output":{"message":"{\"ok\":true}"}}
"#,
        )
        .expect("bare structured output should parse");

        let output = parsed
            .output
            .expect("structured output should contain output")
            .into_output_message()
            .expect("message should serialize");
        assert_eq!(output, r#"{"ok":true}"#);
    }

    #[test]
    fn workflow_structured_output_accepts_code_fences_inside_message() {
        let parsed = parse_workflow_structured_output(
            r####"
```json
{"summary":"test proposed","output":{"message":"Proposed test:\n```ts\nassert.equal(active, null)\n```\nThis covers the terminal state."}}
```
"####,
        )
        .expect("structured output with an embedded code fence should parse");

        let output = parsed
            .output
            .expect("structured output should contain output")
            .into_output_message()
            .expect("message should serialize");
        assert!(output.contains("```ts"));
        assert!(output.contains("terminal state"));
    }
}
