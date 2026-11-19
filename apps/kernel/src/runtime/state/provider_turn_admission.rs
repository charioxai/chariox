//! Hold accepted turns in their normal durable prompt queues while memory is short.
use super::*;
use crate::runtime::process_resources::LaunchAdmission;
use crate::session::{PromptQueueItem, RuntimeSession};

const PROVIDER_MEMORY_WAIT: &str = "provider_memory_wait";
const HELD_TURN_MEMORY_STABILITY: Duration = Duration::from_secs(1);

pub(super) fn provider_memory_wait_error(message: String) -> DaemonError {
    DaemonError::RelayTransport {
        operation: "wait for provider memory",
        code: PROVIDER_MEMORY_WAIT.into(),
        message,
        retryable: true,
    }
}

pub(super) fn provider_memory_wait_reason(error: &DaemonError) -> Option<&str> {
    match error {
        DaemonError::RelayTransport { code, message, .. } if code == PROVIDER_MEMORY_WAIT => {
            Some(message)
        }
        _ => None,
    }
}

impl KernelRuntimeOwnedState {
    pub(super) fn hold_queued_provider_turn(
        &self,
        session: &RuntimeSession,
        agent_id: &str,
        head: &PromptQueueItem,
    ) -> Result<bool, DaemonError> {
        let Some(reason) = self.prompt_state_owner.provider_launch_wait() else {
            return Ok(false);
        };
        if self
            .prompt_state_owner
            .note_provider_launch_wait(session, agent_id, head.id(), &reason)
        {
            let (active, queued) = self.prompt_state_owner.state_parts(session, agent_id);
            self.mirror_prompt_owner_agent_state(session.id(), agent_id, active, queued)?;
        }
        Ok(true)
    }
}

impl KernelRuntimeState {
    pub(crate) fn sample_provider_turn_pressure(&self, admission: &LaunchAdmission) -> bool {
        let short = admission.provider_memory_is_short();
        let owner = &self.owned.prompt_state_owner;
        match (short, owner.provider_launch_wait().is_some()) {
            (true, false) => {
                let machine = self.owned.config_projection.snapshot().machine_name();
                let reason = machine.map_or_else(
                    || "Waiting for memory on this machine".into(),
                    |name| format!("Waiting for memory on {name}"),
                );
                owner.set_provider_launch_wait(Some(reason));
            }
            (false, true) => owner.set_provider_launch_wait(None),
            _ => {}
        }
        short
    }

    pub(crate) async fn run_provider_turn_admission(&self) {
        let admission = LaunchAdmission::default();
        let sample = async {
            let mut interval = provider_admission_interval();
            loop {
                interval.tick().await;
                self.sample_provider_turn_pressure(&admission);
            }
        };
        let promote = async {
            let mut interval = provider_admission_interval();
            let mut next_worker_retry = tokio::time::Instant::now();
            let mut memory_available_since = None;
            loop {
                interval.tick().await;
                if !self.owned.publication_activation.is_active() {
                    continue;
                }
                let now = tokio::time::Instant::now();
                let retry_workers = now >= next_worker_retry;
                if retry_workers {
                    next_worker_retry = now + Duration::from_secs(1);
                }
                // A replacement can briefly release its old process memory while it
                // rebuilds its state. Resume held turns only after headroom persists.
                let mut resume_local = if self
                    .owned
                    .prompt_state_owner
                    .provider_launch_wait()
                    .is_none()
                {
                    now.duration_since(*memory_available_since.get_or_insert(now))
                        >= HELD_TURN_MEMORY_STABILITY
                } else {
                    memory_available_since = None;
                    false
                };
                for (session_id, agent_id) in
                    self.owned.prompt_state_owner.provider_launch_waiters()
                {
                    let Ok(agent) = self.owned.agent_store.get_agent(&agent_id) else {
                        continue;
                    };
                    if agent.remote_execution().is_some() {
                        if retry_workers {
                            if let Err(error) = self
                                .spawn_next_queued_remote_prompt(&session_id, &agent_id)
                                .await
                            {
                                tracing::warn!(%error, %session_id, %agent_id, "Could not advance a remote turn waiting for memory");
                            }
                        }
                    } else if resume_local {
                        if self.sample_provider_turn_pressure(&admission) {
                            memory_available_since = None;
                            resume_local = false;
                            continue;
                        }
                        Box::pin(self.advance_project_queued_prompt_after_settlement(
                            &session_id,
                            &agent_id,
                        ))
                        .await;
                    }
                }
            }
        };
        tokio::join!(sample, promote);
    }
}

