//! MP-08 / MP-09 / MP-10 / MP-11 A12: fresh consent for existing goal work.
use super::*;

impl KernelRuntimeState {
    pub(in crate::runtime::state) fn bind_sudo_goal_resume(
        &self,
        entry: &KernelSudoTurn,
        task: &crate::durable_state::agent_lifecycle::AgentTaskExecution,
    ) -> Result<(), DaemonError> {
        let entry = self.live_queued_sudo(entry)?;
        let prompt = task
            .pending_prompt_id
            .as_deref()
            .ok_or_else(|| error("goal resume has no continuation; retry /goal resume"))?;
        if task.room_id != entry.session_id || task.agent_id != entry.agent_id {
            return Err(error(
                "goal resume changed its target; retry /sudo /goal resume",
            ));
        }
        let session = self.owned.session_store.get_session(&entry.session_id)?;
        self.owned.prompt_state_owner.hold_sudo_work(
            &session,
            &entry.agent_id,
            &entry.entry_id,
            prompt,
        );
        let mut turn = entry.clone();
        turn.prompt_id = Some(prompt.into());
        turn.task_id = Some(task.task_id.clone());
        if entry.placement.is_none() {
            let run = self
                .owned
                .provider_store
                .get_run_for_agent(&entry.session_id, &entry.agent_id)
                .ok_or_else(|| {
                    error("provider unavailable for goal resume; retry when it is ready")
                })?;
            turn.provider_run_id = Some(run.id().to_owned());
        }
        // Only the durable continuation can enter this hold. Authority still
        // requires its running prompt binding, installed by normal admission.
        self.admit_sudo_turn(&entry, &turn, true)
    }
}
