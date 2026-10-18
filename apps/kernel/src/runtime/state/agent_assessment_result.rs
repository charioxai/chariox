//! MP-08/MP-10/MP-11: bounded private completion control, never transcript storage.
use super::*;
use crate::durable_state::agent_lifecycle::{
    goals::{self, Assessment},
    ExecutionState,
};
use zeroize::Zeroize;

mod cache;
mod protected_class;
pub(super) use cache::AssessmentResults;
pub(in crate::runtime::state) use protected_class::{ProtectedFact, ProtectedFacts};

/// MP-11: App queue/promotion and runtime actors share this exact private cell.
/// No App pointer or runtime actor is captured, so this cannot create an ownership cycle.
#[derive(Clone)]
pub(crate) struct PrivateAssessmentDispatch {
    pub(super) work_results: AssessmentResults,
    config_projection: crate::runtime::projection::DaemonConfigProjectionStore,
    durable_state_store: crate::durable_state::DurableKernelStateStore,
    room_secret_observations: super::room_secret_observation::RoomSecretObservations,
    pub(super) protected_facts: ProtectedFacts,
}
impl PrivateAssessmentDispatch {
    pub(super) fn new(
        config_projection: crate::runtime::projection::DaemonConfigProjectionStore,
        durable_state_store: crate::durable_state::DurableKernelStateStore,
        room_secret_observations: super::room_secret_observation::RoomSecretObservations,
    ) -> Self {
        Self {
            work_results: Default::default(),
            config_projection,
            durable_state_store,
            room_secret_observations,
            protected_facts: Default::default(),
        }
    }
    pub(crate) fn for_app_dispatch(
        &self,
        session: &crate::session::RuntimeSession,
        agent: &str,
        prompt: &str,
        run: &str,
        origin: crate::session::PromptOrigin,
    ) -> Result<Option<crate::prompt_assembly::AssessmentEvidence>, crate::error::DaemonError> {
        if self.config_projection.snapshot().room_agent_tools {
            self.durable_state_store.agent_lifecycle(
                crate::durable_state::agent_lifecycle::Operation::Begin {
                    owner: session.owner_user_id().into(),
                    room: session.id().into(),
                    agent: agent.into(),
                    prompt: prompt.into(),
                    run: Some(run.into()),
                    now: crate::session::unix_epoch_ms(),
                },
            )?;
        }
        Ok(self.for_dispatch(session.id(), agent, prompt, run, origin, false))
    }
    pub(crate) fn for_dispatch(
        &self,
        room: &str,
        agent: &str,
        prompt: &str,
        run: &str,
        origin: crate::session::PromptOrigin,
        steering: bool,
    ) -> Option<crate::prompt_assembly::AssessmentEvidence> {
        if origin == crate::session::PromptOrigin::CompletionCheck {
            self.take_private_assessment_evidence(room, agent, prompt, run)
        } else {
            if !steering && self.config_projection.snapshot().room_agent_tools {
                // A new work turn must not inherit a previous turn's paste outcome.
                self.protected_facts.clear(room, agent);
                self.work_results.begin_work(room, agent, prompt, run);
            }
            None
        }
    }
    fn take_private_assessment_evidence(
        &self,
        room: &str,
        agent: &str,
        prompt: &str,
        run: &str,
    ) -> Option<crate::prompt_assembly::AssessmentEvidence> {
        let task = self
            .durable_state_store
            .agent_tasks(Some(room), Some(agent))
            .ok()?
            .into_iter()
            .find(|t| {
                t.state == ExecutionState::Working
                    && t.prompt_id == prompt
                    && t.provider_run_id.as_deref() == Some(run)
                    && goals::is_check(t, prompt)
            })?;
        let check = crate::durable_state::agent_lifecycle::completion::evaluation(&task)?;
        let work_run = check.work_provider_run_id.as_deref()?;
        if work_run != run || work_run.starts_with("leased:") {
            return None;
        }
        let bytes = self
            .work_results
            .take(room, agent, &check.work_prompt_id, work_run)?;
        if !self.room_secret_observations.protects_bytes(room) {
            return None;
        }
        let text = std::str::from_utf8(&bytes).ok()?.trim();
        if text.is_empty() {
            return None;
        }
        let text = self
            .room_secret_observations
            .scrub(room, text.to_string())
            .ok()?;
        let text = crate::secret_redaction::redact_secrets(&text);
        crate::prompt_assembly::AssessmentEvidence::new(serde_json::json!({"task_id":task.task_id,"work_prompt_id":check.work_prompt_id,"provider_run_id":work_run,"complete":true,"provider_assistant_result":text}).to_string())
    }
}

