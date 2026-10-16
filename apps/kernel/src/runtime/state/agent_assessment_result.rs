//! MP-08/MP-10/MP-11: bounded private completion control, never transcript storage.
use super::*;
use crate::durable_state::agent_lifecycle::{
    goals::{self, Assessment},
    ExecutionState,
};
use zeroize::{Zeroize, Zeroizing};

#[derive(Clone, Default)]
pub(super) struct AssessmentResults(Arc<std::sync::Mutex<BTreeMap<(String, String), ResultCell>>>);
struct ResultCell {
    prompt: String,
    run: String,
    expires: Instant,
    bytes: Zeroizing<Vec<u8>>,
    overflow: bool,
}
impl AssessmentResults {
    fn append(&self, room: &str, agent: &str, prompt: &str, run: &str, bytes: &[u8]) {
        let mut cells = self.0.lock().unwrap_or_else(|e| e.into_inner());
        cells.retain(|_, cell| cell.expires > Instant::now());
        let key = (room.to_string(), agent.to_string());
        if !cells.contains_key(&key) && cells.len() >= 128 {
            return;
        }
        let cell = cells.entry(key).or_insert_with(|| ResultCell {
            prompt: prompt.into(),
            run: run.into(),
            expires: Instant::now() + Duration::from_secs(60),
            bytes: Zeroizing::new(Vec::new()),
            overflow: false,
        });
        if cell.prompt != prompt || cell.run != run {
            *cell = ResultCell {
                prompt: prompt.into(),
                run: run.into(),
                expires: Instant::now() + Duration::from_secs(60),
                bytes: Zeroizing::new(Vec::new()),
                overflow: false,
            };
        }
        if cell.overflow {
            return;
        }
        if cell.bytes.len().saturating_add(bytes.len()) > goals::RESULT_LIMIT {
            cell.bytes = Zeroizing::new(Vec::new());
            cell.overflow = true;
        } else {
            cell.bytes.extend_from_slice(bytes);
        }
    }
    fn take(&self, room: &str, agent: &str, prompt: &str, run: &str) -> Option<Zeroizing<Vec<u8>>> {
        let mut cells = self.0.lock().unwrap_or_else(|e| e.into_inner());
        let key = (room.into(), agent.into());
        let cell = cells.get(&key)?;
        if cell.prompt != prompt || cell.run != run {
            return None;
        }
        let cell = cells.remove(&key)?;
        (!cell.overflow && cell.expires > Instant::now()).then_some(cell.bytes)
    }
}
impl KernelRuntimeOwnedState {
    pub(super) fn collect_private_assessment(
        &self,
        room: &str,
        output: &super::prompt_transcript_owned_state::TerminalOutputBatchAppend,
    ) {
        if output.kind != crate::terminal::TerminalOutputKind::ProviderOutput {
            return;
        }
        let Some(agent) = output.agent_id.as_deref() else {
            return;
        };
        let Ok(session) = self.session_store.get_session(room) else {
            return;
        };
        let Some(prompt) = self
            .prompt_state_owner
            .active_prompt_for_agent(&session, agent)
        else {
            return;
        };
        if prompt.prompt_origin() != crate::session::PromptOrigin::CompletionCheck {
            return;
        }
        let Ok(tasks) = self
            .durable_state_store
            .agent_tasks(Some(room), Some(agent))
        else {
            return;
        };
        if !tasks.iter().any(|task| {
            task.state == ExecutionState::Working
                && task.prompt_id == prompt.id()
                && task.provider_run_id.as_deref() == Some(&output.provider_run_id)
                && goals::is_check(task, prompt.id())
        }) {
            return;
        }
        self.private_assessment_results.append(
            room,
            agent,
            prompt.id(),
            &output.provider_run_id,
            &output.bytes,
        );
    }
    pub(super) fn take_private_assessment(
        &self,
        room: &str,
        agent: &str,
        prompt: &str,
        run: &str,
    ) -> String {
        // Parse the private result before applying value redaction. The public byte
        // fence intentionally destroys free text and cannot serve as control input.
        let Some(bytes) = self
            .private_assessment_results
            .take(room, agent, prompt, run)
        else {
            return String::new();
        };
        let Ok(text) = std::str::from_utf8(&bytes) else {
            return String::new();
        };
        let text = text.trim();
        let text = text
            .strip_prefix("```json")
            .or_else(|| text.strip_prefix("```"))
            .and_then(|s| s.strip_suffix("```"))
            .unwrap_or(text)
            .trim();
        // Reject unknown fields before the ledger can retain an invalid receipt.
        let Ok(mut value) = serde_json::from_str::<Assessment>(text) else {
            return String::new();
        };
        if self.room_secret_observations.protects_bytes(room) {
            let live = if matches!(value, Assessment::Waiting { .. }) {
                let task = self
                    .durable_state_store
                    .agent_tasks(Some(room), Some(agent))
                    .ok()
                    .and_then(|tasks| {
                        tasks.into_iter().find(|task| {
                            task.state == ExecutionState::Working
                                && task.provider_run_id.as_deref() == Some(run)
                                && goals::is_check(task, prompt)
                        })
                    });
                let Some(task) = task else {
                    return String::new();
                };
                let Ok(registrations) = self.durable_state_store.agent_registrations(&task.task_id)
                else {
                    return String::new();
                };
                registrations
                    .into_iter()
                    .filter(|r| r.live)
                    .map(|r| r.id)
                    .collect::<Vec<_>>()
            } else {
                Vec::new()
            };
            if !fence_private_assessment(&mut value, &live) {
                return String::new();
            }
        }
        self.room_secret_observations
            .scrub(room, value)
            .ok()
            .and_then(|value| serde_json::to_string(&value).ok())
            .unwrap_or_default()
    }
}

