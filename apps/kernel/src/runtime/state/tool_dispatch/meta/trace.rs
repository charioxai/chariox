use super::session_tools::{meta_agent_ref_json, meta_owned_agent_ref_json};
use super::*;

const META_TRACE_WAIT_DEFAULT_MS: u64 = 30_000;
const META_TRACE_WAIT_MAX_MS: u64 = 60_000;

impl KernelRuntimeState {
    pub(super) fn meta_subscribe_trace(
        &self,
        session: &crate::session::RuntimeSession,
        metaagent: &crate::agent::AgentInstance,
        args: MetaSubscribeTraceArgs,
    ) -> Result<RuntimeToolResult, DaemonError> {
        let Some(mode) =
            crate::runtime::metaagent_trace::MetaagentTraceMode::parse(args.mode.as_deref())
        else {
            return Ok(RuntimeToolResult {
                ok: false,
                payload: serde_json::json!({
                    "error": "trace mode must be `compact` or `verbose`",
                    "mode": args.mode,
                }),
            });
        };
        let target = match self.meta_owned_regular_agent(session.id(), metaagent, &args.agent_ref) {
            Ok(agent) => agent,
            Err(error) => {
                return Ok(RuntimeToolResult {
                    ok: false,
                    payload: serde_json::json!({ "error": error.to_string() }),
                });
            }
        };
        let subscription = self.owned.metaagent_trace_subscriptions.subscribe(
            session.id(),
            metaagent.id(),
            target.id(),
            mode,
        );
        Ok(RuntimeToolResult {
            ok: true,
            payload: serde_json::json!({
                "subscription": subscription,
                "agent": meta_agent_ref_json(&target),
                "message": "subscribed to live worker trace; prompt the worker after subscribing, then call wait_trace for normal supervision or poll_trace for a nonblocking snapshot",
            }),
        })
    }

