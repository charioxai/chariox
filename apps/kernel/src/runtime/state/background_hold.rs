//! MP-08 / MP-09 / MP-10 / MP-11: a sub-agent whose turn ended while a `run_in_background` call
//! still runs has not finished: the provider will wake it with a notification turn when the call
//! ends. Its one task stays Working through that turn, so its parent hears of one finish that
//! carries the notification's text.
use super::*;
use crate::app::BackgroundCalls;
use crate::durable_state::agent_lifecycle::{AgentObligation, AgentTaskExecution, ExecutionState};

/// How long the notification turn has to start after the last background call ended.
const NOTIFICATION_GRACE_MS: u64 = 60_000;
/// How long a call that never ends may hold a child that has no deadline.
const HOLD_LIMIT_MS: u64 = 30 * 60_000;
/// What a parent reads when the child was released with a call still running.
pub(super) const STILL_RUNNING_NOTE: &str =
    "A background command was still running when this agent was released; its result is not part of this answer.";

#[derive(Clone)]
struct HeldTask {
    room: String,
    agent: String,
    run: String,
    since_ms: u64,
    /// The provider's notification turn has produced output: it is executing, not waiting.
    notification_started: bool,
}

/// The tasks held for background calls, by task id. A restart drops them: the run's calls end
/// with it, and the ledger sweep settles the task like any answered turn.
#[derive(Clone, Default)]
pub(super) struct BackgroundHolds(Arc<std::sync::Mutex<BTreeMap<String, HeldTask>>>);

impl BackgroundHolds {
    fn lock(&self) -> std::sync::MutexGuard<'_, BTreeMap<String, HeldTask>> {
        self.0.lock().unwrap_or_else(|e| e.into_inner())
    }
    /// When the task was first held.
    fn hold(&self, task: &AgentTaskExecution, run: &str, now: u64) -> u64 {
        self.lock()
            .entry(task.task_id.clone())
            .or_insert_with(|| HeldTask {
                room: task.room_id.clone(),
                agent: task.agent_id.clone(),
                run: run.into(),
                since_ms: now,
                notification_started: false,
            })
            .since_ms
    }
    fn get(&self, task_id: &str) -> Option<HeldTask> {
        self.lock().get(task_id).cloned()
    }
    #[cfg(test)]
    pub(super) fn held_since_for_test(&self, task_id: &str, since_ms: u64) {
        if let Some(held) = self.lock().get_mut(task_id) {
            held.since_ms = since_ms;
        }
    }
    pub(super) fn note_output_of_run(&self, run: &str) {
        for held in self.lock().values_mut().filter(|held| held.run == run) {
            held.notification_started = true;
        }
    }
    pub(super) fn release(&self, task_id: &str) {
        self.lock().remove(task_id);
    }
    /// These runs still own work after their prompted turn ended.
    pub(super) fn provider_run_ids_for_session(&self, room: &str) -> BTreeSet<String> {
        self.lock()
            .values()
            .filter(|held| held.room == room)
            .map(|held| held.run.clone())
            .collect()
    }

    pub(super) fn holds_run(&self, run: &str) -> bool {
        self.lock().values().any(|held| held.run == run)
    }

    fn of_run(&self, room: &str, agent: &str, run: &str) -> Vec<String> {
        self.lock()
            .iter()
            .filter(|(_, held)| held.room == room && held.agent == agent && held.run == run)
            .map(|(task_id, _)| task_id.clone())
            .collect()
    }
}

impl KernelRuntimeOwnedState {
    /// A run held for background calls produced output: a turn is executing on it.
    pub(super) fn note_output_of_held_run(
        &self,
        run: &str,
        chunks: &[crate::provider::ProviderPromptChunk],
    ) {
        if chunks
            .iter()
            .any(|chunk| crate::app::is_work_of_a_turn(&chunk.kind, &chunk.bytes))
        {
            self.background_holds.note_output_of_run(run);
        }
    }

    fn background_calls_of_run(
        &self,
        room: &str,
        run: &str,
    ) -> Result<BackgroundCalls, DaemonError> {
        Ok(crate::app::background_calls(
            self.operational_history_store
                .load_provider_tool_entries_for_run(room, run)?,
        ))
    }