fn provider_admission_interval() -> tokio::time::Interval {
    let mut interval = tokio::time::interval(Duration::from_millis(250));
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    interval
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::local::test_support::LocalRouterTestHarness;
    use crate::local::{AttachToSessionRequest, LocalDaemonRequest, LocalDaemonResponse};
    use crate::session::{PromptStatus, PromptSubmissionOutcome, SessionAgentDefaults};

    const WAIT: &str = "Waiting for memory on builder";

    #[tokio::test]
    async fn provider_turn_admission_for_a_leased_worker_waits_before_provider_launch() {
        use crate::transport::relay_peer::{
            LeasedPromptReceiptPhase, RelayAgentExecutionProfile, RemoteGitTurnContext,
        };

        let (worker, leased) =
            super::super::leased_agent_deadline_tests::worker_with_leased_agent();
        let home_prompt_id = "home-prompt-waiting-for-memory";
        let owner = &worker.owned.prompt_state_owner;
        owner.set_provider_launch_wait(Some(WAIT.into()));
        let outcome = worker
            .submit_relay_leased_prompt(
                &leased.id,
                RelayAgentExecutionProfile {
                    provider: leased.provider.clone(),
                    account_profile: leased.account_profile.clone(),
                    model: leased.model.clone(),
                    effort: leased.effort.clone(),
                },
                None,
                "Worker turn",
                "",
                Vec::new(),
                None,
                Some(RemoteGitTurnContext {
                    home_session_id: "home-session".into(),
                    home_agent_id: "home-agent".into(),
                    home_prompt_id: home_prompt_id.into(),
                    home_turn_id: home_prompt_id.into(),
                    source_attachment_id: None,
                    workspace_live_sync_mode: None,
                    prompt_origin: None,
                    external_provider: None,
                    external_provider_session_id: None,
                    external_provider_turn_id: None,
                    prompt_summary: "Worker turn".into(),
                }),
                Vec::new(),
                crate::extension::RemoteExtensionManifest::default(),
                None,
                Default::default(),
            )
            .await
            .unwrap_err();
        assert_eq!(provider_memory_wait_reason(&outcome), Some(WAIT));
        assert!(worker.owned.provider_store.list_runs().is_empty());

        let receipt = worker
            .query_relay_leased_prompt_receipt(&leased.id, home_prompt_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(receipt.phase, LeasedPromptReceiptPhase::WaitingForMemory);
        assert_eq!(receipt.home_prompt_id, home_prompt_id);
        assert_eq!(
            receipt.execution_lease_id.as_deref(),
            Some(leased.lease_id.as_str())
        );
        assert!(receipt.worker_provider_run_id.is_empty());
    }

    #[test]
    fn provider_turn_admission_holds_cold_local_turns_and_preserves_person_priority() {
        let harness = LocalRouterTestHarness::new();
        let LocalDaemonResponse::SessionCreated { session, agent, .. } = harness
            .dispatch(LocalDaemonRequest::CreateSession(
                harness
                    .fixture_session_request("provider-turn-admission")
                    .with_agent_defaults(SessionAgentDefaults::new("dev-stub")),
            ))
            .unwrap()
        else {
            panic!("expected a Room");
        };
        let attach = |client_id: &str| {
            let LocalDaemonResponse::SessionAttached { attachment, .. } = harness
                .dispatch(LocalDaemonRequest::AttachToSession(
                    AttachToSessionRequest {
                        session_id: session.id().into(),
                        client_id: client_id.into(),
                        capability_level: crate::attachment::ClientCapabilityLevel::FullTerminal,
                    },
                ))
                .unwrap()
            else {
                panic!("expected an attachment");
            };
            attachment
        };
        let automatic = attach("agent:fixture");
        let person = attach("terminal:person");
        let state = harness.runtime_state();
        let owner = &state.owned.prompt_state_owner;
        owner.set_provider_launch_wait(Some(WAIT.into()));
        for (attachment, text) in [(automatic, "automatic"), (person, "person")] {
            let response = harness
                .dispatch(LocalDaemonRequest::SubmitPrompt(
                    crate::local::SubmitPromptRequest {
                        session_id: session.id().into(),
                        attachment_id: attachment.id().into(),
                        target_agent_id: Some(agent.id().into()),
                        prompt: text.into(),
                        attachments: Vec::new(),
                    },
                ))
                .unwrap();
            let LocalDaemonResponse::PromptSubmitted {
                outcome: PromptSubmissionOutcome::Queued { prompt },
                ..
            } = response
            else {
                panic!("a cold turn should wait when memory is short");
            };
            assert_eq!(serde_json::to_value(prompt).unwrap()["launch_wait"], WAIT);
        }
        state
            .recover_queue_head_after_restart(session.id(), agent.id())
            .unwrap();
        assert!(state
            .owned
            .provider_store
            .get_run_for_agent(session.id(), agent.id())
            .is_none());
        assert_eq!(
            owner.provider_launch_waiters(),
            vec![(session.id().into(), agent.id().into())]
        );
        let persisted = state.owned.session_store.get_session(session.id()).unwrap();
        let restarted = crate::runtime::prompt_state::PromptStateOwner::default();
        restarted.restore_session_state(&persisted);
        let (active, queued) = restarted.state_parts(&persisted, agent.id());
        assert!(active.is_none());
        assert_eq!(
            queued
                .iter()
                .map(PromptQueueItem::prompt)
                .collect::<Vec<_>>(),
            ["person", "automatic"]
        );
        assert!(queued
            .iter()
            .all(|prompt| prompt.launch_wait() == Some(WAIT)));
        let mut admitted = queued[0].clone();
        admitted.set_status(PromptStatus::Running);
        assert_eq!(admitted.launch_wait(), None);
    }
}