    pub(super) async fn meta_poll_trace(
        &self,
        session: &crate::session::RuntimeSession,
        metaagent: &crate::agent::AgentInstance,
        args: MetaPollTraceArgs,
        wait: bool,
    ) -> Result<RuntimeToolResult, DaemonError> {
        let subscription = if let Some(subscription_id) = args.subscription_id.as_deref() {
            self.owned
                .metaagent_trace_subscriptions
                .get_for_metaagent(metaagent.id(), subscription_id)
        } else if let Some(agent_ref) = args.agent_ref.as_deref() {
            let target = match self.meta_owned_regular_agent(session.id(), metaagent, agent_ref) {
                Ok(agent) => agent,
                Err(error) => {
                    return Ok(RuntimeToolResult {
                        ok: false,
                        payload: serde_json::json!({ "error": error.to_string() }),
                    });
                }
            };
            self.owned.metaagent_trace_subscriptions.get_for_target(
                metaagent.id(),
                session.id(),
                target.id(),
            )
        } else {
            return Ok(RuntimeToolResult {
                ok: false,
                payload: serde_json::json!({
                    "error": "poll_trace requires subscription_id or agent_ref",
                }),
            });
        };
        let Some(subscription) = subscription else {
            return Ok(RuntimeToolResult {
                ok: false,
                payload: serde_json::json!({
                    "error": "no live trace subscription matched; call subscribe_trace before prompting the worker",
                }),
            });
        };
        if subscription.session_id != session.id() {
            return Ok(RuntimeToolResult {
                ok: false,
                payload: serde_json::json!({
                    "error": "trace subscription belongs to a different session",
                    "subscription_id": subscription.subscription_id,
                }),
            });
        }
        let Some(mode) =
            crate::runtime::metaagent_trace::MetaagentTraceMode::parse(args.mode.as_deref())
        else {
            return Ok(RuntimeToolResult {
                ok: false,
                payload: serde_json::json!({
                    "error": "trace mode must be `compact` or `verbose`",
                    "mode": args.mode,
                }),
            });
        };
        let mode = if args.mode.is_some() {
            mode
        } else {
            subscription.mode
        };
        let limit = args.limit.unwrap_or(50).clamp(1, 100);
        let until = MetaTraceWaitUntil::parse(args.until.as_deref());
        let wait_ms = if wait {
            args.wait_ms
                .unwrap_or(META_TRACE_WAIT_DEFAULT_MS)
                .clamp(1, META_TRACE_WAIT_MAX_MS)
        } else {
            0
        };
        let started_at = std::time::Instant::now();
        let mut items = Vec::new();
        let mut drained_count = 0usize;
        let mut suppressed_count = 0usize;
        let mut matched = false;
        loop {
            let batch = self.meta_drain_trace_batch(session.id(), &subscription, mode, limit);
            drained_count += batch.drained_count;
            suppressed_count += batch.suppressed_count;
            matched = matched || batch.matches_until(until);
            extend_meta_trace_items(&mut items, batch.items, limit);
            if !wait || matched || started_at.elapsed().as_millis() >= wait_ms as u128 {
                break;
            }
            let Some(remaining) = meta_trace_wait_remaining(started_at, wait_ms) else {
                break;
            };
            let (observed_sequence, notify) = self
                .owned
                .metaagent_trace_subscriptions
                .watch_target_activity(session.id(), &subscription.target_agent_id);

            let batch = self.meta_drain_trace_batch(session.id(), &subscription, mode, limit);
            drained_count += batch.drained_count;
            suppressed_count += batch.suppressed_count;
            matched = matched || batch.matches_until(until);
            extend_meta_trace_items(&mut items, batch.items, limit);
            if matched || started_at.elapsed().as_millis() >= wait_ms as u128 {
                break;
            }

            let notified = notify.notified();
            tokio::pin!(notified);
            let latest_sequence = self
                .owned
                .metaagent_trace_subscriptions
                .target_activity_sequence(session.id(), &subscription.target_agent_id);
            if latest_sequence > observed_sequence {
                continue;
            }
            if tokio::time::timeout(remaining, notified).await.is_err() {
                break;
            }
        }
        let agent_activity = self.agent_activity_for_session(session);
        let worker_activity = agent_activity.get(&subscription.target_agent_id).cloned();
        let supervision = self.meta_trace_supervision_summary(
            session,
            metaagent,
            &subscription,
            mode,
            until,
            wait,
            matched,
            &items,
            worker_activity.as_ref(),
            drained_count,
            suppressed_count,
        );
        Ok(RuntimeToolResult {
            ok: true,
            payload: serde_json::json!({
                "subscription": subscription,
                "mode": mode,
                "until": until.as_str(),
                "wait_ms": wait_ms,
                "timed_out": wait && !matched,
                "matched": matched,
                "drained_count": drained_count,
                "suppressed_count": suppressed_count,
                "items": items,
                "empty": items.is_empty(),
                "worker_activity": worker_activity,
                "supervision": supervision,
            }),
        })
    }

