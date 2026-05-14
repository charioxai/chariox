use std::collections::{HashMap, HashSet};

use serde::Deserialize;

use crate::error::DaemonError;
use crate::local::{SemanticHistoryMatch, SemanticHistorySearchUtilityInput};

#[derive(Debug, Deserialize)]
struct SemanticHistoryUtilityOutput {
    answer: String,
    matches: Vec<SemanticHistoryUtilityOutputMatch>,
}

#[derive(Debug, Deserialize)]
struct SemanticHistoryUtilityOutputMatch {
    event_id: String,
    #[serde(default)]
    chunk_index: Option<usize>,
    relevance: String,
    reason: String,
}

#[derive(Debug)]
pub(crate) struct SemanticHistoryUtilityParsedOutput {
    pub(crate) answer: String,
    pub(crate) matches: Vec<SemanticHistoryMatch>,
}

pub(crate) fn parse_semantic_history_search_utility_output(
    output: &str,
    candidates: &[SemanticHistoryMatch],
) -> Result<SemanticHistoryUtilityParsedOutput, DaemonError> {
    let json = extract_json_object(output).ok_or_else(|| DaemonError::LocalTransport {
        operation: "run semantic history search utility",
        message: "semantic history utility did not return a JSON object".to_string(),
    })?;
    let schema = semantic_history_search_utility_schema();
    crate::transport::runtime_tools::validate_json_output_schema(
        "semantic_history_search_utility_output",
        &schema,
        json,
    )
    .map_err(|warning| DaemonError::LocalTransport {
        operation: "run semantic history search utility",
        message: format!("semantic history utility output failed validation: {warning}"),
    })?;
    let output = serde_json::from_str::<SemanticHistoryUtilityOutput>(json).map_err(|error| {
        DaemonError::LocalTransport {
            operation: "run semantic history search utility",
            message: format!("semantic history utility output was not parseable: {error}"),
        }
    })?;
    let candidates_by_event = candidates
        .iter()
        .map(|candidate| (candidate.event.event_id.as_str(), candidate))
        .collect::<HashMap<_, _>>();
    let mut matches = Vec::new();
    let mut seen = HashSet::new();
    for selected in output.matches {
        if !seen.insert(selected.event_id.clone()) {
            continue;
        }
        let Some(candidate) = candidates_by_event.get(selected.event_id.as_str()) else {
            return Err(DaemonError::LocalTransport {
                operation: "run semantic history search utility",
                message: format!(
                    "semantic history utility referenced unknown event `{}`",
                    selected.event_id
                ),
            });
        };
        let mut candidate = (*candidate).clone();
        candidate.chunk_index = selected.chunk_index.or(candidate.chunk_index);
        candidate.reason = Some(format!("{}: {}", selected.relevance, selected.reason));
        matches.push(candidate);
    }
    Ok(SemanticHistoryUtilityParsedOutput {
        answer: output.answer,
        matches,
    })
}

pub(crate) fn semantic_history_search_utility_prompt(
    input: &SemanticHistorySearchUtilityInput,
    candidates: &[SemanticHistoryMatch],
) -> Result<String, DaemonError> {
    let schema = semantic_history_search_utility_schema();
    let candidate_json = serde_json::to_string_pretty(
        &candidates
            .iter()
            .map(semantic_history_candidate_for_prompt)
            .collect::<Vec<_>>(),
    )
    .map_err(|error| DaemonError::LocalTransport {
        operation: "run semantic history search utility",
        message: format!("could not encode semantic history candidates: {error}"),
    })?;
    Ok(format!(
        "You are running an Arroba history-search utility. Answer the user's question only from the supplied history candidates.\n\
Do not use external knowledge. Do not mention tool calls or runtime mechanics.\n\
Return exactly one JSON object matching this JSON Schema:\n{schema}\n\n\
User question:\n{query}\n\n\
History candidates:\n{candidates}\n\n\
Rules:\n\
- Select only event_id values present in History candidates.\n\
- If the candidates do not answer the question, say that in answer and return an empty matches array.\n\
- Keep answer concise.\n\
- Output JSON only.",
        schema = serde_json::to_string_pretty(&schema).unwrap_or_else(|_| "{}".to_string()),
        query = input.query.trim(),
        candidates = candidate_json,
    ))
}

fn semantic_history_search_utility_schema() -> serde_json::Value {
    serde_json::json!({
        "type": "object",
        "required": ["answer", "matches"],
        "additionalProperties": false,
        "properties": {
            "answer": {"type": "string", "minLength": 1, "maxLength": 2000},
            "matches": {
                "type": "array",
                "maxItems": 20,
                "items": {
                    "type": "object",
                    "required": ["event_id", "relevance", "reason"],
                    "additionalProperties": false,
                    "properties": {
                        "event_id": {"type": "string", "minLength": 1},
                        "chunk_index": {"type": ["integer", "null"], "minimum": 0},
                        "relevance": {"type": "string", "enum": ["high", "medium", "low"]},
                        "reason": {"type": "string", "minLength": 1, "maxLength": 300}
                    }
                }
            }
        }
    })
}

