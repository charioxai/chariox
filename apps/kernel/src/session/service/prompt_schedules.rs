use super::*;

const DEFAULT_CONTINUATION_PROMPT: &str = "Continue from where you left off.";

impl SessionService {
    pub fn next_scheduled_runtime_wake_at_ms(&self) -> Option<u64> {
        self.store
            .non_ended_sessions()
            .flat_map(|session| {
                let watchdogs = session
                    .workflow_watchdogs()
                    .iter()
                    .filter(|watchdog| {
                        watchdog.enabled()
                            && !watchdog
                                .max_wakeups()
                                .is_some_and(|limit| watchdog.wakeups_executed() >= limit)
                    })
                    .map(|watchdog| watchdog.next_run_at_ms());
                let agent_prompts = session
                    .agent_prompt_schedules()
                    .iter()
                    .map(|schedule| schedule.next_run_at_ms());
                watchdogs.chain(agent_prompts)
            })
            .min()
    }

    pub fn create_agent_prompt_schedule(
        &mut self,
        session_id: &str,
        agent_id: &str,
        kind: AgentPromptScheduleKind,
        interval_seconds: u64,
        prompt: Option<String>,
    ) -> Result<AgentPromptSchedule, DaemonError> {
        if interval_seconds == 0 {
            return Err(DaemonError::LocalTransport {
                operation: "agent prompt schedule create",
                message: "wait duration must be greater than zero".to_string(),
            });
        }
        self.get_session(session_id)?;
        let prompt = prompt
            .map(|prompt| prompt.trim().to_string())
            .filter(|prompt| !prompt.is_empty())
            .unwrap_or_else(|| DEFAULT_CONTINUATION_PROMPT.to_string());
        let schedule = AgentPromptSchedule::new(
            self.next_agent_prompt_schedule_id(),
            agent_id,
            kind,
            interval_seconds,
            prompt,
            unix_epoch_ms(),
        );
        let session =
            self.store
                .get_mut(session_id)
                .ok_or_else(|| DaemonError::SessionNotFound {
                    session_id: session_id.to_string(),
                })?;
        Ok(session.add_agent_prompt_schedule(schedule))
    }

    pub fn cancel_agent_prompt_schedule(
        &mut self,
        session_id: &str,
        schedule_id: &str,
    ) -> Result<AgentPromptSchedule, DaemonError> {
        let session =
            self.store
                .get_mut(session_id)
                .ok_or_else(|| DaemonError::SessionNotFound {
                    session_id: session_id.to_string(),
                })?;
        session
            .remove_agent_prompt_schedule(schedule_id)
            .ok_or_else(|| DaemonError::LocalTransport {
                operation: "agent prompt schedule cancel",
                message: format!(
                    "agent prompt schedule `{schedule_id}` was not found in session `{session_id}`"
                ),
            })
    }

    pub fn collect_due_agent_prompt_schedules(&self, now_ms: u64) -> AgentPromptScheduleCollection {
        let mut collection = AgentPromptScheduleCollection::default();
        for session in self.store.non_ended_sessions() {
            for schedule in session.agent_prompt_schedules() {
                if now_ms >= schedule.next_run_at_ms() {
                    collection.dispatches.push(AgentPromptScheduleDispatch {
                        session_id: session.id().to_string(),
                        schedule_id: schedule.id().to_string(),
                        agent_id: schedule.agent_id().to_string(),
                        prompt: schedule.prompt().to_string(),
                    });
                }
            }
        }
        collection
    }

    pub fn mark_agent_prompt_schedule_dispatched(
        &mut self,
        session_id: &str,
        schedule_id: &str,
        now_ms: u64,
    ) -> Result<Option<AgentPromptSchedule>, DaemonError> {
        let session =
            self.store
                .get_mut(session_id)
                .ok_or_else(|| DaemonError::SessionNotFound {
                    session_id: session_id.to_string(),
                })?;
        let Some(index) = session
            .agent_prompt_schedules()
            .iter()
            .position(|schedule| schedule.id() == schedule_id)
        else {
            return Ok(None);
        };
        if session.agent_prompt_schedules()[index].kind() == AgentPromptScheduleKind::Once {
            return Ok(session.remove_agent_prompt_schedule(schedule_id));
        }
        let schedule = &mut session.agent_prompt_schedules_mut()[index];
        schedule.mark_dispatch_succeeded(now_ms);
        Ok(Some(schedule.clone()))
    }

    pub fn mark_agent_prompt_schedule_failed(
        &mut self,
        session_id: &str,
        schedule_id: &str,
        now_ms: u64,
        error: String,
    ) -> Result<Option<AgentPromptSchedule>, DaemonError> {
        let session =
            self.store
                .get_mut(session_id)
                .ok_or_else(|| DaemonError::SessionNotFound {
                    session_id: session_id.to_string(),
                })?;
        let Some(schedule) = session
            .agent_prompt_schedules_mut()
            .iter_mut()
            .find(|schedule| schedule.id() == schedule_id)
        else {
            return Ok(None);
        };
        schedule.mark_dispatch_failed(now_ms, error);
        Ok(Some(schedule.clone()))
    }
}
