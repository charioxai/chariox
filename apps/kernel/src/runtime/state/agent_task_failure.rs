//! MP-08 / MP-10 / MP-11: retain the cause when an admitted provider dispatch fails.
//! A delegate that cannot start fails the delegate obligation and every reply awaiting it.

use super::*;
use crate::durable_state::agent_lifecycle::{ExecutionState, Operation};

impl KernelRuntimeOwnedState {
    pub(super) fn block_vault_refused_inbox_start(
        &self,
        room: &str,
        agent: &str,
        sequence: u64,
        error: &DaemonError,
    ) -> Result<(), DaemonError> {
        let event = self
            .durable_state_store
            .agent_work_inbox(room, agent, sequence.saturating_sub(1), None)?
            .into_iter()
            .find(|event| event.sequence == sequence)
            .ok_or_else(|| {
                crate::durable_state::agent_lifecycle::error("startup refusal has no inbox event")
            })?;
        if event.submit_epoch.is_some() {
            return Ok(());
        }
        let cause = match error {
            DaemonError::VaultUnlock {
                code: "vault_unlock_declined",
                ..
            } => "The Vault unlock was declined",
            DaemonError::VaultUnlock {
                code: "vault_unlock_timed_out",
                ..
            } => "The Vault unlock timed out",
            _ => return Ok(()),
        };
        for task in self
            .durable_state_store
            .agent_tasks(Some(room), Some(agent))?
        {
            if matches!(
                task.state,
                ExecutionState::Working | ExecutionState::Blocked
            ) && task.wait.is_some()
                && (event.payload["task_id"].as_str() == Some(task.task_id.as_str())
                    || event.payload["task_ids"].as_array().is_some_and(|ids| {
                        ids.iter()
                            .any(|id| id.as_str() == Some(task.task_id.as_str()))
                    }))
            {
                self.durable_state_store.agent_lifecycle(Operation::Block {
                    task: task.task_id,
                    prompt: task.prompt_id,
                    reason: format!("{cause}; unlock the Vault and Resume, or Cancel"),
                })?;
            }
        }
        Ok(())
    }

    pub(super) fn settle_failed_delegate_start(
        &self,
        room: &str,
        agent: &str,
        diagnostic: &str,
    ) -> Result<(), DaemonError> {
        crate::logging::warn_with_fields(
            "daemon.agent_task",
            "delegate could not start its requested turn",
            serde_json::json!({
                "session_id": room,
                "agent_id": agent,
                "reason": crate::provider::sanitize_provider_diagnostic(diagnostic),
            }),
        );
        self.durable_state_store
            .agent_lifecycle(Operation::SourceOutcome {
                room: room.into(),
                source: agent.into(),
                occurrence: "failed-start".into(),
                success: false,
                public_answer: Some(
                    crate::durable_state::agent_lifecycle::DelegateGoneReason::FailedStart
                        .answer(agent),
                ),
                now: crate::session::unix_epoch_ms(),
            })?;
        Ok(())
    }

    pub(super) fn block_failed_prompt_task(
        &self,
        room: &str,
        agent: &str,
        prompt: &str,
        run: Option<&str>,
        diagnostic: &str,
    ) -> Result<(), DaemonError> {
        if !self.config_projection.snapshot().room_agent_tools {
            return Ok(());
        }
        let task = self
            .durable_state_store
            .agent_tasks(Some(room), Some(agent))?
            .into_iter()
            .find(|task| {
                task.prompt_id == prompt
                    && task.state == ExecutionState::Working
                    && run.is_none_or(|run| {
                        task.provider_run_id.as_deref().is_none_or(|id| id == run)
                    })
            });
        if let Some(task) = task {
            if let Some(event) = self
                .durable_state_store
                .agent_event_for_prompt(room, agent, prompt)?
            {
                if event.reply_requested {
                    self.settle_failed_delegate_start(room, agent, diagnostic)?;
                }
            }
            let diagnostic = crate::provider::sanitize_provider_diagnostic(diagnostic);
            self.durable_state_store.agent_lifecycle(Operation::Block {
                task: task.task_id,
                prompt: task.prompt_id,
                reason: format!(
                    "{}. Check the result before using Resume to retry, or Cancel.",
                    diagnostic.trim_end_matches('.')
                ),
            })?;
        }
        Ok(())
    }
}
