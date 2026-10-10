use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::sync::{Arc, Mutex as StdMutex};

use crate::error::DaemonError;
use crate::session::{PromptQueueItem, PromptStatus, PromptSubmissionOutcome, RuntimeSession};

mod profile_transition;
pub(crate) use profile_transition::AgentProfileTransitionClaim;

pub(crate) const PROMPT_QUEUE_LIMIT: usize = 128;

pub(crate) fn prompt_claims_provider_run(
    prompt: &PromptQueueItem,
    provider_run_id: &str,
    unbound_run_selected: bool,
) -> bool {
    // An explicit delivery binding takes precedence over selection fallback.
    prompt
        .durable_delivery_provider_run_id()
        .map_or(unbound_run_selected, |delivery_run_id| {
            delivery_run_id == provider_run_id
        })
}

#[derive(Debug, Clone, Default)]
struct OwnedAgentPromptState {
    // Ephemeral: never copied from the durable session mirror.
    sudo_entry_id: Option<String>,
    // MP-08/MP-10/MP-11 F5: captured at each running sudo prompt binding.
    sudo_process_cutoff: Option<u64>,
    // MP-08/MP-10/MP-11 A04: while a sudo window's work is open, only its
    // kernel-correlated prompts may start; others stay queued (causal fence).
    sudo_work: Option<SudoWorkHold>,
    active_prompt: Option<PromptQueueItem>,
    queued_prompts: VecDeque<PromptQueueItem>,
}

#[derive(Debug, Clone, Default)]
struct SudoWorkHold {
    entry_id: String,
    prompts: BTreeSet<String>,
}

impl OwnedAgentPromptState {
    /// Whether `prompt` may become active now; a held work prompt is bound.
    fn sudo_admits(&self, prompt: &str) -> bool {
        self.sudo_work
            .as_ref()
            .is_none_or(|hold| hold.prompts.contains(prompt))
    }

    fn bind_sudo_work(&mut self) {
        let Some(active) = self.active_prompt.as_ref() else {
            return;
        };
        if let Some(hold) = self
            .sudo_work
            .as_ref()
            .filter(|hold| hold.prompts.contains(active.id()))
        {
            if self.sudo_entry_id.as_deref() != Some(hold.entry_id.as_str()) {
                self.sudo_process_cutoff =
                    crate::runtime::kernel_access::process::birth_cutoff().ok();
            }
            self.sudo_entry_id = Some(hold.entry_id.clone());
        }
    }

    fn take_active_prompt(&mut self) -> Option<PromptQueueItem> {
        self.sudo_entry_id = None;
        self.sudo_process_cutoff = None;
        self.active_prompt.take()
    }

    fn set_active_prompt(&mut self, prompt: Option<PromptQueueItem>) {
        if self.active_prompt.as_ref().map(PromptQueueItem::id)
            != prompt.as_ref().map(PromptQueueItem::id)
        {
            self.sudo_entry_id = None;
            self.sudo_process_cutoff = None;
        }
        self.active_prompt = prompt;
    }