    fn meta_trace_supervision_summary(
        &self,
        session: &crate::session::RuntimeSession,
        metaagent: &crate::agent::AgentInstance,
        subscription: &crate::runtime::metaagent_trace::MetaagentTraceSubscription,
        mode: crate::runtime::metaagent_trace::MetaagentTraceMode,
        until: MetaTraceWaitUntil,
        wait: bool,
        matched: bool,
        items: &[serde_json::Value],
        worker_activity: Option<&crate::runtime::projection::AgentRuntimeActivity>,
        drained_count: usize,
        suppressed_count: usize,
    ) -> serde_json::Value {
        let agent_activity = self.agent_activity_for_session(session);
        let active_owned_workers = self
            .meta_owned_regular_agents(session.id(), metaagent)
            .into_iter()
            .filter_map(|agent| {
                let activity = agent_activity.get(agent.id())?;
                if activity.busy {
                    Some(serde_json::json!({
                        "agent": meta_owned_agent_ref_json(&agent),
                        "activity": activity,
                    }))
                } else {
                    None
                }
            })
            .collect::<Vec<_>>();
        let target_agent = self
            .owned
            .agent_store
            .get_agent(&subscription.target_agent_id)
            .ok()
            .map(|agent| meta_owned_agent_ref_json(&agent));
        let last_meaningful_output = items.iter().rev().find_map(|item| {
            let kind = item_kind(item)?;
            if kind == "prompt_echo" {
                return None;
            }
            if !item
                .get("worker_generated")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false)
            {
                return None;
            }
            Some(serde_json::json!({
                "kind": kind,
                "title": item.get("title").cloned().unwrap_or(serde_json::Value::Null),
                "summary": item.get("summary").cloned().unwrap_or(serde_json::Value::Null),
                "excerpt": item.get("excerpt").cloned().unwrap_or(serde_json::Value::Null),
            }))
        });
        let completion_events = items
            .iter()
            .filter(|item| item_kind(item) == Some("assistant_message_completed"))
            .cloned()
            .collect::<Vec<_>>();
        let failure_events = items
            .iter()
            .filter(|item| {
                matches!(
                    item_kind(item),
                    Some("provider_error") | Some("runtime_notice")
                )
            })
            .cloned()
            .collect::<Vec<_>>();
        let worker_busy = worker_activity.is_some_and(|activity| activity.busy);
        let suggested_next_action = if !failure_events.is_empty() {
            "review the failure event, inspect the worker turn if needed, then either steer the owned worker or mark the task blocked"
        } else if matched && !completion_events.is_empty() {
            "review the completed worker output; if it proves the task goal, call complete_task"
        } else if matched {
            "review the worker output; continue supervision, prompt the owned worker, or complete if the task goal is proven"
        } else if items.is_empty() && wait {
            "no meaningful worker output arrived before the wait ended; wait again, inspect turn_overview, or yield for kernel continuation"
        } else if worker_busy {
            "the worker is still active; call wait_trace with a clear until condition or yield for kernel continuation"
        } else if items.is_empty() {
            "no buffered meaningful worker output is available yet; subscribe before prompting workers and use wait_trace for live supervision"
        } else {
            "inspect the compact trace items; use verbose mode only if the summary is insufficient"
        };
        serde_json::json!({
            "mode": mode,
            "until": until.as_str(),
            "matched": matched,
            "target_agent": target_agent,
            "active_owned_workers": active_owned_workers,
            "last_meaningful_output": last_meaningful_output,
            "completion_events": completion_events,
            "failure_events": failure_events,
            "drained_count": drained_count,
            "suppressed_count": suppressed_count,
            "message": if items.is_empty() {
                "no meaningful worker output yet"
            } else {
                "worker trace activity available"
            },
            "suggested_next_action": suggested_next_action,
        })
    }

    fn meta_drain_trace_batch(
        &self,
        session_id: &str,
        subscription: &crate::runtime::metaagent_trace::MetaagentTraceSubscription,
        mode: crate::runtime::metaagent_trace::MetaagentTraceMode,
        limit: usize,
    ) -> MetaTraceBatch {
        let records = self
            .owned
            .terminal_stream
            .drain_output_records(session_id, &subscription.recipient_attachment_id);
        let completions = self
            .owned
            .terminal_stream
            .drain_completion_records(session_id, &subscription.recipient_attachment_id);
        let notices = self
            .owned
            .terminal_stream
            .drain_notice_records(session_id, &subscription.recipient_attachment_id);
        let mut items = Vec::new();
        let mut seen = std::collections::BTreeSet::new();
        let mut drained_count = 0usize;
        let mut suppressed_count = 0usize;
        for record in records {
            drained_count += 1;
            let item = meta_trace_output_item(record, mode);
            if self.meta_trace_should_emit_item(&subscription.subscription_id, mode, &item)
                && seen.insert(meta_trace_item_key(&item))
            {
                extend_meta_trace_items(&mut items, vec![item], limit);
            } else {
                suppressed_count += 1;
            }
        }
        for completion in completions {
            drained_count += 1;
            let item = serde_json::json!({
                "kind": "assistant_message_completed",
                "provider_run_id": completion.provider_run_id,
                "agent_id": completion.agent_id,
                "message_id": completion.message_id,
                "completed_at_ms": completion.completed_at_ms,
                "worker_generated": true,
            });
            if self.meta_trace_should_emit_item(&subscription.subscription_id, mode, &item)
                && seen.insert(meta_trace_item_key(&item))
            {
                extend_meta_trace_items(&mut items, vec![item], limit);
            } else {
                suppressed_count += 1;
            }
        }
        for notice in notices {
            drained_count += 1;
            let item = serde_json::json!({
                "kind": "runtime_notice",
                "provider_run_id": notice.provider_run_id,
                "agent_id": notice.agent_id,
                "summary": truncate_single_line(&notice.message, 240),
                "text": if mode == crate::runtime::metaagent_trace::MetaagentTraceMode::Verbose {
                    Some(truncate_text(&notice.message, 8_000))
                } else {
                    None
                },
                "worker_generated": false,
            });
            if self.meta_trace_should_emit_item(&subscription.subscription_id, mode, &item)
                && seen.insert(meta_trace_item_key(&item))
            {
                extend_meta_trace_items(&mut items, vec![item], limit);
            } else {
                suppressed_count += 1;
            }
        }
        MetaTraceBatch {
            items,
            drained_count,
            suppressed_count,
        }
    }

    fn meta_trace_should_emit_item(
        &self,
        subscription_id: &str,
        mode: crate::runtime::metaagent_trace::MetaagentTraceMode,
        item: &serde_json::Value,
    ) -> bool {
        if mode == crate::runtime::metaagent_trace::MetaagentTraceMode::Verbose {
            return true;
        }
        self.owned
            .metaagent_trace_subscriptions
            .remember_compact_item_key(subscription_id, meta_trace_item_key(item))
    }

    pub(super) fn meta_unsubscribe_trace(
        &self,
        session: &crate::session::RuntimeSession,
        metaagent: &crate::agent::AgentInstance,
        args: MetaUnsubscribeTraceArgs,
    ) -> Result<RuntimeToolResult, DaemonError> {
        let removed = self
            .owned
            .metaagent_trace_subscriptions
            .unsubscribe(metaagent.id(), &args.subscription_id);
        if let Some(subscription) = removed.as_ref() {
            let _ = self
                .owned
                .terminal_stream
                .drain_output_records(session.id(), &subscription.recipient_attachment_id);
            let _ = self
                .owned
                .terminal_stream
                .drain_completion_records(session.id(), &subscription.recipient_attachment_id);
            let _ = self
                .owned
                .terminal_stream
                .drain_notice_records(session.id(), &subscription.recipient_attachment_id);
        }
        Ok(RuntimeToolResult {
            ok: true,
            payload: serde_json::json!({
                "subscription_id": args.subscription_id,
                "status": if removed.is_some() { "removed" } else { "not_found" },
            }),
        })
    }
}

