//! MP-08/MP-10/MP-11: human takeover suspends agent work without a modal.
use super::*;
use crate::durable_state::agent_lifecycle::{AgentTaskExecution, ExecutionState};
use crate::session::{EnvironmentActorKind, InputTarget};

fn describes_human_control_wait(reason: &str) -> bool {
    let reason = reason.to_ascii_lowercase();
    reason.contains("environment_input_owned")
        || reason.contains("human_takeover")
        || (reason.contains("human")
            && [
                "control",
                "takeover",
                "desktop",
                "release",
                "input",
                "took over",
            ]
            .iter()
            .any(|word| reason.contains(word)))
}

// A late Blocked event can arrive after Release. Only a concrete Computer
// ownership refusal authorizes continuation without a current human owner.
fn describes_released_human_control_wait(reason: &str) -> bool {
    let reason = reason.to_ascii_lowercase();
    reason.contains("environment_input_owned")
        || reason.contains("human_takeover")
        || (describes_human_control_wait(&reason)
            && reason.contains("computer")
            && ["owned", "belongs", "stopped", "refused", "rejected"]
                .iter()
                .any(|word| reason.contains(word)))
}

impl KernelRuntimeState {
    fn task_has_human_input_owner(&self, task: &AgentTaskExecution) -> bool {
        if self
            .room_environment_snapshot(&task.room_id)
            .is_ok_and(|room| {
                room.input_ownership.iter().any(|owner| {
                    owner.target == InputTarget::Desktop
                        && room.actors.iter().any(|actor| {
                            actor.actor_id == owner.actor_id
                                && actor.kind == EnvironmentActorKind::Human
                        })
                })
            })
        {
            return true;
        }
        let user = self.provider_account_authority_owner_user_id(&task.owner_user_id);
        self.owned
            .kernel_browser_host
            .actor_snapshot(&user)
            .is_ok_and(|snapshot| {
                snapshot["input_ownership"]
                    .as_array()
                    .is_some_and(|owners| {
                        owners.iter().any(|owner| {
                            snapshot["actors"].as_array().is_some_and(|actors| {
                                actors.iter().any(|actor| {
                                    actor["actor_id"] == owner["actor_id"]
                                        && actor["kind"] == "human"
                                })
                            })
                        })
                    })
            })
    }

    pub(super) async fn defer_task_for_human_control(&self, task: &AgentTaskExecution) -> bool {
        let eligible = task.state == ExecutionState::Blocked
            && !task.goal.as_ref().is_some_and(|goal| goal.paused);
        let recorded = {
            let mut waits = self
                .owned
                .human_control_task_waits
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            if !eligible
                || waits
                    .get(&task.task_id)
                    .is_some_and(|revision| *revision != task.blocked_revision)
            {
                waits.remove(&task.task_id);
            }
            waits.get(&task.task_id) == Some(&task.blocked_revision)
        };
        if !eligible {
            return false;
        }
        if self.task_has_human_input_owner(task) {
            // Even a genuine task decision must leave Release accessible. It
            // becomes an owner interaction after Release; only a control wait
            // is eligible for automatic continuation.
            let control_wait = recorded || describes_human_control_wait(&task.reason);
            let fresh = control_wait
                && self
                    .owned
                    .human_control_task_waits
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .insert(task.task_id.clone(), task.blocked_revision)
                    .is_none();
            if fresh {
                self.owned.record_notice_for_agent(
                    &task.room_id,
                    None,
                    Some(&task.agent_id),
                    self.owned
                        .attachment_store
                        .list_session_attachment_ids(&task.room_id),
                    "Waiting — you have control. The agent continues after Release.",
                );
                self.owned
                    .terminal_stream
                    .notify_terminal_projection_change(&task.room_id);
            }
            let id = format!("task-blocked-{}-{}", task.task_id, task.blocked_revision);
            let _ = self.timeout_runtime_interaction(&task.room_id, &id).await;
            return true;
        }
        if !recorded {
            if !describes_released_human_control_wait(&task.reason) {
                return false;
            }
            self.owned
                .human_control_task_waits
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .insert(task.task_id.clone(), task.blocked_revision);
        }
        // The native turn may still be settling after its Blocked event. Never
        // overlap it with the continuation triggered by the human's Release.
        if let Ok(session) = self.owned.session_store.get_session(&task.room_id) {
            if self
                .owned
                .prompt_state_owner
                .active_prompt_for_agent(&session, &task.agent_id)
                .is_some()
                && self.blocked_turn_has_executor(task)
            {
                return true;
            }
        }
        let claimed = {
            let mut waits = self
                .owned
                .human_control_task_waits
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            if waits.get(&task.task_id) == Some(&task.blocked_revision) {
                waits.remove(&task.task_id);
                true
            } else {
                false
            }
        };
        if claimed {
            Box::pin(self.resolve_agent_task_owner_action(task, true)).await;
        }
        true
    }
}
