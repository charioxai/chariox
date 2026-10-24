//! MP-08 / MP-10 / MP-11: restart recovery never waits for a person. A cold
//! Claude launch whose folder ships hooks asks once per agent in the
//! background; the recovery pass moves on to other agents and picks the answer
//! up on a later pass. A refusal or timeout is final for this recovery run:
//! the person starts the agent again to be asked again.
use super::*;
use std::sync::Mutex as StdMutex;

enum Decision {
    Waiting,
    Granted,
    Refused { message: String, reported: bool },
}

/// Why a recovery step did not start its Claude launch.
pub(super) enum HeldBack {
    /// The hooks card is open; a later pass continues once it is answered.
    Waiting,
    /// The person refused (or the card timed out) with this reason.
    Refused(String),
}

pub(super) enum RecoveryAdmission {
    Admitted,
    HeldBack(HeldBack),
}

#[derive(Default)]
pub(crate) struct RestartLaunchAdmissions {
    decisions: Arc<StdMutex<BTreeMap<String, Decision>>>,
    asks: StdMutex<tokio::task::JoinSet<()>>,
    /// Agents whose next queued prompt still has to start once its launch is
    /// admitted: (session id, agent id).
    owed_followers: StdMutex<BTreeSet<(String, String)>>,
}

impl RestartLaunchAdmissions {
    /// Work the recovery loop must still pass over: open cards, answers not yet
    /// acted on, and queued followers waiting for their agent.
    pub(super) fn unsettled(&self) -> usize {
        let decisions = self.decisions.lock().expect("admission state poisoned");
        decisions
            .values()
            .filter(|decision| !matches!(decision, Decision::Refused { reported: true, .. }))
            .count()
            + self.owed_followers().len()
    }

    /// Whether a card is still open, so the answer can still arrive.
    pub(super) fn any_waiting(&self) -> bool {
        self.decisions
            .lock()
            .expect("admission state poisoned")
            .values()
            .any(|decision| matches!(decision, Decision::Waiting))
    }

    pub(super) fn owe_follower(&self, session_id: &str, agent_id: &str) {
        self.owed_followers
            .lock()
            .expect("admission state poisoned")
            .insert((session_id.to_string(), agent_id.to_string()));
    }

    pub(super) fn take_owed_followers(&self) -> Vec<(String, String)> {
        std::mem::take(&mut *self.owed_followers())
            .into_iter()
            .collect()
    }

    fn owed_followers(&self) -> std::sync::MutexGuard<'_, BTreeSet<(String, String)>> {
        self.owed_followers
            .lock()
            .expect("admission state poisoned")
    }

    /// What this agent's open question stands at; an answered question is
    /// consumed here, except a refusal, which stays final.
    fn recorded(&self, agent_id: &str) -> Option<HeldBack> {
        let mut decisions = self.decisions.lock().expect("admission state poisoned");
        match decisions.get_mut(agent_id)? {
            Decision::Waiting => Some(HeldBack::Waiting),
            Decision::Refused { message, reported } => {
                *reported = true;
                Some(HeldBack::Refused(message.clone()))
            }
            Decision::Granted => {
                decisions.remove(agent_id);
                None
            }
        }
    }

    fn ask(
        &self,
        state: &KernelRuntimeState,
        request: crate::provider::LaunchProviderRequest,
        agent_id: &str,
    ) {
        self.decisions
            .lock()
            .expect("admission state poisoned")
            .insert(agent_id.to_string(), Decision::Waiting);
        let (state, decisions, agent_id) = (
            state.clone(),
            Arc::clone(&self.decisions),
            agent_id.to_string(),
        );
        let mut asks = self.asks.lock().expect("admission state poisoned");
        while asks.try_join_next().is_some() {}
        asks.spawn(async move {
            let decision = match state.admit_claude_workspace_trust(&request).await {
                Ok(()) => Decision::Granted,
                Err(error) => {
                    let message = error.to_string();
                    let recipients = state
                        .owned
                        .attachment_store
                        .list_session_attachment_ids(&request.session_id);
                    state.owned.record_notice_for_agent(
                        &request.session_id,
                        None,
                        request.agent_id.as_deref(),
                        recipients,
                        message.clone(),
                    );
                    Decision::Refused {
                        message,
                        reported: false,
                    }
                }
            };
            decisions
                .lock()
                .expect("admission state poisoned")
                .insert(agent_id, decision);
        });
    }
}

impl KernelRuntimeState {
    /// The restart-recovery form of `admit_cold_claude_launch`: it never waits
    /// for the person, and it asks at most once per agent.
    pub(super) async fn admit_restart_recovery_launch(
        &self,
        admissions: &RestartLaunchAdmissions,
        session_id: &str,
        agent_id: &str,
    ) -> Result<RecoveryAdmission, DaemonError> {
        if let Some(held_back) = admissions.recorded(agent_id) {
            return Ok(RecoveryAdmission::HeldBack(held_back));
        }
        let (session, agent) = (session_id.to_owned(), agent_id.to_owned());
        let Some(request) = self
            .with_app_side_effect(move |app| app.cold_claude_launch_for_agent(&session, &agent))
            .await?
        else {
            return Ok(RecoveryAdmission::Admitted);
        };
        if self
            .untrusted_folder_awaiting_hook_approval(&request)?
            .is_none()
        {
            self.admit_claude_workspace_trust(&request).await?;
            return Ok(RecoveryAdmission::Admitted);
        }
        admissions.ask(self, request, agent_id);
        Ok(RecoveryAdmission::HeldBack(HeldBack::Waiting))
    }
}
