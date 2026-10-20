//! MP-08 / MP-09 / MP-10 / MP-11 A03: agent timers and the wakes list.
use super::*;
use crate::durable_state::agent_lifecycle::{self as ledger, AgentTaskExecution, AgentWake};

const MAX_DELAY_MS: u64 = 7 * 86_400_000;

fn label(args: &serde_json::Value) -> Result<String, DaemonError> {
    let label = args["label"].as_str().unwrap_or_default().trim();
    if label.is_empty() || label.chars().count() > 200 {
        return Err(ledger::error("label of 1-200 characters required"));
    }
    let label = metadata(label);
    if label.trim().is_empty() {
        return Err(ledger::error("visible label required after sanitization"));
    }
    Ok(label)
}

/// Strips terminal escapes and control characters, then redacts secrets,
/// before agent-supplied text becomes visible metadata.
fn metadata(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        let control = if c == '\u{1b}' {
            chars.next()
        } else {
            match c {
                '\u{9b}' => Some('['),
                '\u{9d}' => Some(']'),
                '\u{90}' | '\u{98}' | '\u{9e}' | '\u{9f}' => Some('P'),
                _ => None,
            }
        };
        match control {
            Some('[') => {
                for n in chars.by_ref() {
                    if ('@'..='~').contains(&n) {
                        break;
                    }
                }
            }
            Some(']' | 'P' | 'X' | '^' | '_') => {
                while let Some(n) = chars.next() {
                    if n == '\u{7}' || n == '\u{9c}' {
                        break;
                    }
                    if n == '\u{1b}' && chars.peek() == Some(&'\\') {
                        chars.next();
                        break;
                    }
                }
            }
            Some(_) => {}
            None if !c.is_control() => out.push(c),
            None => {}
        }
    }
    crate::secret_redaction::redact_secrets(&out).into_owned()
}

fn new_timer(task: &AgentTaskExecution, label: String, due: u64, now: u64) -> AgentWake {
    let id = format!("wake-{:032x}", rand::random::<u128>());
    AgentWake {
        registration_id: format!("completion-{id}"),
        id,
        task_id: task.task_id.clone(),
        room_id: task.room_id.clone(),
        agent_id: task.agent_id.clone(),
        kind: "timer".into(),
        label,
        state: "scheduled".into(),
        created_at_ms: now,
        verified_at_ms: None,
        next_due_ms: Some(due),
        interval_ms: None,
        summary: String::new(),
        detail: serde_json::Value::Null,
        fire_count: 0,
        missed_fires: 0,
        last_fired_at_ms: None,
        last_sequence: None,
        last_delivery: None,
        last_delivered_at_ms: None,
        last_acknowledged_at_ms: None,
        alerted_sequence: None,
    }
}

impl KernelRuntimeState {
    pub(super) async fn dispatch_agent_wake_tool(
        &self,
        name: &str,
        args: &serde_json::Value,
        task: &AgentTaskExecution,
    ) -> Result<serde_json::Value, DaemonError> {
        let store = &self.owned.durable_state_store;
        let now = crate::session::unix_epoch_ms();
        Ok(match name {
            "chariox.events.wakes" => {
                let wakes = super::agent_wake_scheduler::visible_wakes(
                    store.agent_wakes(Some(&task.room_id), Some(&task.agent_id))?,
                    now,
                );
                let receipts =
                    store.agent_wake_receipts(Some(&task.room_id), Some(&task.agent_id))?;
                serde_json::json!({"now_ms":now,"scheduler":self.wake_scheduler_health(now),"wakes":wakes,"receipts":receipts})
            }
            "chariox.events.timer" => {
                let delay = args["delay_ms"]
                    .as_u64()
                    .filter(|d| (1_000..=MAX_DELAY_MS).contains(d))
                    .ok_or_else(|| ledger::error("delay_ms between 1000 and 7 days required"))?;
                let interval = match args.get("interval_ms").filter(|v| !v.is_null()) {
                    None => None,
                    Some(v) => Some(
                        v.as_u64()
                            .filter(|i| (60_000..=MAX_DELAY_MS).contains(i))
                            .ok_or_else(|| {
                                ledger::error("interval_ms between 60000 and 7 days required")
                            })?,
                    ),
                };
                let mut wake = new_timer(task, label(args)?, now + delay, now);
                wake.interval_ms = interval;
                self.arm_wake_source(task, &wake)?;
                let verified = self.await_wake_verification(&wake.id).await;
                if verified.is_none() {
                    self.wake_notice(&wake, format!("Wake alert: '{}' was created but the wake scheduler did not confirm it within 5 s; the kernel sweep fires it when due", wake.label));
                } else {
                    self.refresh_wake_projection(&task.room_id);
                }
                serde_json::json!({"wake_id":wake.id,"registration_id":wake.registration_id,"first_fire_at_ms":wake.next_due_ms,"interval_ms":interval,"scheduler_confirmed":verified.is_some(),"verified_at_ms":verified})
            }
            "chariox.events.cancel_wake" => {
                let id = args["wake_id"]
                    .as_str()
                    .ok_or_else(|| ledger::error("wake_id required"))?;
                let wake = store
                    .agent_wake(id)?
                    .ok_or_else(|| ledger::error("wake unavailable"))?;
                self.cancel_wake_source(&wake, &task.task_id, Some(&task.prompt_id))?;
                self.refresh_wake_projection(&wake.room_id);
                let settled = store
                    .agent_wake(id)?
                    .is_some_and(|w| w.state == "cancelled");
                serde_json::json!({"cancel_requested":id,"settled":settled})
            }
            _ => return Err(ledger::error("unknown wake tool")),
        })
    }
}

#[cfg(test)]
mod security_tests {
    use super::*;

    #[test]
    fn security_f13_wake_label_strips_terminal_controls() {
        let result =
            label(&serde_json::json!({"label":"safe\u{1b}[2J\u{1b}]0;forged-title\u{7}\u{85}end"}))
                .unwrap();
        assert!(
            !result.chars().any(char::is_control),
            "unsafe label retained: {result:?}"
        );
        assert!(
            !result.contains("forged-title"),
            "OSC payload must not become visible metadata"
        );
    }
}