fn meta_trace_output_item(
    record: crate::terminal::TerminalOutputRecord,
    mode: crate::runtime::metaagent_trace::MetaagentTraceMode,
) -> serde_json::Value {
    let text = String::from_utf8_lossy(&record.bytes).into_owned();
    let worker_generated = record.kind != crate::terminal::TerminalOutputKind::PromptEcho;
    let (title, summary) = match record.kind {
        crate::terminal::TerminalOutputKind::ProviderTool => {
            let (title, summary) = summarize_tool_trace(&text);
            (title, summary)
        }
        crate::terminal::TerminalOutputKind::ProviderOutput => {
            ("assistant".to_string(), truncate_single_line(&text, 240))
        }
        crate::terminal::TerminalOutputKind::ProviderReasoning => {
            ("thinking".to_string(), truncate_single_line(&text, 240))
        }
        crate::terminal::TerminalOutputKind::ProviderError => {
            ("error".to_string(), truncate_single_line(&text, 240))
        }
        crate::terminal::TerminalOutputKind::ProviderStatus => {
            ("status".to_string(), truncate_single_line(&text, 240))
        }
        crate::terminal::TerminalOutputKind::PromptEcho => {
            ("prompt".to_string(), truncate_single_line(&text, 240))
        }
    };
    let mut item = serde_json::json!({
        "kind": &record.kind,
        "provider_run_id": record.provider_run_id,
        "agent_id": record.agent_id,
        "merge_key": record.merge_key,
        "title": title,
        "summary": summary,
        "byte_len": record.bytes.len(),
        "worker_generated": worker_generated,
    });
    if mode == crate::runtime::metaagent_trace::MetaagentTraceMode::Verbose {
        item["text"] = serde_json::json!(truncate_text(&text, 8_000));
    } else if record.kind != crate::terminal::TerminalOutputKind::ProviderTool {
        item["excerpt"] = serde_json::json!(truncate_text(&text, 1_000));
    }
    item
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MetaTraceWaitUntil {
    Any,
    Activity,
    WorkerOutput,
    Completion,
    Error,
}

impl MetaTraceWaitUntil {
    fn parse(value: Option<&str>) -> Self {
        match value.unwrap_or("any") {
            "activity" => Self::Activity,
            "worker_output" => Self::WorkerOutput,
            "completion" => Self::Completion,
            "error" => Self::Error,
            _ => Self::Any,
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Any => "any",
            Self::Activity => "activity",
            Self::WorkerOutput => "worker_output",
            Self::Completion => "completion",
            Self::Error => "error",
        }
    }
}

struct MetaTraceBatch {
    items: Vec<serde_json::Value>,
    drained_count: usize,
    suppressed_count: usize,
}

impl MetaTraceBatch {
    fn matches_until(&self, until: MetaTraceWaitUntil) -> bool {
        self.items.iter().any(|item| match until {
            MetaTraceWaitUntil::Any => true,
            MetaTraceWaitUntil::Activity => item
                .get("worker_generated")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false),
            MetaTraceWaitUntil::WorkerOutput => item_kind(item) == Some("provider_output"),
            MetaTraceWaitUntil::Completion => {
                item_kind(item) == Some("assistant_message_completed")
                    || item_kind(item) == Some("provider_output")
            }
            MetaTraceWaitUntil::Error => {
                item_kind(item) == Some("runtime_notice")
                    || item_kind(item) == Some("provider_error")
            }
        })
    }
}