impl KernelRuntimeOwnedState {
    pub(super) fn private_evidence_for_dispatch(
        &self,
        room: &str,
        agent: &str,
        prompt: &str,
        run: &str,
        origin: crate::session::PromptOrigin,
        steering: bool,
    ) -> Option<crate::prompt_assembly::AssessmentEvidence> {
        self.private_assessment_dispatch
            .for_dispatch(room, agent, prompt, run, origin, steering)
    }
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
        let Ok(tasks) = self
            .durable_state_store
            .agent_tasks(Some(room), Some(agent))
        else {
            return;
        };
        if prompt.prompt_origin() != crate::session::PromptOrigin::CompletionCheck {
            if tasks.iter().any(|t| {
                t.state == ExecutionState::Working
                    && t.prompt_id == prompt.id()
                    && t.provider_run_id.as_deref() == Some(&output.provider_run_id)
            }) {
                self.private_work_results.append_work(
                    room,
                    agent,
                    prompt.id(),
                    &output.provider_run_id,
                    &output.bytes,
                );
            }
            return;
        }
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
    #[cfg(test)]
    pub(super) fn take_private_assessment_evidence(
        &self,
        room: &str,
        agent: &str,
        prompt: &str,
        run: &str,
    ) -> Option<crate::prompt_assembly::AssessmentEvidence> {
        self.private_assessment_dispatch
            .take_private_assessment_evidence(room, agent, prompt, run)
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
            let facts = &self.private_assessment_dispatch.protected_facts;
            let fact = facts.peek(room, agent);
            if !fence_private_assessment(&mut value, &live, fact) {
                return String::new();
            }
            if protected_class::is_terminal(&value) {
                facts.clear(room, agent);
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
// The only public prose is a fixed kernel-authored reason class.
fn fence_private_assessment(
    value: &mut Assessment,
    live: &[String],
    fact: Option<ProtectedFact>,
) -> bool {
    let public = protected_class::public_text(value, fact);
    let fence = |text: &mut String| {
        text.zeroize();
        *text = public.into();
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
            assert!(fence_private_assessment(&mut value, &["known-live".into()], None));
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
                &["known-live".into()],
                None
            ));
        }
        assert!(serde_json::from_str::<Assessment>(
            r#"{"status":"blocked","reason":"safe","extra":"PRIVATE_DERIVED"}"#
        )
        .is_err());
    }
    #[test]
    fn mp11_reason_class_is_fixed_and_fails_closed() {
        use ProtectedFact::*;
        let class = |raw: &str, fact| {
            let mut value: Assessment = serde_json::from_str(raw).unwrap();
            assert!(fence_private_assessment(&mut value, &["live".into()], fact));
            serde_json::to_string(&value).unwrap()
        };
        let waiting = r#"{"status":"waiting","reason":"PRIVATE","registration_ids":["live"],"deadline_ms":1}"#;
        let cont = r#"{"status":"continue","reason":"PRIVATE"}"#;
        let achieved = r#"{"status":"achieved","final_answer":"PRIVATE","evidence":["PRIVATE"]}"#;
        assert!(class(waiting, None).contains("[protected task: waiting]"));
        assert!(class(waiting, Some(ApprovalDenied)).contains("approval denied"));
        assert!(class(cont, Some(FieldNotFound)).contains("protected field not found"));
        assert!(class(achieved, Some(Submitted)).contains("[protected task: completed]"));
        assert!(class(achieved, Some(ApprovalDenied)).contains("[protected task: approval denied]"));
        assert!(class(achieved, Some(ApprovalTimedOut)).contains("[protected task: timed out]"));
        for fact in [FieldNotFound, Failed] {
            let out = class(achieved, Some(fact));
            assert!(out.contains("[protected completion details withheld]"), "{out}");
        }
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