    /// A child's plain turn ends while a background call runs: its task stays Working, so the
    /// parent is told once, after the notification turn. Only a child waits this way, and only
    /// while the call can end: its deadline stops the child, and a child without one is released
    /// after `HOLD_LIMIT_MS`.
    pub(super) fn holds_task_for_background_calls(
        &self,
        room: &str,
        agent: &str,
        prompt: &str,
        run: &str,
    ) -> Result<bool, DaemonError> {
        let Some(task) = self
            .durable_state_store
            .agent_turn_task(room, agent, prompt)?
        else {
            return Ok(false);
        };
        let plain = task.state == ExecutionState::Working
            && task.goal.is_none()
            && task.wait.is_none()
            && task.pending_prompt_id.is_none()
            && !task
                .obligations
                .iter()
                .any(AgentObligation::requires_resolution);
        if !plain
            || !self
                .agent_store
                .get_agent(agent)
                .is_ok_and(|agent| agent.parent_agent_id().is_some())
            || self.background_calls_of_run(room, run)? != BackgroundCalls::Running
        {
            self.background_holds.release(&task.task_id);
            return Ok(false);
        }
        let now = crate::session::unix_epoch_ms();
        let since = self.background_holds.hold(&task, run, now);
        if now.saturating_sub(since) >= HOLD_LIMIT_MS
            && self
                .durable_state_store
                .agent_deadline(room, agent)?
                .is_none()
        {
            self.background_holds.release(&task.task_id);
            return Ok(false);
        }
        Ok(true)
    }

    /// Whether the sweep settles a held task now. Without a call still running, the
    /// notification turn has `NOTIFICATION_GRACE_MS` to start; once it executes it is held until
    /// it completes, like a call that runs on, until `HOLD_LIMIT_MS` (a deadline decides sooner).
    pub(super) fn held_task_due(
        &self,
        task: &AgentTaskExecution,
        now: u64,
    ) -> Result<bool, DaemonError> {
        let Some(held) = self.background_holds.get(&task.task_id) else {
            return Ok(false);
        };
        let limit_reached = || -> Result<bool, DaemonError> {
            Ok(now.saturating_sub(held.since_ms) >= HOLD_LIMIT_MS
                && self
                    .durable_state_store
                    .agent_deadline(&held.room, &held.agent)?
                    .is_none())
        };
        Ok(match self.background_calls_of_run(&held.room, &held.run)? {
            BackgroundCalls::None => true,
            BackgroundCalls::Running => limit_reached()?,
            BackgroundCalls::Ended { .. } if held.notification_started => limit_reached()?,
            BackgroundCalls::Ended { last_ended_at_ms } => {
                now.saturating_sub(last_ended_at_ms) >= NOTIFICATION_GRACE_MS
            }
        })
    }

    pub(super) fn is_held_for_background_calls(&self, task_id: &str) -> bool {
        self.background_holds.get(task_id).is_some()
    }

    /// The tasks of `agent` held on `run` whose background calls have all ended: the turn
    /// that just completed was the provider's notification.
    pub(super) fn held_tasks_after_notification(
        &self,
        room: &str,
        agent: &str,
        run: &str,
    ) -> Result<Vec<AgentTaskExecution>, DaemonError> {
        if self.background_calls_of_run(room, run)? == BackgroundCalls::Running {
            return Ok(Vec::new());
        }
        let mut tasks = Vec::new();
        for task_id in self.background_holds.of_run(room, agent, run) {
            if let Some(task) = self.durable_state_store.agent_task(&task_id)? {
                tasks.push(task);
            }
        }
        Ok(tasks)
    }

    /// The answer of a task that is stopped or released while its run still has a background
    /// call running, so the parent does not take the call's absence for a result.
    pub(super) fn answer_with_running_background_note(
        &self,
        task: &AgentTaskExecution,
        answer: Option<serde_json::Value>,
    ) -> Result<Option<serde_json::Value>, DaemonError> {
        let Some(run) = task.provider_run_id.as_deref() else {
            return Ok(answer);
        };
        if self.background_calls_of_run(&task.room_id, run)? != BackgroundCalls::Running {
            return Ok(answer);
        }
        let mut answer = answer.unwrap_or_else(|| {
            serde_json::json!({"agent_id":task.agent_id,"task_id":task.task_id,"prompt_id":task.prompt_id,"excerpt":""})
        });
        if let Some(excerpt) = answer.get_mut("excerpt") {
            let text = excerpt.as_str().unwrap_or_default().trim_end();
            *excerpt = if text.is_empty() {
                STILL_RUNNING_NOTE.into()
            } else {
                format!("{text}\n\n{STILL_RUNNING_NOTE}").into()
            };
        }
        Ok(Some(answer))
    }
}