fn meta_trace_wait_remaining(
    started_at: std::time::Instant,
    wait_ms: u64,
) -> Option<std::time::Duration> {
    let elapsed_ms = u64::try_from(started_at.elapsed().as_millis()).unwrap_or(u64::MAX);
    if elapsed_ms >= wait_ms {
        return None;
    }
    Some(std::time::Duration::from_millis(wait_ms - elapsed_ms))
}

fn item_kind(item: &serde_json::Value) -> Option<&str> {
    item.get("kind").and_then(serde_json::Value::as_str)
}

fn meta_trace_item_key(item: &serde_json::Value) -> String {
    serde_json::json!({
        "kind": item.get("kind"),
        "provider_run_id": item.get("provider_run_id"),
        "agent_id": item.get("agent_id"),
        "merge_key": item.get("merge_key"),
        "title": item.get("title"),
        "summary": item.get("summary"),
    })
    .to_string()
}

fn extend_meta_trace_items(
    items: &mut Vec<serde_json::Value>,
    new_items: Vec<serde_json::Value>,
    limit: usize,
) {
    for item in new_items {
        if items.len() >= limit {
            break;
        }
        items.push(item);
    }
}

fn summarize_tool_trace(text: &str) -> (String, String) {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(text) else {
        return ("tool".to_string(), truncate_single_line(text, 240));
    };
    let tool = value
        .get("tool")
        .and_then(serde_json::Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .unwrap_or("tool");
    let status = value
        .get("status")
        .and_then(serde_json::Value::as_str)
        .filter(|value| !value.trim().is_empty());
    let title = match status {
        Some(status) => format!("{tool} · {}", status.to_ascii_uppercase()),
        None => tool.to_string(),
    };
    let summary = value
        .pointer("/input/command")
        .and_then(serde_json::Value::as_str)
        .map(|command| format!("$ {command}"))
        .or_else(|| {
            value
                .get("description")
                .or_else(|| value.get("title"))
                .or_else(|| value.get("output"))
                .and_then(serde_json::Value::as_str)
                .map(str::to_string)
        })
        .unwrap_or_else(|| truncate_single_line(text, 240));
    (title, truncate_single_line(&summary, 240))
}

fn truncate_single_line(text: &str, max_chars: usize) -> String {
    let normalized = text.replace("\r\n", "\n").replace('\r', "\n");
    let line = normalized
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or("");
    truncate_text(line, max_chars)
}

fn truncate_text(text: &str, max_chars: usize) -> String {
    let mut chars = text.chars();
    let truncated = chars.by_ref().take(max_chars).collect::<String>();
    if chars.next().is_some() {
        format!("{truncated}...")
    } else {
        truncated
    }
}

pub(super) fn suggest_metaagent_event_kinds(input: &str) -> Vec<&'static str> {
    let normalized_input = normalize_event_kind_for_suggestion(input);
    let mut scored = META_EVENT_KINDS
        .iter()
        .filter_map(|kind| {
            let normalized_kind = normalize_event_kind_for_suggestion(kind);
            let score = event_kind_suggestion_score(&normalized_input, &normalized_kind);
            (score > 0).then_some((*kind, score))
        })
        .collect::<Vec<_>>();
    scored.sort_by(|(left_kind, left_score), (right_kind, right_score)| {
        right_score
            .cmp(left_score)
            .then_with(|| left_kind.cmp(right_kind))
    });
    scored.into_iter().take(3).map(|(kind, _)| kind).collect()
}