    fn from_session(session: &RuntimeSession, agent_id: &str) -> Self {
        session
            .prompt_states()
            .get(agent_id)
            .map(|state| Self {
                sudo_entry_id: None,
                sudo_process_cutoff: None,
                sudo_work: None,
                active_prompt: state.active_prompt().cloned(),
                queued_prompts: state.queued_prompts().clone(),
            })
            .unwrap_or_default()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct PromptStateKey {
    session_id: String,
    agent_id: String,
}

impl PromptStateKey {
    fn new(session_id: &str, agent_id: &str) -> Self {
        Self {
            session_id: session_id.to_string(),
            agent_id: agent_id.to_string(),
        }
    }
}

#[derive(Debug, Clone, Default)]
pub(crate) struct PromptStateOwner {
    state: Arc<StdMutex<PromptStateOwnerState>>,
    delivery_settlement_claims: Arc<StdMutex<BTreeSet<PromptStateKey>>>,
}

pub(crate) struct PromptDeliverySettlementClaim {
    key: PromptStateKey,
    claims: Arc<StdMutex<BTreeSet<PromptStateKey>>>,
}

impl Drop for PromptDeliverySettlementClaim {
    fn drop(&mut self) {
        self.claims
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(&self.key);
    }
}

#[derive(Debug, Default)]
struct PromptStateOwnerState {
    states: BTreeMap<PromptStateKey, OwnedAgentPromptState>,
    profile_transitions: BTreeMap<PromptStateKey, Arc<()>>,
    next_pending_prompt_number: u64,
}

impl PromptStateOwner {
    /// Commit delivery settlement while admission/cancellation cannot replace
    /// this prompt. The callback must not reenter this owner.
    pub(crate) fn settle_active_remote_dispatch_if_matches(
        &self,
        session: &RuntimeSession,
        agent_id: &str,
        prompt_id: &str,
        delivered_run_id: Option<&str>,
        commit: impl FnOnce(
            &PromptQueueItem,
            Option<PromptQueueItem>,
            VecDeque<PromptQueueItem>,
        ) -> Result<bool, DaemonError>,
    ) -> Result<Option<PromptQueueItem>, DaemonError> {
        let mut owner = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let state = owner.ensure_agent_state(session, agent_id);
        let Some(current) = state.active_prompt.as_ref().filter(|prompt| {
            prompt.id() == prompt_id
                && matches!(
                    prompt.durable_delivery_phase(),
                    Some(crate::session::DurablePromptDeliveryPhase::Accepted)
                        | Some(crate::session::DurablePromptDeliveryPhase::Dispatching)
                )
        }) else {
            return Ok(None);
        };
        let mut settled = current.clone();
        let next = if let Some(run_id) = delivered_run_id {
            settled.set_durable_delivery(
                crate::session::DurablePromptDeliveryPhase::Delivered,
                Some(run_id.to_string()),
                None,
            );
            if settled.status() == PromptStatus::Dispatching {
                settled.set_status(PromptStatus::Running);
            }
            Some(settled.clone())
        } else {
            settled.set_status(PromptStatus::Cancelled);
            None
        };
        if !commit(current, next.clone(), state.queued_prompts.clone())? {
            return Ok(None);
        }
        state.active_prompt = next;
        Ok(Some(settled))
    }

    pub(crate) fn try_claim_active_prompt_delivery_settlement(
        &self,
        session: &RuntimeSession,
        agent_id: &str,
        prompt_id: &str,
        provider_run_id: &str,
    ) -> Option<PromptDeliverySettlementClaim> {
        let key = PromptStateKey::new(session.id(), agent_id);
        {
            let mut claims = self
                .delivery_settlement_claims
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if !claims.insert(key.clone()) {
                return None;
            }
        }
        let matches = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .ensure_agent_state(session, agent_id)
            .active_prompt
            .as_ref()
            .is_some_and(|prompt| {
                prompt.id() == prompt_id
                    && prompt.durable_delivery_phase()
                        == Some(crate::session::DurablePromptDeliveryPhase::Dispatching)
                    && prompt.durable_delivery_provider_run_id() == Some(provider_run_id)
            });
        if !matches {
            self.delivery_settlement_claims
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .remove(&key);
            return None;
        }
        Some(PromptDeliverySettlementClaim {
            key,
            claims: Arc::clone(&self.delivery_settlement_claims),
        })
    }

    pub(crate) fn compare_and_mark_active_prompt_delivery_failure(
        &self,
        session: &RuntimeSession,
        agent_id: &str,
        prompt_id: &str,
        provider_run_id: &str,
        provider_session_id: &str,
        status_transition: (PromptStatus, PromptStatus),
    ) -> Option<PromptQueueItem> {
        let (expected_status, next_status) = status_transition;
        let mut owner = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let state = owner.ensure_agent_state(session, agent_id);
        let active = state.active_prompt.as_mut()?;
        if active.id() != prompt_id
            || active.status() != expected_status
            || active.durable_delivery_phase()
                != Some(crate::session::DurablePromptDeliveryPhase::Dispatching)
            || active.durable_delivery_provider_run_id() != Some(provider_run_id)
            || active.durable_delivery_provider_session_id() != Some(provider_session_id)
        {
            return None;
        }
        if next_status == PromptStatus::Cancelling {
            state.sudo_entry_id = None;
            state.sudo_process_cutoff = None;
        }
        active.set_status(next_status);
        active.set_durable_delivery_failure_pending(next_status == PromptStatus::Cancelling);
        Some(active.clone())
    }

    pub(crate) fn compare_and_restore_active_prompt_after_resume_superseded(
        &self,
        session: &RuntimeSession,
        agent_id: &str,
        prompt_id: &str,
        provider_run_id: &str,
        failed_provider_session_id: &str,
        current_provider_session_id: &str,
    ) -> Option<(PromptQueueItem, PromptQueueItem)> {
        let mut owner = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let active = owner
            .ensure_agent_state(session, agent_id)
            .active_prompt
            .as_mut()?;
        if active.id() != prompt_id
            || active.status() != PromptStatus::Cancelling
            || !active.durable_delivery_failure_pending()
            || active.durable_delivery_phase()
                != Some(crate::session::DurablePromptDeliveryPhase::Dispatching)
            || active.durable_delivery_provider_run_id() != Some(provider_run_id)
            || active.durable_delivery_provider_session_id() != Some(failed_provider_session_id)
        {
            return None;
        }
        let previous = active.clone();
        active.set_status(PromptStatus::Dispatching);
        active.set_durable_delivery(
            crate::session::DurablePromptDeliveryPhase::Dispatching,
            Some(provider_run_id.to_string()),
            Some(current_provider_session_id.to_string()),
        );
        active.set_durable_delivery_failure_pending(false);
        Some((previous, active.clone()))
    }

    pub(crate) fn replay_durable_submission(
        &self,
        session: &RuntimeSession,
        prompt: &PromptQueueItem,
    ) -> Result<Option<PromptSubmissionOutcome>, DaemonError> {
        let Some(operation_id) = prompt.durable_operation_id() else {
            return Ok(None);
        };
        let fingerprint =
            prompt
                .durable_operation_fingerprint()
                .ok_or_else(|| DaemonError::LocalTransport {
                    operation: "replay durable prompt submission",
                    message: format!(
                        "prompt operation `{operation_id}` is missing its request fingerprint"
                    ),
                })?;
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .submission_for_durable_operation(session.id(), operation_id, fingerprint, prompt)
    }

    pub(crate) fn active_prompt_for_agent(
        &self,
        session: &RuntimeSession,
        agent_id: &str,
    ) -> Option<PromptQueueItem> {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .ensure_agent_state(session, agent_id)
            .active_prompt
            .clone()
    }

    pub(crate) fn active_prompt_for_agent_snapshot(
        &self,
        session: &RuntimeSession,
        agent_id: &str,
    ) -> Option<PromptQueueItem> {
        let key = PromptStateKey::new(session.id(), agent_id);
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .states
            .get(&key)
            .and_then(|state| state.active_prompt.clone())
    }

    pub(crate) fn active_prompt_agent_id(&self, session: &RuntimeSession) -> Option<String> {
        let owner = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(focused_agent_id) = session.focused_agent_id() {
            if owner
                .states
                .get(&PromptStateKey::new(session.id(), focused_agent_id))
                .and_then(|state| state.active_prompt.as_ref())
                .is_some()
            {
                return Some(focused_agent_id.to_string());
            }
        }

        let active_agents = owner
            .agent_ids_for_session(session)
            .into_iter()
            .filter(|agent_id| {
                owner
                    .states
                    .get(&PromptStateKey::new(session.id(), agent_id))
                    .and_then(|state| state.active_prompt.as_ref())
                    .is_some()
            })
            .collect::<Vec<_>>();
        if active_agents.len() == 1 {
            active_agents.into_iter().next()
        } else {
            None
        }
    }

    pub(crate) fn has_any_active_prompt(&self, session: &RuntimeSession) -> bool {
        let owner = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        owner
            .agent_ids_for_session(session)
            .into_iter()
            .any(|agent_id| {
                owner
                    .states
                    .get(&PromptStateKey::new(session.id(), &agent_id))
                    .and_then(|state| state.active_prompt.as_ref())
                    .is_some()
            })
    }

    pub(crate) fn active_prompt_agent_ids(&self, session: &RuntimeSession) -> Vec<String> {
        let owner = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        owner
            .agent_ids_for_session(session)
            .into_iter()
            .filter(|agent_id| {
                owner
                    .states
                    .get(&PromptStateKey::new(session.id(), agent_id))
                    .and_then(|state| state.active_prompt.as_ref())
                    .is_some()
            })
            .collect()
    }

    pub(crate) fn queued_prompt_count_for_agent(
        &self,
        session: &RuntimeSession,
        agent_id: &str,
    ) -> usize {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .ensure_agent_state(session, agent_id)
            .queued_prompts
            .len()
    }

    pub(crate) fn submit_prepared_prompt(
        &self,
        session: &RuntimeSession,
        prompt: PromptQueueItem,
        force_queue: bool,
    ) -> Result<PromptSubmissionOutcome, DaemonError> {
        self.submit_prepared_prompt_with_queue_policy(session, prompt, force_queue, true)
    }

    pub(crate) fn submit_prepared_prompt_with_queue_policy(
        &self,
        session: &RuntimeSession,
        mut prompt: PromptQueueItem,
        force_queue: bool,
        allow_queue: bool,
    ) -> Result<PromptSubmissionOutcome, DaemonError> {
        let agent_id = prompt.target_agent_id().to_string();
        let mut owner = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let (Some(operation_id), Some(fingerprint)) = (
            prompt.durable_operation_id(),
            prompt.durable_operation_fingerprint(),
        ) {
            if let Some(outcome) = owner.submission_for_durable_operation(
                session.id(),
                operation_id,
                fingerprint,
                &prompt,
            )? {
                return Ok(outcome);
            }
        }
        let profile_transition_pending = owner
            .profile_transitions
            .contains_key(&PromptStateKey::new(session.id(), &agent_id));
        let should_start = {
            let state = owner.ensure_agent_state(session, &agent_id);
            !force_queue
                && !profile_transition_pending
                && state.active_prompt.is_none()
                && state.sudo_admits(prompt.id())
        };
        if should_start {
            let state = owner.ensure_agent_state(session, &agent_id);
            prompt.set_durable_initially_queued(false);
            prompt.set_durable_delivery(
                crate::session::DurablePromptDeliveryPhase::Accepted,
                None,
                None,
            );
            prompt.set_status(PromptStatus::Running);
            state.set_active_prompt(Some(prompt.clone()));
            state.bind_sudo_work();
            Ok(PromptSubmissionOutcome::Started { prompt })
        } else {
            if !allow_queue {
                return Err(DaemonError::LocalTransport {
                    operation: "send agent message",
                    message: "target agent is busy; retry when its provider is ready".to_string(),
                });
            }
            let pending_prompt_id = if prompt.durable_operation_id().is_some() {
                prompt.id().to_string()
            } else {
                owner.next_pending_prompt_id()
            };
            let state = owner.ensure_agent_state(session, &agent_id);
            if state.queued_prompts.len() >= PROMPT_QUEUE_LIMIT {
                crate::logging::warn_with_fields(
                    "daemon.prompt_queue",
                    "agent prompt queue overloaded",
                    serde_json::json!({
                        "session_id": session.id(),
                        "agent_id": agent_id,
                        "prompt_id": prompt.id(),
                        "queued_prompts": state.queued_prompts.len(),
                        "queue_limit": PROMPT_QUEUE_LIMIT,
                    }),
                );
                return Err(DaemonError::LocalTransport {
                    operation: "queue prompt",
                    message: format!(
                        "agent prompt queue overloaded: queued prompt limit {PROMPT_QUEUE_LIMIT} reached"
                    ),
                });
            }
            prompt.set_durable_initially_queued(true);
            prompt.set_durable_delivery(
                crate::session::DurablePromptDeliveryPhase::Accepted,
                None,
                None,
            );
            prompt = prompt.into_pending_queue_item(pending_prompt_id);
            if prompt.workflow_run_id().is_some() {
                state.queued_prompts.push_back(prompt.clone());
            } else {
                let insert_at = state
                    .queued_prompts
                    .iter()
                    .position(|queued| queued.workflow_run_id().is_some())
                    .unwrap_or(state.queued_prompts.len());
                state.queued_prompts.insert(insert_at, prompt.clone());
            }
            Ok(PromptSubmissionOutcome::Queued { prompt })
        }
    }

    pub(crate) fn complete_active_prompt_only(
        &self,
        session: &RuntimeSession,
        agent_id: &str,
    ) -> Option<PromptQueueItem> {
        self.complete_active_prompt_if_matches(session, agent_id, None)
    }

    /// Keep prompt replacement fenced while a synchronous, authorized operation uses it.
    pub(crate) fn with_current_prompt<R>(
        &self,
        session: &RuntimeSession,
        agent_id: &str,
        expected_prompt_id: Option<&str>,
        operation: impl FnOnce() -> Result<R, crate::error::DaemonError>,
    ) -> Result<R, crate::error::DaemonError> {
        let mut owner = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let state = owner.ensure_agent_state(session, agent_id);
        if state.active_prompt.as_ref().map(PromptQueueItem::id) != expected_prompt_id
            || state
                .active_prompt
                .as_ref()
                .is_some_and(|prompt| prompt.status() != PromptStatus::Running)
        {
            return Err(crate::error::DaemonError::LocalTransport {
                operation: "revalidate forwarded prompt",
                message: "forwarded prompt changed before resource use".into(),
            });
        }
        operation()
    }

    /// Run a synchronous ownership transition while the active prompt cannot change.
    pub(crate) fn with_active_prompt<R>(
        &self,
        session: &RuntimeSession,
        agent_id: &str,
        operation: impl FnOnce(Option<&PromptQueueItem>) -> Result<R, DaemonError>,
    ) -> Result<R, DaemonError> {
        let mut owner = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        operation(
            owner
                .ensure_agent_state(session, agent_id)
                .active_prompt
                .as_ref(),
        )
    }

    pub(crate) fn complete_active_prompt_if_matches(
        &self,
        session: &RuntimeSession,
        agent_id: &str,
        expected_prompt_id: Option<&str>,
    ) -> Option<PromptQueueItem> {
        let mut owner = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let state = owner.ensure_agent_state(session, agent_id);
        if expected_prompt_id.is_some_and(|expected_prompt_id| {
            state.active_prompt.as_ref().map(PromptQueueItem::id) != Some(expected_prompt_id)
        }) {
            return None;
        }
        let mut completed = state.take_active_prompt()?;
        completed.set_status(PromptStatus::Completed);
        Some(completed)
    }

    pub(crate) fn cancel_active_prompt_only(
        &self,
        session: &RuntimeSession,
        agent_id: &str,
    ) -> Option<PromptQueueItem> {
        let mut owner = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let state = owner.ensure_agent_state(session, agent_id);
        let mut cancelled = state.take_active_prompt()?;
        cancelled.set_status(PromptStatus::Cancelled);
        Some(cancelled)
    }

    pub(crate) fn bind_sudo_turn(
        &self,
        session: &RuntimeSession,
        agent: &str,
        prompt: &str,
        entry: &str,
    ) -> bool {
        let mut owner = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let state = owner.ensure_agent_state(session, agent);
        if !state
            .active_prompt
            .as_ref()
            .is_some_and(|active| active.id() == prompt && active.status() == PromptStatus::Running)
        {
            return false;
        }
        if state.sudo_entry_id.as_deref() != Some(entry) {
            state.sudo_process_cutoff = crate::runtime::kernel_access::process::birth_cutoff().ok();
        }
        state.sudo_entry_id = Some(entry.into());
        true
    }

    pub(crate) fn prompt_is_sudo_bound(
        &self,
        session: &RuntimeSession,
        agent: &str,
        prompt: &str,
    ) -> bool {
        let mut owner = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let state = owner.ensure_agent_state(session, agent);
        state.sudo_entry_id.is_some()
            && state
                .active_prompt
                .as_ref()
                .is_some_and(|active| active.id() == prompt)
    }

    /// The running prompt bound to a sudo window, as `(entry_id, prompt_id)`.
    pub(crate) fn sudo_bound_prompt(
        &self,
        session: &RuntimeSession,
        agent: &str,
    ) -> Option<(String, String)> {
        let mut owner = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let state = owner.ensure_agent_state(session, agent);
        let entry = state.sudo_entry_id.clone()?;
        state
            .active_prompt
            .as_ref()
            .filter(|active| active.status() == PromptStatus::Running)
            .map(|active| (entry, active.id().to_owned()))
    }

    /// Returns the cutoff only for the exact running prompt/window binding.
    pub(crate) fn sudo_bound_process_cutoff(
        &self,
        session: &RuntimeSession,
        agent: &str,
        entry: &str,
        prompt: &str,
    ) -> Option<u64> {
        let mut owner = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let state = owner.ensure_agent_state(session, agent);
        if state.sudo_entry_id.as_deref() != Some(entry)
            || !state.active_prompt.as_ref().is_some_and(|active| {
                active.id() == prompt && active.status() == PromptStatus::Running
            })
        {
            return None;
        }
        state.sudo_process_cutoff
    }

    /// Hold `agent` for one sudo window's work; `prompt` is its first turn.
    pub(crate) fn hold_sudo_work(
        &self,
        session: &RuntimeSession,
        agent: &str,
        entry: &str,
        prompt: &str,
    ) {
        let mut owner = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        owner.ensure_agent_state(session, agent).sudo_work = Some(SudoWorkHold {
            entry_id: entry.into(),
            prompts: BTreeSet::from([prompt.to_owned()]),
        });
    }

    /// Admit a kernel-correlated continuation of the held work.
    pub(crate) fn admit_sudo_work_prompt(
        &self,
        session: &RuntimeSession,
        agent: &str,
        entry: &str,
        prompt: &str,
    ) -> bool {
        let mut owner = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let Some(hold) = owner
            .ensure_agent_state(session, agent)
            .sudo_work
            .as_mut()
            .filter(|hold| hold.entry_id == entry)
        else {
            return false;
        };
        hold.prompts.insert(prompt.into());
        true
    }

    pub(crate) fn sudo_work_held(&self, session: &RuntimeSession, agent: &str) -> bool {
        let mut owner = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        owner.ensure_agent_state(session, agent).sudo_work.is_some()
    }

    /// End the window's hold and any binding; regular work keeps running.
    pub(crate) fn release_sudo_work(&self, session: &RuntimeSession, agent: &str, entry: &str) {
        let mut owner = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let state = owner.ensure_agent_state(session, agent);
        if state
            .sudo_work
            .as_ref()
            .is_some_and(|hold| hold.entry_id == entry)
        {
            state.sudo_work = None;
        }
        if state.sudo_entry_id.as_deref() == Some(entry) {
            state.sudo_entry_id = None;
            state.sudo_process_cutoff = None;
        }
    }

    pub(crate) fn begin_cancelling_active_prompt(
        &self,
        session: &RuntimeSession,
        agent_id: &str,
    ) -> Option<PromptQueueItem> {
        self.begin_cancelling_prompt_if_matches(session, agent_id, None)
    }

    pub(crate) fn begin_cancelling_prompt_if_matches(
        &self,
        session: &RuntimeSession,
        agent_id: &str,
        expected: Option<&str>,
    ) -> Option<PromptQueueItem> {
        let mut owner = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let state = owner.ensure_agent_state(session, agent_id);
        let active = state.active_prompt.as_mut()?;
        if expected.is_some_and(|id| active.id() != id) {
            return None;
        }
        state.sudo_entry_id = None;
        state.sudo_process_cutoff = None;
        active.set_status(PromptStatus::Cancelling);
        Some(active.clone())
    }

    pub(crate) fn mark_active_prompt_running(
        &self,
        session: &RuntimeSession,
        agent_id: &str,
    ) -> Option<PromptQueueItem> {
        let mut owner = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let active = owner
            .ensure_agent_state(session, agent_id)
            .active_prompt
            .as_mut()?;
        active.set_status(PromptStatus::Running);
        Some(active.clone())
    }

    pub(crate) fn mark_active_prompt_delivery(
        &self,
        session: &RuntimeSession,
        agent_id: &str,
        prompt_id: &str,
        phase: crate::session::DurablePromptDeliveryPhase,
        provider_run_id: Option<String>,
        provider_session_id: Option<String>,
    ) -> Result<PromptQueueItem, DaemonError> {
        let mut owner = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let active = owner
            .ensure_agent_state(session, agent_id)
            .active_prompt
            .as_mut()
            .ok_or_else(|| DaemonError::NoActivePrompt {
                session_id: session.id().to_string(),
            })?;
        if active.id() != prompt_id {
            return Err(DaemonError::LocalTransport {
                operation: "mark prompt delivery",
                message: format!(
                    "active prompt `{}` does not match delivery prompt `{prompt_id}`",
                    active.id()
                ),
            });
        }
        if phase == crate::session::DurablePromptDeliveryPhase::Dispatching
            && active.status() == PromptStatus::Cancelling
        {
            if active.durable_delivery_phase()
                != Some(crate::session::DurablePromptDeliveryPhase::Dispatching)
            {
                return Err(DaemonError::LocalTransport {
                    operation: "mark prompt delivery",
                    message: format!(
                        "cancelling prompt `{prompt_id}` cannot start remote dispatch"
                    ),
                });
            }
            // The delivery phase already crossed the durable pre-send boundary. Preserve its
            // receipt identity so cancellation can reconcile the in-flight worker request.
            return Ok(active.clone());
        }
        active.set_durable_delivery(phase, provider_run_id, provider_session_id);
        if phase == crate::session::DurablePromptDeliveryPhase::Delivered
            && active.status() == PromptStatus::Dispatching
        {
            active.set_status(PromptStatus::Running);
        }
        Ok(active.clone())
    }

    pub(crate) fn replace_active_prompt_if_matches(
        &self,
        session: &RuntimeSession,
        agent_id: &str,
        expected: &PromptQueueItem,
        replacement: PromptQueueItem,
    ) -> bool {
        let mut owner = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let active = &mut owner.ensure_agent_state(session, agent_id).active_prompt;
        if active.as_ref() != Some(expected) {
            return false;
        }
        *active = Some(replacement);
        true
    }

    pub(crate) fn restore_reserved_prompt_if_unclaimed(
        &self,
        session: &RuntimeSession,
        agent_id: &str,
        prompt: PromptQueueItem,
        expected_queue: &VecDeque<PromptQueueItem>,
    ) -> bool {
        let mut owner = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let state = owner.ensure_agent_state(session, agent_id);
        if state.active_prompt.is_some() || &state.queued_prompts != expected_queue {
            return false;
        }
        state.set_active_prompt(Some(prompt));
        true
    }

    pub(crate) fn begin_active_prompt_recovery(
        &self,
        session: &RuntimeSession,
        agent_id: &str,
        prompt_id: &str,
    ) -> Result<PromptQueueItem, DaemonError> {
        let mut owner = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let active = owner
            .ensure_agent_state(session, agent_id)
            .active_prompt
            .as_mut()
            .ok_or_else(|| DaemonError::NoActivePrompt {
                session_id: session.id().to_string(),
            })?;
        if active.id() != prompt_id {
            return Err(DaemonError::LocalTransport {
                operation: "begin prompt recovery",
                message: format!(
                    "active prompt `{}` does not match recovery prompt `{prompt_id}`",
                    active.id()
                ),
            });
        }
        active.begin_durable_recovery_operation();
        Ok(active.clone())
    }

    pub(crate) fn mark_active_prompt_recovery_phase(
        &self,
        session: &RuntimeSession,
        agent_id: &str,
        prompt_id: &str,
        operation_id: &str,
        phase: crate::session::DurablePromptDeliveryPhase,
    ) -> Result<PromptQueueItem, DaemonError> {
        let mut owner = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let active = owner
            .ensure_agent_state(session, agent_id)
            .active_prompt
            .as_mut()
            .ok_or_else(|| DaemonError::NoActivePrompt {
                session_id: session.id().to_string(),
            })?;
        if active.id() != prompt_id || !active.mark_durable_recovery_phase(operation_id, phase) {
            return Err(DaemonError::LocalTransport {
                operation: "mark prompt recovery",
                message: format!(
                    "active prompt `{}` does not match recovery operation `{operation_id}` for prompt `{prompt_id}`",
                    active.id()
                ),
            });
        }
        Ok(active.clone())
    }

    pub(crate) fn compare_and_mark_active_prompt_recovery_phase(
        &self,
        session: &RuntimeSession,
        agent_id: &str,
        prompt_id: &str,
        operation_id: &str,
        expected_phase: crate::session::DurablePromptDeliveryPhase,
        next_phase: crate::session::DurablePromptDeliveryPhase,
    ) -> Result<Option<PromptQueueItem>, DaemonError> {
        let mut owner = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let active = owner
            .ensure_agent_state(session, agent_id)
            .active_prompt
            .as_mut()
            .ok_or_else(|| DaemonError::NoActivePrompt {
                session_id: session.id().to_string(),
            })?;
        if active.id() != prompt_id
            || active.durable_recovery_operation_id() != Some(operation_id)
            || active.durable_recovery_phase() != Some(expected_phase)
        {
            return Ok(None);
        }
        if !active.mark_durable_recovery_phase(operation_id, next_phase) {
            return Ok(None);
        }
        Ok(Some(active.clone()))
    }

    pub(crate) fn finalize_active_prompt_cancellation(
        &self,
        session: &RuntimeSession,
        agent_id: &str,
    ) -> Option<PromptQueueItem> {
        let mut owner = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let state = owner.ensure_agent_state(session, agent_id);
        let active_status = state.active_prompt.as_ref()?.status();
        if active_status != PromptStatus::Cancelling {
            return None;
        }
        let mut cancelled = state.take_active_prompt()?;
        cancelled.set_status(PromptStatus::Cancelled);
        Some(cancelled)
    }

    pub(crate) fn peek_next_queued_prompt(
        &self,
        session: &RuntimeSession,
        agent_id: &str,
    ) -> Option<PromptQueueItem> {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .ensure_agent_state(session, agent_id)
            .queued_prompts
            .front()
            .filter(|prompt| !prompt.remote_steer_reserved())
            .cloned()
    }

    pub(crate) fn reserve_queued_prompt_remote_steer(
        &self,
        session: &RuntimeSession,
        agent_id: &str,
        expected_active_prompt_id: &str,
        expected_prompt: &PromptQueueItem,
    ) -> Result<u64, DaemonError> {
        self.reserve_queued_prompt_remote_steer_with_hook(
            session,
            agent_id,
            expected_active_prompt_id,
            expected_prompt,
            || {},
        )
    }

    fn reserve_queued_prompt_remote_steer_with_hook<F>(
        &self,
        session: &RuntimeSession,
        agent_id: &str,
        expected_active_prompt_id: &str,
        expected_prompt: &PromptQueueItem,
        before_reserve: F,
    ) -> Result<u64, DaemonError>
    where
        F: FnOnce(),
    {
        let mut owner = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let state = owner.ensure_agent_state(session, agent_id);
        let active_prompt =
            state
                .active_prompt
                .as_ref()
                .ok_or_else(|| DaemonError::NoActivePrompt {
                    session_id: session.id().to_string(),
                })?;
        if active_prompt.id() != expected_active_prompt_id
            || active_prompt.status() != PromptStatus::Running
        {
            return Err(DaemonError::LocalTransport {
                operation: "steer remote queued prompt",
                message: "active prompt changed before queued steer reservation".to_string(),
            });
        }
        let queued_prompt = state
            .queued_prompts
            .iter_mut()
            .find(|prompt| prompt.id() == expected_prompt.id())
            .ok_or_else(|| DaemonError::LocalTransport {
                operation: "steer remote queued prompt",
                message: format!(
                    "queued prompt `{}` disappeared before steer reservation",
                    expected_prompt.id()
                ),
            })?;
        if &*queued_prompt != expected_prompt || queued_prompt.target_agent_id() != agent_id {
            return Err(DaemonError::LocalTransport {
                operation: "steer remote queued prompt",
                message: format!(
                    "queued prompt `{}` changed before steer reservation",
                    expected_prompt.id()
                ),
            });
        }

        // The reservation and every ordinary activation share this owner lock.
        // A completion cannot observe an unreserved clone and pop it in between.
        before_reserve();
        queued_prompt
            .reserve_remote_steer()
            .ok_or_else(|| DaemonError::LocalTransport {
                operation: "steer remote queued prompt",
                message: format!(
                    "queued prompt `{}` already has an in-flight remote steer",
                    expected_prompt.id()
                ),
            })
    }

    #[cfg(test)]
    pub(crate) fn activate_next_queued_prompt(
        &self,
        session: &RuntimeSession,
        agent_id: &str,
        expected_prompt_id: Option<&str>,
    ) -> Result<Option<PromptQueueItem>, DaemonError> {
        let mut owner = self
            .state
            .lock()
            .expect("prompt state owner lock should not be poisoned");
        let state = owner.ensure_agent_state(session, agent_id);
        if let Some(active_prompt) = state.active_prompt.as_ref() {
            return Err(DaemonError::LocalTransport {
                operation: "activate queued prompt",
                message: format!(
                    "cannot activate queued prompt for agent `{agent_id}` while active prompt `{}` is still running",
                    active_prompt.id()
                ),
            });
        }
        let Some(front) = state.queued_prompts.front() else {
            return Ok(None);
        };
        if front.remote_steer_reserved() {
            return Ok(None);
        }
        if let Some(expected_prompt_id) = expected_prompt_id {
            if front.id() != expected_prompt_id {
                return Err(DaemonError::LocalTransport {
                    operation: "activate expected queued prompt",
                    message: format!(
                        "expected queued prompt `{}` but prompt owner queue front was `{}`",
                        expected_prompt_id,
                        front.id()
                    ),
                });
            }
        }
        validate_prompt_target_agent("activate queued prompt", agent_id, front)?;
        let mut active = state
            .queued_prompts
            .pop_front()
            .expect("queue front checked above");
        active.set_status(PromptStatus::Running);
        state.set_active_prompt(Some(active.clone()));
        Ok(Some(active))
    }

    pub(crate) fn activate_next_queued_prompt_with_prompt_id(
        &self,
        session: &RuntimeSession,
        agent_id: &str,
        expected_prompt_id: Option<&str>,
        prompt_id: String,
    ) -> Result<Option<PromptQueueItem>, DaemonError> {
        let mut owner = self
            .state
            .lock()
            .expect("prompt state owner lock should not be poisoned");
        if owner
            .profile_transitions
            .contains_key(&PromptStateKey::new(session.id(), agent_id))
        {
            return Ok(None);
        }
        let state = owner.ensure_agent_state(session, agent_id);
        Self::activate_owned_queued_prompt(state, agent_id, expected_prompt_id, prompt_id)
    }

    /// Opportunistic queue dispatch claims only the head it observed, while idle.
    /// Preparation runs only for the eligible head, under this lock; it must
    /// not reenter the prompt owner. This keeps workspace claims with the winner.
    /// Another admission or completion may win between the caller's peek and
    /// this lock; that is deferred work, not a provider initialization failure.
    pub(crate) fn try_activate_next_queued_prompt_with_prompt_id(
        &self,
        session: &RuntimeSession,
        agent_id: &str,
        expected_prompt_id: &str,
        prompt_id: String,
        prepare: impl FnOnce(&PromptQueueItem) -> Result<(), DaemonError>,
    ) -> Result<Option<PromptQueueItem>, DaemonError> {
        let mut owner = self
            .state
            .lock()
            .expect("prompt state owner lock should not be poisoned");
        if owner
            .profile_transitions
            .contains_key(&PromptStateKey::new(session.id(), agent_id))
        {
            return Ok(None);
        }
        let state = owner.ensure_agent_state(session, agent_id);
        if state.active_prompt.is_some()
            || state.queued_prompts.front().is_none_or(|front| {
                front.id() != expected_prompt_id
                    || front.remote_steer_reserved()
                    || !state.sudo_admits(front.id())
            })
        {
            return Ok(None);
        }
        let front = state
            .queued_prompts
            .front()
            .expect("eligible head checked above");
        validate_prompt_target_agent("activate queued prompt", agent_id, front)?;
        prepare(front)?;
        Self::activate_owned_queued_prompt(state, agent_id, Some(expected_prompt_id), prompt_id)
    }

    fn activate_owned_queued_prompt(
        state: &mut OwnedAgentPromptState,
        agent_id: &str,
        expected_prompt_id: Option<&str>,
        prompt_id: String,
    ) -> Result<Option<PromptQueueItem>, DaemonError> {
        if let Some(active_prompt) = state.active_prompt.as_ref() {
            return Err(DaemonError::LocalTransport {
                operation: "activate queued prompt",
                message: format!(
                    "cannot activate queued prompt for agent `{agent_id}` while active prompt `{}` is still running",
                    active_prompt.id()
                ),
            });
        }
        let Some(front) = state.queued_prompts.front() else {
            return Ok(None);
        };
        if front.remote_steer_reserved() || !state.sudo_admits(front.id()) {
            return Ok(None);
        }
        if let Some(expected_prompt_id) = expected_prompt_id {
            if front.id() != expected_prompt_id {
                return Err(DaemonError::LocalTransport {
                    operation: "activate expected queued prompt",
                    message: format!(
                        "expected queued prompt `{}` but prompt owner queue front was `{}`",
                        expected_prompt_id,
                        front.id()
                    ),
                });
            }
        }
        validate_prompt_target_agent("activate queued prompt", agent_id, front)?;
        let prompt_id = if front.durable_operation_id().is_some() {
            front.id().to_string()
        } else {
            prompt_id
        };
        let mut active = state
            .queued_prompts
            .pop_front()
            .expect("queue front checked above")
            .with_id(prompt_id);
        active.set_status(PromptStatus::Dispatching);
        state.set_active_prompt(Some(active.clone()));
        state.bind_sudo_work();
        Ok(Some(active))
    }

    #[cfg(test)]
    pub(crate) fn activate_prompt(
        &self,
        session: &RuntimeSession,
        mut prompt: PromptQueueItem,
    ) -> Result<PromptQueueItem, DaemonError> {
        let agent_id = prompt.target_agent_id().to_string();
        let mut owner = self
            .state
            .lock()
            .expect("prompt state owner lock should not be poisoned");
        let state = owner.ensure_agent_state(session, &agent_id);
        if let Some(active_prompt) = state.active_prompt.as_ref() {
            if active_prompt.id() != prompt.id() {
                return Err(DaemonError::LocalTransport {
                    operation: "activate prompt",
                    message: format!(
                        "cannot activate prompt `{}` for agent `{agent_id}` while active prompt `{}` is still running",
                        prompt.id(),
                        active_prompt.id()
                    ),
                });
            }
        }
        state
            .queued_prompts
            .retain(|queued| queued.id() != prompt.id());
        prompt.set_status(PromptStatus::Running);
        state.set_active_prompt(Some(prompt.clone()));
        Ok(prompt)
    }

    pub(crate) fn sync_external_active_prompt(
        &self,
        session: &RuntimeSession,
        agent_id: &str,
        active_prompt: Option<PromptQueueItem>,
    ) -> bool {
        let mut owner = self
            .state
            .lock()
            .expect("prompt state owner lock should not be poisoned");
        let state = owner.ensure_agent_state(session, agent_id);
        match active_prompt {
            Some(mut prompt) => {
                if prompt.target_agent_id() != agent_id {
                    crate::logging::warn_with_fields(
                        "daemon.prompt_state",
                        "ignored external active prompt with mismatched target agent",
                        serde_json::json!({
                            "session_id": session.id(),
                            "agent_id": agent_id,
                            "prompt_id": prompt.id(),
                            "prompt_target_agent_id": prompt.target_agent_id(),
                        }),
                    );
                    return false;
                }
                if state
                    .active_prompt
                    .as_ref()
                    .is_some_and(|active| active.is_chariox_owned())
                {
                    return false;
                }
                prompt.set_status(PromptStatus::Running);
                if state.active_prompt.as_ref() == Some(&prompt) {
                    return false;
                }
                state.set_active_prompt(Some(prompt));
                true
            }
            None => {
                if state
                    .active_prompt
                    .as_ref()
                    .is_some_and(|active| active.is_external())
                {
                    state.set_active_prompt(None);
                    return true;
                }
                false
            }
        }
    }

    pub(crate) fn remove_queued_prompts_by_workflow_run(
        &self,
        session: &RuntimeSession,
        workflow_run_id: &str,
    ) -> usize {
        self.remove_queued_prompts_matching(session, |prompt| {
            prompt.workflow_run_id() == Some(workflow_run_id)
        })
    }

    pub(crate) fn remove_queued_prompts_for_agent(
        &self,
        session: &RuntimeSession,
        agent_id: &str,
    ) -> usize {
        let mut owner = self
            .state
            .lock()
            .expect("prompt state owner lock should not be poisoned");
        let state = owner.ensure_agent_state(session, agent_id);
        let removed = state.queued_prompts.len();
        state.queued_prompts.clear();
        removed
    }

    pub(crate) fn remove_queued_metaagent_event_prompts_for_agent(
        &self,
        session: &RuntimeSession,
        agent_id: &str,
        source_attachment_id: &str,
    ) -> usize {
        self.remove_queued_prompts_matching(session, |prompt| {
            prompt.target_agent_id() == agent_id
                && prompt.source_attachment_id() == source_attachment_id
                && prompt.prompt().trim()
                    == crate::scheduler::prompt_injection::METAAGENT_EVENT_VISIBLE_PROMPT
        })
    }

    pub(crate) fn remove_queued_prompt(
        &self,
        session: &RuntimeSession,
        agent_id: &str,
        prompt_id: &str,
    ) -> Option<PromptQueueItem> {
        let mut owner = self
            .state
            .lock()
            .expect("prompt state owner lock should not be poisoned");
        let state = owner.ensure_agent_state(session, agent_id);
        let index = state
            .queued_prompts
            .iter()
            .position(|prompt| prompt.id() == prompt_id)?;
        state.queued_prompts.remove(index)
    }

    pub(crate) fn update_queued_prompt(
        &self,
        session: &RuntimeSession,
        agent_id: &str,
        prompt_id: &str,
        prompt: impl Into<String>,
        editor: &crate::attachment::RuntimeAttachment,
    ) -> Option<PromptQueueItem> {
        let mut owner = self
            .state
            .lock()
            .expect("prompt state owner lock should not be poisoned");
        let state = owner.ensure_agent_state(session, agent_id);
        let queued = state
            .queued_prompts
            .iter_mut()
            .find(|queued| queued.id() == prompt_id)?;
        queued.set_prompt(prompt);
        // MP-08/MP-11 R1: text and acquisition provenance change under one
        // queue lock. An automated edit cannot keep a human owner's request.
        *queued = queued.clone().with_source_attachment(editor);
        Some(queued.clone())
    }

    pub(crate) fn state_parts(
        &self,
        session: &RuntimeSession,
        agent_id: &str,
    ) -> (Option<PromptQueueItem>, VecDeque<PromptQueueItem>) {
        let mut owner = self
            .state
            .lock()
            .expect("prompt state owner lock should not be poisoned");
        let state = owner.ensure_agent_state(session, agent_id);
        (state.active_prompt.clone(), state.queued_prompts.clone())
    }

    pub(crate) fn project_into_session(&self, session: &mut RuntimeSession) {
        let projected_states = {
            let owner = self
                .state
                .lock()
                .expect("prompt state owner lock should not be poisoned");
            owner
                .agent_ids_for_session(session)
                .into_iter()
                .map(|agent_id| {
                    let state = owner
                        .states
                        .get(&PromptStateKey::new(session.id(), &agent_id))
                        .cloned()
                        .unwrap_or_default();
                    (agent_id, state.active_prompt, state.queued_prompts)
                })
                .collect::<Vec<_>>()
        };
        for (agent_id, active_prompt, queued_prompts) in projected_states {
            session.mirror_agent_prompt_state(&agent_id, active_prompt, queued_prompts);
        }
    }

    pub(crate) fn restore_session_state(&self, session: &RuntimeSession) {
        let mut owner = self
            .state
            .lock()
            .expect("prompt state owner lock should not be poisoned");
        let restored_agent_ids = session
            .prompt_states()
            .keys()
            .cloned()
            .collect::<std::collections::BTreeSet<_>>();
        owner.states.retain(|key, state| {
            if key.session_id != session.id() || restored_agent_ids.contains(&key.agent_id) {
                return true;
            }
            // Keep only an idle agent's sudo hold; the mirror has no prompts.
            state.sudo_entry_id = None;
            state.sudo_process_cutoff = None;
            state.active_prompt = None;
            state.queued_prompts.clear();
            state.sudo_work.is_some()
        });
        for agent_id in session.prompt_states().keys() {
            let mut restored = OwnedAgentPromptState::from_session(session, agent_id);
            // A live sudo hold is kernel memory, not mirror state: keep the fence.
            if let Some(previous) = owner
                .states
                .get(&PromptStateKey::new(session.id(), agent_id))
            {
                restored.sudo_work = previous.sudo_work.clone();
                if previous.active_prompt.as_ref().map(PromptQueueItem::id)
                    == restored.active_prompt.as_ref().map(PromptQueueItem::id)
                {
                    restored.sudo_entry_id = previous.sudo_entry_id.clone();
                    restored.sudo_process_cutoff = previous.sudo_process_cutoff;
                }
            }
            if restored.active_prompt.is_none()
                && restored.queued_prompts.is_empty()
                && restored.sudo_work.is_none()
            {
                owner
                    .states
                    .remove(&PromptStateKey::new(session.id(), agent_id));
            } else {
                owner
                    .states
                    .insert(PromptStateKey::new(session.id(), agent_id), restored);
            }
        }
    }

    pub(crate) fn remove_session(&self, session_id: &str) {
        self.state
            .lock()
            .expect("prompt state owner lock should not be poisoned")
            .states
            .retain(|key, _| key.session_id.as_str() != session_id);
    }

    pub(crate) fn remove_agent(&self, session_id: &str, agent_id: &str) {
        self.state
            .lock()
            .expect("prompt state owner lock should not be poisoned")
            .states
            .remove(&PromptStateKey::new(session_id, agent_id));
    }

    fn remove_queued_prompts_matching(
        &self,
        session: &RuntimeSession,
        mut should_remove: impl FnMut(&PromptQueueItem) -> bool,
    ) -> usize {
        let mut owner = self
            .state
            .lock()
            .expect("prompt state owner lock should not be poisoned");
        let mut agent_ids = session
            .agents()
            .iter()
            .map(|agent| agent.id().to_string())
            .collect::<Vec<_>>();
        agent_ids.extend(session.prompt_states().keys().cloned());
        agent_ids.sort();
        agent_ids.dedup();

        let mut removed = 0;
        for agent_id in agent_ids {
            let state = owner.ensure_agent_state(session, &agent_id);
            let original_len = state.queued_prompts.len();
            state.queued_prompts.retain(|prompt| !should_remove(prompt));
            removed += original_len - state.queued_prompts.len();
        }
        removed
    }
}

fn validate_prompt_target_agent(
    operation: &'static str,
    agent_id: &str,
    prompt: &PromptQueueItem,
) -> Result<(), DaemonError> {
    if prompt.target_agent_id() == agent_id {
        return Ok(());
    }
    Err(DaemonError::LocalTransport {
        operation,
        message: format!(
            "prompt `{}` targets agent `{}` but is stored under agent `{agent_id}`",
            prompt.id(),
            prompt.target_agent_id()
        ),
    })
}

impl PromptStateOwnerState {
    fn submission_for_durable_operation(
        &self,
        session_id: &str,
        operation_id: &str,
        fingerprint: &str,
        requested: &PromptQueueItem,
    ) -> Result<Option<PromptSubmissionOutcome>, DaemonError> {
        let prompt = self
            .states
            .iter()
            .filter(|(key, _)| key.session_id == session_id)
            .flat_map(|(_, state)| {
                state
                    .active_prompt
                    .iter()
                    .chain(state.queued_prompts.iter())
            })
            .find(|prompt| {
                prompt.durable_operation_id() == Some(operation_id)
                    && (!operation_id.starts_with(
                        crate::durable_state::workflow_dispatch_intents::OPERATION_PREFIX,
                    ) || (prompt.workflow_run_id() == requested.workflow_run_id()
                        && prompt.workflow_node_run_id() == requested.workflow_node_run_id()
                        && prompt.target_agent_id() == requested.target_agent_id()))
            });
        let Some(prompt) = prompt else {
            return Ok(None);
        };
        if prompt.durable_operation_fingerprint() != Some(fingerprint) {
            return Err(DaemonError::LocalTransport {
                operation: "replay durable prompt submission",
                message: format!(
                    "operation id `{operation_id}` was already used for a different prompt request"
                ),
            });
        }
        let initially_queued = prompt.durable_initially_queued().unwrap_or_else(|| {
            prompt.status() == PromptStatus::Queued || prompt.pending_prompt_id().is_some()
        });
        Ok(Some(if initially_queued {
            PromptSubmissionOutcome::Queued {
                prompt: prompt.clone(),
            }
        } else {
            PromptSubmissionOutcome::Started {
                prompt: prompt.clone(),
            }
        }))
    }

    fn agent_ids_for_session(&self, session: &RuntimeSession) -> Vec<String> {
        let mut agent_ids = session
            .agents()
            .iter()
            .map(|agent| agent.id().to_string())
            .collect::<Vec<_>>();
        agent_ids.extend(session.prompt_states().keys().cloned());
        agent_ids.extend(
            self.states
                .keys()
                .filter(|key| key.session_id == session.id())
                .map(|key| key.agent_id.clone()),
        );
        agent_ids.sort();
        agent_ids.dedup();
        agent_ids
    }

    fn next_pending_prompt_id(&mut self) -> String {
        self.next_pending_prompt_number = self.next_pending_prompt_number.wrapping_add(1);
        format!(
            "pending-prompt-{:016x}",
            crate::session::unix_epoch_ms() ^ self.next_pending_prompt_number.rotate_left(17)
        )
    }

    fn ensure_agent_state(
        &mut self,
        session: &RuntimeSession,
        agent_id: &str,
    ) -> &mut OwnedAgentPromptState {
        let key = PromptStateKey::new(session.id(), agent_id);
        self.states.entry(key).or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_queue_admission_rejects_a_busy_target_without_mutation() {
        let owner = PromptStateOwner::default();
        let session = RuntimeSession::new(
            "session-direct-message",
            None,
            "workspace-direct-message",
            "worktree-direct-message",
            "machine-direct-message",
            "daemon-direct-message",
        );
        let active = PromptQueueItem::new(
            "active-prompt",
            "attachment-1",
            "agent-1",
            "active task",
            PromptStatus::Queued,
        );
        assert!(matches!(
            owner
                .submit_prepared_prompt(&session, active, false)
                .expect("first prompt should start"),
            PromptSubmissionOutcome::Started { .. }
        ));

        let message = PromptQueueItem::new(
            "agent-message",
            "attachment-2",
            "agent-1",
            "new information",
            PromptStatus::Queued,
        );
        assert!(owner
            .submit_prepared_prompt_with_queue_policy(&session, message, false, false)
            .is_err());
        assert_eq!(owner.queued_prompt_count_for_agent(&session, "agent-1"), 0);
        assert_eq!(
            owner
                .active_prompt_for_agent(&session, "agent-1")
                .expect("first prompt should remain active")
                .id(),
            "active-prompt"
        );
    }

    #[test]
    fn submit_prepared_prompt_rejects_queue_overflow() {
        let owner = PromptStateOwner::default();
        let session = RuntimeSession::new(
            "session-1",
            None,
            "workspace-1",
            "worktree-1",
            "machine-1",
            "daemon-1",
        );

        for index in 0..PROMPT_QUEUE_LIMIT {
            let outcome = owner
                .submit_prepared_prompt(
                    &session,
                    PromptQueueItem::new(
                        format!("prompt-{index}"),
                        "attachment-1",
                        "agent-1",
                        "queued",
                        PromptStatus::Queued,
                    ),
                    true,
                )
                .expect("prompt should fit while under queue limit");
            assert!(matches!(outcome, PromptSubmissionOutcome::Queued { .. }));
        }

        let error = owner
            .submit_prepared_prompt(
                &session,
                PromptQueueItem::new(
                    "prompt-overflow",
                    "attachment-1",
                    "agent-1",
                    "overflow",
                    PromptStatus::Queued,
                ),
                true,
            )
            .expect_err("queue limit should reject overflow prompt");

        assert!(error.to_string().contains("agent prompt queue overloaded"));
        assert_eq!(
            owner.queued_prompt_count_for_agent(&session, "agent-1"),
            PROMPT_QUEUE_LIMIT
        );
    }

    #[test]
    fn durable_submission_replay_does_not_duplicate_active_or_queued_prompts() {
        let owner = PromptStateOwner::default();
        let session = RuntimeSession::new(
            "session-durable",
            None,
            "workspace-1",
            "worktree-1",
            "machine-1",
            "daemon-1",
        );
        let first = PromptQueueItem::new(
            "prompt-active",
            "attachment-1",
            "agent-1",
            "active",
            PromptStatus::Queued,
        );
        owner
            .submit_prepared_prompt(&session, first, false)
            .expect("active prompt should submit");
        let durable = PromptQueueItem::new(
            "prompt-durable-draft",
            "attachment-1",
            "agent-1",
            "queued once",
            PromptStatus::Queued,
        )
        .with_durable_operation("command-1", "fingerprint-1");
        let queued = owner
            .submit_prepared_prompt(&session, durable.clone(), false)
            .expect("durable prompt should queue");
        let replayed = owner
            .submit_prepared_prompt(&session, durable, false)
            .expect("durable prompt should replay");

        let queued_id = match queued {
            PromptSubmissionOutcome::Queued { prompt } => prompt.id().to_string(),
            other => panic!("expected queued prompt, got {other:?}"),
        };
        match replayed {
            PromptSubmissionOutcome::Queued { prompt } => assert_eq!(prompt.id(), queued_id),
            other => panic!("expected replayed queued prompt, got {other:?}"),
        }
        assert_eq!(owner.queued_prompt_count_for_agent(&session, "agent-1"), 1);

        let conflict = PromptQueueItem::new(
            "prompt-conflict",
            "attachment-1",
            "agent-1",
            "different",
            PromptStatus::Queued,
        )
        .with_durable_operation("command-1", "fingerprint-2");
        assert!(owner
            .submit_prepared_prompt(&session, conflict, false)
            .expect_err("fingerprint drift should conflict")
            .to_string()
            .contains("already used for a different prompt request"));
    }

    #[test]
    fn active_prompt_delivery_phase_tracks_matching_provider_acknowledgement() {
        let owner = PromptStateOwner::default();
        let session = RuntimeSession::new(
            "session-delivery",
            None,
            "workspace-1",
            "worktree-1",
            "machine-1",
            "daemon-1",
        );
        let prompt = PromptQueueItem::new(
            "prompt-delivery",
            "attachment-1",
            "agent-1",
            "deliver once",
            PromptStatus::Queued,
        )
        .with_durable_operation("command-delivery", "fingerprint-delivery");
        let PromptSubmissionOutcome::Started { prompt } = owner
            .submit_prepared_prompt(&session, prompt, false)
            .expect("prompt should start")
        else {
            panic!("prompt should be active");
        };
        assert_eq!(
            prompt.durable_delivery_phase(),
            Some(crate::session::DurablePromptDeliveryPhase::Accepted)
        );

        let dispatching = owner
            .mark_active_prompt_delivery(
                &session,
                "agent-1",
                prompt.id(),
                crate::session::DurablePromptDeliveryPhase::Dispatching,
                Some("provider-run-1".to_string()),
                None,
            )
            .expect("dispatching phase should persist");
        assert_eq!(
            dispatching.durable_delivery_phase(),
            Some(crate::session::DurablePromptDeliveryPhase::Dispatching)
        );
        assert_eq!(
            dispatching.durable_delivery_provider_run_id(),
            Some("provider-run-1")
        );

        let delivered = owner
            .mark_active_prompt_delivery(
                &session,
                "agent-1",
                prompt.id(),
                crate::session::DurablePromptDeliveryPhase::Delivered,
                Some("provider-run-1".to_string()),
                Some("provider-session-1".to_string()),
            )
            .expect("delivery acknowledgement should persist");
        assert_eq!(
            delivered.durable_delivery_phase(),
            Some(crate::session::DurablePromptDeliveryPhase::Delivered)
        );
        assert_eq!(delivered.status(), PromptStatus::Running);
        assert_eq!(
            delivered.durable_delivery_provider_session_id(),
            Some("provider-session-1")
        );
        assert!(owner
            .mark_active_prompt_delivery(
                &session,
                "agent-1",
                "another-prompt",
                crate::session::DurablePromptDeliveryPhase::Delivered,
                None,
                None,
            )
            .is_err());
    }

    #[test]
    fn opportunistic_queue_claim_defers_busy_and_consumed_heads_without_losing_successors() {
        let owner = PromptStateOwner::default();
        let session = RuntimeSession::new(
            "session-1",
            None,
            "workspace-1",
            "worktree-1",
            "machine-1",
            "daemon-1",
        );
        let mut queued_ids = Vec::new();
        for (id, text) in [
            ("active", "active"),
            ("first", "first"),
            ("second", "second"),
        ] {
            let outcome = owner
                .submit_prepared_prompt(
                    &session,
                    PromptQueueItem::new(id, "attachment-1", "agent-1", text, PromptStatus::Queued),
                    false,
                )
                .expect("normal prompt admission");
            if let PromptSubmissionOutcome::Queued { prompt } = outcome {
                queued_ids.push(prompt.id().to_string());
            }
        }
        assert_eq!(queued_ids.len(), 2);
        assert!(owner
            .try_activate_next_queued_prompt_with_prompt_id(
                &session,
                "agent-1",
                &queued_ids[0],
                "busy-claim".to_string(),
                |_| Ok(()),
            )
            .expect("busy is ordinary contention")
            .is_none());
        owner
            .complete_active_prompt_only(&session, "agent-1")
            .expect("complete active turn");
        let first = owner
            .activate_next_queued_prompt_with_prompt_id(
                &session,
                "agent-1",
                Some(&queued_ids[0]),
                "rival-first".to_string(),
            )
            .expect("another dispatcher claims the first head")
            .expect("first queued turn");
        assert_eq!(first.prompt(), "first");
        owner
            .complete_active_prompt_only(&session, "agent-1")
            .expect("complete rival turn");
        assert!(owner
            .try_activate_next_queued_prompt_with_prompt_id(
                &session,
                "agent-1",
                &queued_ids[0],
                "stale-claim".to_string(),
                |_| Ok(()),
            )
            .expect("consumed head is ordinary contention")
            .is_none());
        assert_eq!(
            owner
                .peek_next_queued_prompt(&session, "agent-1")
                .unwrap()
                .id(),
            queued_ids[1]
        );
        let second = owner
            .try_activate_next_queued_prompt_with_prompt_id(
                &session,
                "agent-1",
                &queued_ids[1],
                "second-dispatch".to_string(),
                |_| Ok(()),
            )
            .expect("current idle head can progress")
            .expect("second queued turn");
        assert_eq!(second.prompt(), "second");
        assert!(owner.peek_next_queued_prompt(&session, "agent-1").is_none());
    }

    #[test]
    fn cancelling_accepted_prompt_cannot_transition_to_dispatching() {
        let owner = PromptStateOwner::default();
        let session = RuntimeSession::new(
            "session-cancel-before-dispatch",
            None,
            "workspace-1",
            "worktree-1",
            "machine-1",
            "daemon-1",
        );
        let PromptSubmissionOutcome::Started { prompt } = owner
            .submit_prepared_prompt(
                &session,
                PromptQueueItem::new(
                    "prompt-cancel-before-dispatch",
                    "attachment-1",
                    "agent-1",
                    "cancel before remote dispatch",
                    PromptStatus::Queued,
                ),
                false,
            )
            .expect("prompt should be admitted")
        else {
            panic!("prompt should start");
        };
        assert_eq!(
            prompt.durable_delivery_phase(),
            Some(crate::session::DurablePromptDeliveryPhase::Accepted)
        );
        owner
            .begin_cancelling_active_prompt(&session, "agent-1")
            .expect("accepted prompt should enter cancellation");

        let error = owner
            .mark_active_prompt_delivery(
                &session,
                "agent-1",
                prompt.id(),
                crate::session::DurablePromptDeliveryPhase::Dispatching,
                None,
                None,
            )
            .expect_err("cancellation must win before the dispatching boundary");

        assert!(error.to_string().contains("cannot start remote dispatch"));
        let active = owner
            .active_prompt_for_agent(&session, "agent-1")
            .expect("same prompt should remain active for cancellation");
        assert_eq!(active.id(), prompt.id());
        assert_eq!(active.status(), PromptStatus::Cancelling);
        assert_eq!(
            active.durable_delivery_phase(),
            Some(crate::session::DurablePromptDeliveryPhase::Accepted)
        );
    }

    #[test]
    fn cancelling_dispatching_prompt_keeps_receipt_identity() {
        let owner = PromptStateOwner::default();
        let session = RuntimeSession::new(
            "session-cancel-during-dispatch",
            None,
            "workspace-1",
            "worktree-1",
            "machine-1",
            "daemon-1",
        );
        let PromptSubmissionOutcome::Started { prompt } = owner
            .submit_prepared_prompt(
                &session,
                PromptQueueItem::new(
                    "prompt-cancel-during-dispatch",
                    "attachment-1",
                    "agent-1",
                    "cancel while remote dispatch is in flight",
                    PromptStatus::Queued,
                ),
                false,
            )
            .expect("prompt should be admitted")
        else {
            panic!("prompt should start");
        };
        owner
            .mark_active_prompt_delivery(
                &session,
                "agent-1",
                prompt.id(),
                crate::session::DurablePromptDeliveryPhase::Dispatching,
                Some("worker-run-1".to_string()),
                None,
            )
            .expect("dispatching phase should persist");
        owner
            .begin_cancelling_active_prompt(&session, "agent-1")
            .expect("dispatching prompt should enter cancellation");

        let active = owner
            .mark_active_prompt_delivery(
                &session,
                "agent-1",
                prompt.id(),
                crate::session::DurablePromptDeliveryPhase::Dispatching,
                None,
                None,
            )
            .expect("already-dispatching prompt must remain eligible for receipt reconciliation");

        assert_eq!(active.status(), PromptStatus::Cancelling);
        assert_eq!(
            active.durable_delivery_phase(),
            Some(crate::session::DurablePromptDeliveryPhase::Dispatching)
        );
        assert_eq!(
            active.durable_delivery_provider_run_id(),
            Some("worker-run-1")
        );
    }

    #[test]
    fn activate_next_queued_prompt_rejects_when_active_prompt_exists() {
        let owner = PromptStateOwner::default();
        let session = RuntimeSession::new(
            "session-1",
            None,
            "workspace-1",
            "worktree-1",
            "machine-1",
            "daemon-1",
        );
        let started = owner
            .submit_prepared_prompt(
                &session,
                PromptQueueItem::new(
                    "prompt-active",
                    "attachment-1",
                    "agent-1",
                    "active",
                    PromptStatus::Queued,
                ),
                false,
            )
            .expect("first prompt should start");
        assert!(matches!(started, PromptSubmissionOutcome::Started { .. }));
        let queued = owner
            .submit_prepared_prompt(
                &session,
                PromptQueueItem::new(
                    "prompt-queued",
                    "attachment-1",
                    "agent-1",
                    "queued",
                    PromptStatus::Queued,
                ),
                false,
            )
            .expect("second prompt should queue");
        let queued_prompt_id = match queued {
            PromptSubmissionOutcome::Queued { prompt } => {
                assert!(prompt.id().starts_with("pending-prompt-"));
                assert_eq!(prompt.pending_prompt_id(), Some(prompt.id()));
                prompt.id().to_string()
            }
            PromptSubmissionOutcome::Started { .. } => panic!("second prompt should queue"),
        };

        let error = owner
            .activate_next_queued_prompt(&session, "agent-1", Some(&queued_prompt_id))
            .expect_err("queued prompt must not activate while active prompt is running");

        assert!(error.to_string().contains("cannot activate queued prompt"));
        assert_eq!(
            owner
                .active_prompt_for_agent_snapshot(&session, "agent-1")
                .as_ref()
                .map(|prompt| prompt.id()),
            Some("prompt-active")
        );
        assert_eq!(
            owner
                .peek_next_queued_prompt(&session, "agent-1")
                .as_ref()
                .map(|prompt| prompt.id()),
            Some(queued_prompt_id.as_str())
        );
    }

    #[test]
    fn queued_prompt_promotes_with_new_real_prompt_id() {
        let owner = PromptStateOwner::default();
        let session = RuntimeSession::new(
            "session-1",
            None,
            "workspace-1",
            "worktree-1",
            "machine-1",
            "daemon-1",
        );
        owner
            .submit_prepared_prompt(
                &session,
                PromptQueueItem::new(
                    "draft-active",
                    "attachment-1",
                    "agent-1",
                    "active",
                    PromptStatus::Queued,
                ),
                false,
            )
            .expect("first prompt should start");
        let pending_prompt_id = match owner
            .submit_prepared_prompt(
                &session,
                PromptQueueItem::new(
                    "draft-queued",
                    "attachment-1",
                    "agent-1",
                    "queued",
                    PromptStatus::Queued,
                ),
                false,
            )
            .expect("second prompt should queue")
        {
            PromptSubmissionOutcome::Queued { prompt } => {
                assert!(prompt.id().starts_with("pending-prompt-"));
                assert_eq!(prompt.pending_prompt_id(), Some(prompt.id()));
                prompt.id().to_string()
            }
            PromptSubmissionOutcome::Started { .. } => panic!("second prompt should queue"),
        };

        let completed = owner
            .complete_active_prompt_only(&session, "agent-1")
            .expect("active prompt should complete");
        assert_eq!(completed.id(), "draft-active");

        let started = owner
            .activate_next_queued_prompt_with_prompt_id(
                &session,
                "agent-1",
                Some(&pending_prompt_id),
                "prompt-real-2".to_string(),
            )
            .expect("queued prompt should activate")
            .expect("queued prompt should exist");

        assert_eq!(started.id(), "prompt-real-2");
        assert_eq!(started.pending_prompt_id(), None);
        assert_eq!(started.prompt(), "queued");
        assert_eq!(started.status(), PromptStatus::Dispatching);
        assert!(owner.peek_next_queued_prompt(&session, "agent-1").is_none());

        let running = owner
            .mark_active_prompt_running(&session, "agent-1")
            .expect("dispatching prompt should become running");
        assert_eq!(running.id(), "prompt-real-2");
        assert_eq!(running.status(), PromptStatus::Running);
    }

    #[test]
    fn remote_steer_reservation_and_completion_share_the_queue_owner_lock() {
        let owner = PromptStateOwner::default();
        let session = RuntimeSession::new(
            "session-remote-steer-reservation",
            None,
            "workspace-remote-steer-reservation",
            "worktree-remote-steer-reservation",
            "machine-1",
            "daemon-1",
        );
        owner
            .submit_prepared_prompt(
                &session,
                PromptQueueItem::new(
                    "prompt-active",
                    "attachment-1",
                    "agent-1",
                    "active",
                    PromptStatus::Queued,
                ),
                false,
            )
            .expect("first prompt should start");
        owner
            .submit_prepared_prompt(
                &session,
                PromptQueueItem::new(
                    "prompt-queued",
                    "attachment-1",
                    "agent-1",
                    "queued",
                    PromptStatus::Queued,
                ),
                false,
            )
            .expect("second prompt should queue");

        let queued = owner
            .peek_next_queued_prompt(&session, "agent-1")
            .expect("queued prompt should exist before reservation");
        let reservation_entered = std::sync::Arc::new(std::sync::Barrier::new(2));
        let reservation_continue = std::sync::Arc::new(std::sync::Barrier::new(2));
        let reservation = std::thread::scope(|scope| {
            let reservation_entered_for_thread = std::sync::Arc::clone(&reservation_entered);
            let reservation_continue_for_thread = std::sync::Arc::clone(&reservation_continue);
            let reservation_owner = &owner;
            let reservation_session = &session;
            let reservation_queued = &queued;
            let reservation_thread = scope.spawn(move || {
                reservation_owner.reserve_queued_prompt_remote_steer_with_hook(
                    reservation_session,
                    "agent-1",
                    "prompt-active",
                    reservation_queued,
                    || {
                        reservation_entered_for_thread.wait();
                        reservation_continue_for_thread.wait();
                    },
                )
            });

            reservation_entered.wait();
            assert!(matches!(
                owner.state.try_lock(),
                Err(std::sync::TryLockError::WouldBlock)
            ));

            let activation_start = std::sync::Arc::new(std::sync::Barrier::new(2));
            let activation_start_for_thread = std::sync::Arc::clone(&activation_start);
            let completion_owner = &owner;
            let completion_session = &session;
            let completion_queued = &queued;
            let completion_thread = scope.spawn(move || {
                activation_start_for_thread.wait();
                completion_owner
                    .complete_active_prompt_only(completion_session, "agent-1")
                    .expect("active prompt should complete after reservation commits");
                completion_owner.activate_next_queued_prompt_with_prompt_id(
                    completion_session,
                    "agent-1",
                    Some(completion_queued.id()),
                    "prompt-real-2".to_string(),
                )
            });
            activation_start.wait();
            reservation_continue.wait();

            let reservation = reservation_thread
                .join()
                .expect("reservation thread should not panic")
                .expect("exact queue item should reserve under the owner lock");
            let activation = completion_thread
                .join()
                .expect("completion thread should not panic")
                .expect("completion should defer reserved prompt activation");
            assert!(activation.is_none());
            reservation
        });

        assert!(owner.peek_next_queued_prompt(&session, "agent-1").is_none());
        assert!(owner
            .active_prompt_for_agent_snapshot(&session, "agent-1")
            .is_none());

        assert!(queued.release_remote_steer(reservation));
        assert_eq!(
            owner
                .peek_next_queued_prompt(&session, "agent-1")
                .as_ref()
                .map(PromptQueueItem::id),
            Some(queued.id())
        );
        assert_eq!(
            owner
                .activate_next_queued_prompt_with_prompt_id(
                    &session,
                    "agent-1",
                    Some(queued.id()),
                    "prompt-real-2".to_string(),
                )
                .expect("released prompt should be activatable")
                .expect("queued prompt should remain in place")
                .id(),
            "prompt-real-2"
        );
    }

    #[test]
    fn stale_completion_cannot_consume_a_promoted_prompt() {
        let owner = PromptStateOwner::default();
        let session = RuntimeSession::new(
            "session-stale-completion",
            None,
            "workspace-stale-completion",
            "worktree-stale-completion",
            "machine-1",
            "daemon-1",
        );
        owner
            .submit_prepared_prompt(
                &session,
                PromptQueueItem::new(
                    "prompt-first",
                    "attachment-1",
                    "agent-1",
                    "first",
                    PromptStatus::Queued,
                ),
                false,
            )
            .expect("first prompt should start");
        let pending_prompt_id = match owner
            .submit_prepared_prompt(
                &session,
                PromptQueueItem::new(
                    "prompt-queued",
                    "attachment-2",
                    "agent-1",
                    "second",
                    PromptStatus::Queued,
                ),
                false,
            )
            .expect("second prompt should queue")
        {
            PromptSubmissionOutcome::Queued { prompt } => prompt.id().to_string(),
            PromptSubmissionOutcome::Started { .. } => panic!("second prompt should queue"),
        };

        owner
            .complete_active_prompt_if_matches(&session, "agent-1", Some("prompt-first"))
            .expect("the matching first prompt should complete");
        owner
            .activate_next_queued_prompt_with_prompt_id(
                &session,
                "agent-1",
                Some(&pending_prompt_id),
                "prompt-second".to_string(),
            )
            .expect("queued prompt activation should succeed")
            .expect("queued prompt should promote");

        assert!(owner
            .complete_active_prompt_if_matches(&session, "agent-1", Some("prompt-first"))
            .is_none());
        assert_eq!(
            owner
                .active_prompt_for_agent(&session, "agent-1")
                .map(|prompt| prompt.id().to_string()),
            Some("prompt-second".to_string())
        );
    }

    #[test]
    fn project_into_session_uses_owned_prompt_state() {
        let owner = PromptStateOwner::default();
        let mut session = RuntimeSession::new(
            "session-1",
            None,
            "workspace-1",
            "worktree-1",
            "machine-1",
            "daemon-1",
        );
        session.set_agents(vec![crate::agent::AgentInstance::new(
            "agent-1",
            "agent-1",
            session.id(),
            None,
            "codex",
            None,
            None,
            None,
            crate::agent::GridPosition::new(0, 0, 1, 1),
        )]);
        let active = owner
            .submit_prepared_prompt(
                &session,
                PromptQueueItem::new(
                    "prompt-active",
                    "attachment-1",
                    "agent-1",
                    "active",
                    PromptStatus::Queued,
                ),
                false,
            )
            .expect("active prompt should submit");
        assert!(matches!(active, PromptSubmissionOutcome::Started { .. }));
        let queued = owner
            .submit_prepared_prompt(
                &session,
                PromptQueueItem::new(
                    "prompt-queued",
                    "attachment-1",
                    "agent-1",
                    "queued",
                    PromptStatus::Queued,
                ),
                false,
            )
            .expect("queued prompt should submit");
        assert!(matches!(queued, PromptSubmissionOutcome::Queued { .. }));
        assert!(session.active_prompt_for_agent("agent-1").is_none());
        assert!(session.queued_prompts_for_agent("agent-1").is_none());

        owner.project_into_session(&mut session);

        assert_eq!(
            session
                .active_prompt_for_agent("agent-1")
                .map(|prompt| prompt.prompt()),
            Some("active")
        );
        assert_eq!(
            session
                .queued_prompts_for_agent("agent-1")
                .and_then(|queued| queued.front())
                .map(|prompt| prompt.prompt()),
            Some("queued")
        );
    }

    #[test]
    fn projection_does_not_rehydrate_unrestored_session_mirror() {
        let owner = PromptStateOwner::default();
        let mut session = RuntimeSession::new(
            "session-1",
            None,
            "workspace-1",
            "worktree-1",
            "machine-1",
            "daemon-1",
        );
        session.mirror_agent_prompt_state(
            "agent-1",
            Some(PromptQueueItem::new(
                "stale-prompt",
                "attachment-1",
                "agent-1",
                "stale",
                PromptStatus::Running,
            )),
            VecDeque::new(),
        );

        assert!(session.active_prompt_for_agent("agent-1").is_some());
        assert!(owner
            .active_prompt_for_agent_snapshot(&session, "agent-1")
            .is_none());

        owner.project_into_session(&mut session);

        assert!(session.active_prompt_for_agent("agent-1").is_none());
    }

    #[test]
    fn active_prompt_lookup_does_not_rehydrate_unrestored_session_mirror() {
        let owner = PromptStateOwner::default();
        let mut session = RuntimeSession::new(
            "session-1",
            None,
            "workspace-1",
            "worktree-1",
            "machine-1",
            "daemon-1",
        );
        session.mirror_agent_prompt_state(
            "agent-1",
            Some(PromptQueueItem::new(
                "stale-prompt",
                "attachment-1",
                "agent-1",
                "stale",
                PromptStatus::Running,
            )),
            VecDeque::new(),
        );

        assert!(owner.active_prompt_for_agent(&session, "agent-1").is_none());
        assert!(owner
            .active_prompt_for_agent_snapshot(&session, "agent-1")
            .is_none());
    }

    #[test]
    fn submit_prepared_prompt_ignores_unrestored_session_mirror() {
        let owner = PromptStateOwner::default();
        let mut session = RuntimeSession::new(
            "session-1",
            None,
            "workspace-1",
            "worktree-1",
            "machine-1",
            "daemon-1",
        );
        session.mirror_agent_prompt_state(
            "agent-1",
            Some(PromptQueueItem::new(
                "stale-prompt",
                "attachment-1",
                "agent-1",
                "stale",
                PromptStatus::Running,
            )),
            VecDeque::new(),
        );

        let outcome = owner
            .submit_prepared_prompt(
                &session,
                PromptQueueItem::new(
                    "fresh-prompt",
                    "attachment-1",
                    "agent-1",
                    "fresh",
                    PromptStatus::Queued,
                ),
                false,
            )
            .expect("fresh prompt should submit");

        let started = match outcome {
            PromptSubmissionOutcome::Started { prompt } => prompt,
            PromptSubmissionOutcome::Queued { prompt } => {
                panic!("stale mirror queued fresh prompt as `{}`", prompt.id())
            }
        };
        assert_eq!(started.id(), "fresh-prompt");
        assert_eq!(
            owner
                .active_prompt_for_agent_snapshot(&session, "agent-1")
                .as_ref()
                .map(|prompt| prompt.id()),
            Some("fresh-prompt")
        );
    }

    #[test]
    fn reserved_prompt_restore_preserves_other_agents_and_rejects_replacement() {
        let owner = PromptStateOwner::default();
        let mut session = RuntimeSession::new(
            "session-1",
            None,
            "workspace-1",
            "worktree-1",
            "machine-1",
            "daemon-1",
        );
        let original = PromptQueueItem::new(
            "original",
            "attachment-1",
            "agent-1",
            "first",
            PromptStatus::Running,
        );
        session.mirror_agent_prompt_state("agent-1", Some(original.clone()), VecDeque::new());
        owner.restore_session_state(&session);
        owner
            .complete_active_prompt_if_matches(&session, "agent-1", Some("original"))
            .expect("first prompt should be reserved");
        owner
            .submit_prepared_prompt(
                &session,
                PromptQueueItem::new(
                    "other",
                    "attachment-2",
                    "agent-2",
                    "other",
                    PromptStatus::Queued,
                ),
                false,
            )
            .expect("another agent should accept work");
        let other_before = owner.state_parts(&session, "agent-2");
        assert!(owner.restore_reserved_prompt_if_unclaimed(
            &session,
            "agent-1",
            original.clone(),
            &VecDeque::new(),
        ));
        assert_eq!(owner.state_parts(&session, "agent-2"), other_before);
        assert_eq!(
            owner.state_parts(&session, "agent-1").0,
            Some(original.clone())
        );

        owner
            .complete_active_prompt_if_matches(&session, "agent-1", Some("original"))
            .expect("first prompt should be reserved again");
        owner
            .submit_prepared_prompt(
                &session,
                PromptQueueItem::new(
                    "replacement",
                    "attachment-1",
                    "agent-1",
                    "new",
                    PromptStatus::Queued,
                ),
                false,
            )
            .expect("replacement prompt should start");
        let replacement = owner.state_parts(&session, "agent-1");
        assert!(!owner.restore_reserved_prompt_if_unclaimed(
            &session,
            "agent-1",
            original,
            &VecDeque::new(),
        ));
        assert_eq!(owner.state_parts(&session, "agent-1"), replacement);
        assert_eq!(owner.state_parts(&session, "agent-2"), other_before);
    }

    #[test]
    fn restore_session_state_hydrates_owner_before_projection() {
        let owner = PromptStateOwner::default();
        let mut session = RuntimeSession::new(
            "session-1",
            None,
            "workspace-1",
            "worktree-1",
            "machine-1",
            "daemon-1",
        );
        session.mirror_agent_prompt_state(
            "agent-1",
            Some(PromptQueueItem::new(
                "restored-prompt",
                "attachment-1",
                "agent-1",
                "restored",
                PromptStatus::Running,
            )),
            VecDeque::from([PromptQueueItem::new(
                "restored-queued",
                "attachment-1",
                "agent-1",
                "queued",
                PromptStatus::Queued,
            )]),
        );

        owner.restore_session_state(&session);
        session.mirror_agent_prompt_state("agent-1", None, VecDeque::new());

        assert!(session.active_prompt_for_agent("agent-1").is_none());
        owner.project_into_session(&mut session);

        assert_eq!(
            session
                .active_prompt_for_agent("agent-1")
                .map(|prompt| prompt.id()),
            Some("restored-prompt")
        );
        assert_eq!(
            session
                .queued_prompts_for_agent("agent-1")
                .and_then(|queued| queued.front())
                .map(|prompt| prompt.id()),
            Some("restored-queued")
        );
    }

    #[test]
    fn restore_session_state_replaces_removed_prompt_states() {
        let owner = PromptStateOwner::default();
        let mut restored_with_prompt = RuntimeSession::new(
            "session-1",
            None,
            "workspace-1",
            "worktree-1",
            "machine-1",
            "daemon-1",
        );
        restored_with_prompt.mirror_agent_prompt_state(
            "agent-1",
            Some(PromptQueueItem::new(
                "restored-prompt",
                "attachment-1",
                "agent-1",
                "restored",
                PromptStatus::Running,
            )),
            VecDeque::new(),
        );
        owner.restore_session_state(&restored_with_prompt);
        assert!(owner
            .active_prompt_for_agent_snapshot(&restored_with_prompt, "agent-1")
            .is_some());

        let mut restored_without_prompt = RuntimeSession::new(
            "session-1",
            None,
            "workspace-1",
            "worktree-1",
            "machine-1",
            "daemon-1",
        );
        restored_without_prompt.set_agents(vec![crate::agent::AgentInstance::new(
            "agent-1",
            "agent-1",
            restored_without_prompt.id(),
            None,
            "codex",
            None,
            None,
            None,
            crate::agent::GridPosition::new(0, 0, 1, 1),
        )]);

        owner.restore_session_state(&restored_without_prompt);
        owner.project_into_session(&mut restored_without_prompt);

        assert!(restored_without_prompt
            .active_prompt_for_agent("agent-1")
            .is_none());
        assert!(owner
            .active_prompt_for_agent_snapshot(&restored_without_prompt, "agent-1")
            .is_none());
    }

    #[test]
    fn queued_prompt_activation_rejects_prompt_stored_under_wrong_agent() {
        let owner = PromptStateOwner::default();
        let session = RuntimeSession::new(
            "session-1",
            None,
            "workspace-1",
            "worktree-1",
            "machine-1",
            "daemon-1",
        );
        let queued_prompt = PromptQueueItem::new(
            "prompt-wrong-agent",
            "attachment-1",
            "agent-2",
            "queued",
            PromptStatus::Queued,
        );
        owner
            .state
            .lock()
            .expect("prompt state lock should not be poisoned")
            .states
            .insert(
                PromptStateKey::new(session.id(), "agent-1"),
                OwnedAgentPromptState {
                    sudo_entry_id: None,
                    sudo_process_cutoff: None,
                    sudo_work: None,
                    active_prompt: None,
                    queued_prompts: VecDeque::from([queued_prompt]),
                },
            );

        let error = owner
            .activate_next_queued_prompt_with_prompt_id(
                &session,
                "agent-1",
                Some("prompt-wrong-agent"),
                "prompt-real-1".to_string(),
            )
            .expect_err("mismatched queued prompt target must be rejected");

        assert!(error.to_string().contains("prompt `prompt-wrong-agent`"));
        assert!(error.to_string().contains("targets agent `agent-2`"));
        assert!(error.to_string().contains("stored under agent `agent-1`"));
        assert!(owner
            .active_prompt_for_agent_snapshot(&session, "agent-1")
            .is_none());
        assert_eq!(
            owner
                .peek_next_queued_prompt(&session, "agent-1")
                .as_ref()
                .map(|prompt| prompt.id()),
            Some("prompt-wrong-agent")
        );
    }

    #[test]
    fn external_active_prompt_sync_ignores_prompt_targeting_different_agent() {
        let owner = PromptStateOwner::default();
        let session = RuntimeSession::new(
            "session-1",
            None,
            "workspace-1",
            "worktree-1",
            "machine-1",
            "daemon-1",
        );
        let external_prompt = PromptQueueItem::external_observed_running(
            "codex",
            "thread-1",
            "user-1",
            "agent-2",
            "external prompt",
        );

        let changed = owner.sync_external_active_prompt(&session, "agent-1", Some(external_prompt));

        assert!(!changed);
        assert!(owner
            .active_prompt_for_agent_snapshot(&session, "agent-1")
            .is_none());
    }

    #[test]
    fn activate_prompt_rejects_replacing_different_active_prompt() {
        let owner = PromptStateOwner::default();
        let session = RuntimeSession::new(
            "session-1",
            None,
            "workspace-1",
            "worktree-1",
            "machine-1",
            "daemon-1",
        );
        let active = owner
            .activate_prompt(
                &session,
                PromptQueueItem::new(
                    "prompt-active",
                    "attachment-1",
                    "agent-1",
                    "active",
                    PromptStatus::Queued,
                ),
            )
            .expect("first prompt should activate");
        assert_eq!(active.status(), PromptStatus::Running);

        let error = owner
            .activate_prompt(
                &session,
                PromptQueueItem::new(
                    "prompt-replacement",
                    "attachment-1",
                    "agent-1",
                    "replacement",
                    PromptStatus::Queued,
                ),
            )
            .expect_err("different active prompt must not be replaced");

        assert!(error.to_string().contains("cannot activate prompt"));
        assert_eq!(
            owner
                .active_prompt_for_agent_snapshot(&session, "agent-1")
                .as_ref()
                .map(|prompt| prompt.id()),
            Some("prompt-active")
        );
    }
}

#[cfg(test)]
#[path = "prompt_state/dispatch_intent_tests.rs"]
mod dispatch_intent_tests;