fn extract_json_object(output: &str) -> Option<&str> {
    let trimmed = output.trim();
    if trimmed.starts_with('{') && trimmed.ends_with('}') {
        return Some(trimmed);
    }
    let start = trimmed.find('{')?;
    let end = trimmed.rfind('}')?;
    (start < end).then_some(&trimmed[start..=end])
}

fn semantic_history_candidate_for_prompt(match_: &SemanticHistoryMatch) -> serde_json::Value {
    serde_json::json!({
        "event_id": match_.event.event_id,
        "chunk_index": match_.chunk_index,
        "score_millis": match_.score_millis,
        "timestamp_ms": match_.event.timestamp_ms,
        "session_id": match_.event.session_id,
        "agent_id": match_.event.agent_id,
        "provider": match_.event.provider,
        "model": match_.event.model,
        "kind": match_.event.kind,
        "role": match_.event.role,
        "content": match_.chunk_text.as_ref().or(match_.event.content.as_ref()),
        "metadata": match_.event.metadata,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    use crate::history::{HistoryEvent, HistoryEventKind, HistoryEventRole};

    #[test]
    fn parses_selected_history_matches_and_dedupes_events() {
        let candidates = vec![
            candidate("event-1", "first chunk"),
            candidate("event-2", "second chunk"),
        ];

        let parsed = parse_semantic_history_search_utility_output(
            r#"prefix {"answer":"Use event 2","matches":[
              {"event_id":"event-2","chunk_index":4,"relevance":"high","reason":"answers it"},
              {"event_id":"event-2","relevance":"low","reason":"duplicate"}
            ]} suffix"#,
            &candidates,
        )
        .expect("parse utility output");

        assert_eq!(parsed.answer, "Use event 2");
        assert_eq!(parsed.matches.len(), 1);
        assert_eq!(parsed.matches[0].event.event_id, "event-2");
        assert_eq!(parsed.matches[0].chunk_index, Some(4));
        assert_eq!(
            parsed.matches[0].reason.as_deref(),
            Some("high: answers it")
        );
    }

    #[test]
    fn rejects_unknown_history_event_references() {
        let error = parse_semantic_history_search_utility_output(
            r#"{"answer":"bad","matches":[{"event_id":"missing","relevance":"high","reason":"no"}]}"#,
            &[candidate("event-1", "first")],
        )
        .expect_err("unknown event should fail");

        assert!(format!("{error}").contains("unknown event `missing`"));
    }

    #[test]
    fn prompt_contains_trimmed_query_schema_and_candidate_payload() {
        let prompt = semantic_history_search_utility_prompt(
            &SemanticHistorySearchUtilityInput {
                query: "  where did deploy fail?  ".to_string(),
                session_id: None,
                agent_id: None,
                provider: None,
                model: None,
                workflow_id: None,
                machine_id: None,
                repo_root: None,
                worktree_path: None,
                kind: None,
                limit: None,
            },
            &[candidate("event-1", "deploy failed")],
        )
        .expect("build prompt");

        assert!(prompt.contains("where did deploy fail?"));
        assert!(prompt.contains("\"event_id\": \"event-1\""));
        assert!(prompt.contains("\"content\": \"deploy failed\""));
        assert!(prompt.contains("\"required\""));
    }

    fn candidate(event_id: &str, chunk_text: &str) -> SemanticHistoryMatch {
        SemanticHistoryMatch {
            event: HistoryEvent {
                event_id: event_id.to_string(),
                sequence: 1,
                timestamp_ms: 100,
                workspace_id: Some("/repo".to_string()),
                session_id: Some("session-1".to_string()),
                agent_id: Some("agent-1".to_string()),
                agent_alias: None,
                provider: Some("codex".to_string()),
                model: Some("gpt".to_string()),
                turn_id: None,
                prompt_id: None,
                provider_run_id: None,
                provider_session_id: None,
                workflow_id: None,
                workflow_run_id: None,
                workflow_node_id: None,
                machine_id: None,
                repo_root: None,
                worktree_path: None,
                kind: HistoryEventKind::ProviderOutput,
                role: Some(HistoryEventRole::Assistant),
                content: Some("full event content".to_string()),
                content_ref: None,
                metadata: BTreeMap::new(),
                candidate_agent_ids: Vec::new(),
                candidate_prompt_ids: Vec::new(),
                candidate_turn_ids: Vec::new(),
                attribution_confidence: None,
                caused_by_event_id: None,
            },
            score_millis: Some(10),
            chunk_index: Some(1),
            chunk_text: Some(chunk_text.to_string()),
            reason: None,
        }
    }
}