fn normalize_event_kind_for_suggestion(value: &str) -> String {
    value
        .to_ascii_lowercase()
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric())
        .collect()
}

fn event_kind_suggestion_score(input: &str, candidate: &str) -> u8 {
    if input.is_empty() {
        return 0;
    }
    if input == candidate {
        return 100;
    }
    if candidate.contains(input) || input.contains(candidate) {
        return 80;
    }
    let input_tokens = event_kind_tokens(input);
    let candidate_tokens = event_kind_tokens(candidate);
    let overlap = input_tokens
        .iter()
        .filter(|token| candidate_tokens.iter().any(|candidate| candidate == *token))
        .count();
    if overlap > 0 {
        return 40 + overlap.min(4) as u8 * 10;
    }
    let common_prefix = input
        .chars()
        .zip(candidate.chars())
        .take_while(|(left, right)| left == right)
        .count();
    (common_prefix >= 5).then_some(20).unwrap_or(0)
}

fn event_kind_tokens(normalized: &str) -> Vec<&str> {
    const TOKENS: &[&str] = &[
        "agent",
        "turn",
        "completed",
        "failed",
        "runtime",
        "interaction",
        "workflow",
        "run",
        "started",
        "updated",
        "cancelled",
        "output",
        "final",
        "intermediate",
    ];
    TOKENS
        .iter()
        .copied()
        .filter(|token| normalized.contains(token))
        .collect()
}