// The structured disposition is control input. Protected provider prose cannot
// enter the ledger/audit/public result, including through a malformed receipt.
fn fence_private_assessment(value: &mut Assessment, live: &[String]) -> bool {
    let fence = |text: &mut String| {
        text.zeroize();
        *text = "[protected completion details withheld]".into();
    };
    match value {
        Assessment::Achieved {
            final_answer,
            evidence,
        } => {
            if final_answer.trim().is_empty()
                || evidence.is_empty()
                || evidence.iter().any(|s| s.trim().is_empty())
            {
                return false;
            }
            fence(final_answer);
            for item in evidence.iter_mut() {
                item.zeroize()
            }
            evidence.clear();
            evidence.push("[protected completion evidence withheld]".into());
        }
        Assessment::Continue { reason } | Assessment::Blocked { reason } => {
            if reason.trim().is_empty() {
                return false;
            }
            fence(reason);
        }
        Assessment::Waiting {
            reason,
            registration_ids,
            ..
        } => {
            // Only pre-existing live task references may survive as control.
            // The ledger still revalidates sources, obligations and the deadline.
            if reason.trim().is_empty()
                || registration_ids.is_empty()
                || registration_ids.iter().any(|id| !live.contains(id))
            {
                return false;
            }
            fence(reason);
            registration_ids.sort();
            registration_ids.dedup();
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn mp11_private_assessment_fences_prose_and_rejects_unadmitted_fields() {
        for raw in [
            r#"{"status":"achieved","final_answer":"PRIVATE_DERIVED","evidence":["PRIVATE_REFERENCE"]}"#,
            r#"{"status":"continue","reason":"PRIVATE_DERIVED"}"#,
            r#"{"status":"blocked","reason":"PRIVATE_DERIVED"}"#,
            r#"{"status":"waiting","reason":"PRIVATE_DERIVED","registration_ids":["known-live"],"deadline_ms":123}"#,
        ] {
            let mut value: Assessment = serde_json::from_str(raw).unwrap();
            assert!(fence_private_assessment(&mut value, &["known-live".into()]));
            assert!(!serde_json::to_string(&value).unwrap().contains("PRIVATE_"));
        }
        for raw in [
            r#"{"status":"achieved","final_answer":"","evidence":["PRIVATE_REFERENCE"]}"#,
            r#"{"status":"achieved","final_answer":"PRIVATE_DERIVED","evidence":[]}"#,
            r#"{"status":"waiting","reason":"PRIVATE_DERIVED","registration_ids":["PRIVATE_REFERENCE"],"deadline_ms":123}"#,
        ] {
            let mut value: Assessment = serde_json::from_str(raw).unwrap();
            assert!(!fence_private_assessment(
                &mut value,
                &["known-live".into()]
            ));
        }
        assert!(serde_json::from_str::<Assessment>(
            r#"{"status":"blocked","reason":"safe","extra":"PRIVATE_DERIVED"}"#
        )
        .is_err());
    }
    #[test]
    fn mp11_private_assessment_is_single_use_binding_exact_and_never_truncates() {
        let results = AssessmentResults::default();
        results.append("room", "agent", "check", "run", b"private control");
        for (room, agent, prompt, run) in [
            ("foreign", "agent", "check", "run"),
            ("room", "foreign", "check", "run"),
            ("room", "agent", "old-check", "run"),
            ("room", "agent", "check", "old-run"),
        ] {
            assert!(results.take(room, agent, prompt, run).is_none());
        }
        assert_eq!(
            results
                .take("room", "agent", "check", "run")
                .unwrap()
                .as_slice(),
            b"private control"
        );
        assert!(results.take("room", "agent", "check", "run").is_none());
        results.append(
            "room",
            "agent",
            "check",
            "run",
            &vec![b'x'; goals::RESULT_LIMIT + 1],
        );
        results.append("room", "agent", "check", "run", b"{}");
        assert!(results.take("room", "agent", "check", "run").is_none());
    }
}
