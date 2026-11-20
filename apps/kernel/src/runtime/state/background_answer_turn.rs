//! A native background notification may be consumed by the prompt's already-open provider turn.
use super::*;
use crate::durable_state::agent_lifecycle::{AgentTaskExecution, ExecutionState};
use crate::provider::{AnsweredTurnReceipt, ProviderBackgroundNotification, TurnOutcome};

impl KernelRuntimeOwnedState {
    fn background_call_records(
        &self,
        task: &AgentTaskExecution,
    ) -> Result<BTreeMap<String, serde_json::Value>, DaemonError> {
        let Some(run) = task.provider_run_id.as_deref() else {
            return Ok(BTreeMap::new());
        };
        let mut calls = BTreeMap::new();
        for entry in self
            .operational_history_store
            .load_provider_tool_entries_for_prompt(&task.room_id, run, &task.prompt_id)?
        {
            let Ok(value) = serde_json::from_str::<serde_json::Value>(&entry.text) else {
                continue;
            };
            if value.get("background").and_then(serde_json::Value::as_bool) == Some(true) {
                if let Some(id) = value.get("id").and_then(serde_json::Value::as_str) {
                    calls.insert(id.to_string(), value);
                }
            }
        }
        Ok(calls)
    }

    pub(super) fn record_background_answer_turn(
        &self,
        room: &str,
        agent: &str,
        run: &str,
        prompt: &crate::session::PromptQueueItem,
        notifications: &[ProviderBackgroundNotification],
    ) -> Result<(), DaemonError> {
        if notifications.is_empty() {
            return Ok(());
        }
        self.durable_state_store
            .record_provider_background_notifications(run, prompt.id(), notifications)?;
        let Some(answer_task) =
            self.durable_state_store
                .agent_turn_task(room, agent, prompt.id())?
        else {
            return Ok(());
        };
        for (task_id, _) in self.background_holds.of_run(room, agent, run) {
            let Some(task) = self.durable_state_store.agent_task(&task_id)? else {
                continue;
            };
            if task.task_id == answer_task.task_id {
                continue;
            }
            let calls = self.background_call_records(&task)?;
            for notification in notifications {
                for (id, value) in &calls {
                    let matches = notification
                        .tool_use_id
                        .as_ref()
                        .is_some_and(|tool| tool == id)
                        || notification.task_id.as_ref().is_some_and(|task| {
                            crate::provider::launched_task_id(
                                value.get("output_structured"),
                                value
                                    .get("output")
                                    .and_then(serde_json::Value::as_str)
                                    .unwrap_or_default(),
                            )
                            .as_ref()
                                == Some(task)
                        });
                    if matches {
                        self.durable_state_store.record_answer_turn(
                            &task_id,
                            &AnsweredTurnReceipt {
                                session_id: room.to_string(),
                                agent_id: agent.to_string(),
                                prompt_id: prompt.id().to_string(),
                                task_id: answer_task.task_id.clone(),
                            },
                            id,
                        )?;
                    }
                }
            }
        }
        Ok(())
    }

    pub(super) fn background_answer_outcome(
        &self,
        receipt: AnsweredTurnReceipt,
        answer: &AgentTaskExecution,
    ) -> TurnOutcome {
        if answer.state == ExecutionState::Cancelled {
            let mut outcome = self.finished_task_outcome(answer, None);
            outcome.answered_in_turn = Some(receipt);
            outcome
        } else {
            TurnOutcome::answered_in_turn(receipt)
        }
    }

    /// The ledger can be terminal before its receipt-bearing history append.
    pub(super) fn recover_background_answer_outcome(
        &self,
        task: &AgentTaskExecution,
    ) -> Result<Option<TurnOutcome>, DaemonError> {
        Ok(self
            .background_answer_target(task)?
            .filter(|(_, answer)| answer.state == task.state)
            .map(|(receipt, answer)| self.background_answer_outcome(receipt, &answer)))
    }

    /// A receipt is ready only after every background notification was consumed and its answering task ended.
    pub(super) fn background_answer_target(
        &self,
        task: &AgentTaskExecution,
    ) -> Result<Option<(AnsweredTurnReceipt, AgentTaskExecution)>, DaemonError> {
        let Some(record) = self.durable_state_store.answer_turn(&task.task_id)? else {
            return Ok(None);
        };
        let calls = self.background_call_records(task)?;
        if calls.is_empty() || calls.keys().any(|id| !record.consumed_calls.contains(id)) {
            return Ok(None);
        }
        let Some(answer) = self.durable_state_store.agent_task(&record.turn.task_id)? else {
            return Ok(None);
        };
        if !matches!(
            answer.state,
            ExecutionState::Done | ExecutionState::Cancelled
        ) {
            return Ok(None);
        }
        Ok(Some((record.turn, answer)))
    }
}
