//! Row 554: an agent that is idle with nothing armed is stranded, and nothing says so. The kernel
//! helps and never blocks (Miguel 11-12, "the kernel must not park or block agents"):
//!
//! 1. a turn that ends with background commands still running gets each one adopted as a wake;
//! 2. a turn that ends saying it waits, with nothing armed, gets a backstop timer;
//! 3. a turn end is never refused: both are best effort, and a failure is only logged;
//! 4. a message queued for an idle agent with nothing armed starts its turn.
//!
//! Each tells the agent what the kernel did, in a notice next to its turn.
use super::adopted_process_wake::{self as adopted, AdoptedDetail};
use super::*;
use crate::durable_state::agent_lifecycle::{
    AgentTaskExecution, AgentWake, ExecutionState, Operation, DEFAULT_WAIT_TIMEOUT_MS,
};

/// What a backstop timer's label starts with; the delivery of its firing reads it.
pub(super) const BACKSTOP_LABEL: &str = "backstop for a stated wait";
/// How much of the end of a turn's answer is read for a stated wait.
const ANSWER_TAIL_CHARS: usize = 1200;
/// The longest stated wait quoted back to the agent.
const PHRASE_CHARS: usize = 140;

/// Words that say the agent waits for something.
const WAIT_PHRASES: &[&str] = &[
    "waiting for",
    "waiting on",
    "awaiting",
    "will wait for",
    "will wake me",
    "will be woken",
    "wake me when",
];
/// Words that say it does not.
const NOT_WAITING: &[&str] = &[
    "not waiting",
    "no longer waiting",
    "isn't waiting",
    "isnt waiting",
    "nothing to wait",
    "without waiting",
    "no need to wait",
    "don't need to wait",
    "not awaiting",
];

/// The sentence of the end of `answer` that says the agent waits, if one does.
pub(super) fn stated_wait(answer: &str) -> Option<String> {
    let start = answer
        .char_indices()
        .rev()
        .nth(ANSWER_TAIL_CHARS)
        .map_or(0, |(at, _)| at);
    answer[start..]
        .split(['.', '!', '?', '\n'])
        .rev()
        .map(str::trim)
        .find(|sentence| {
            let lower = sentence.to_lowercase();
            WAIT_PHRASES.iter().any(|phrase| lower.contains(phrase))
                && !NOT_WAITING.iter().any(|phrase| lower.contains(phrase))
        })
        .map(|sentence| match sentence.char_indices().nth(PHRASE_CHARS) {
            Some((cut, _)) => format!("{}…", &sentence[..cut]),
            None => sentence.to_string(),
        })
}

/// `HH:MM` UTC of a Unix time in milliseconds.
fn clock_utc(ms: u64) -> String {
    let minutes = ms / 60_000 % (24 * 60);
    format!("{:02}:{:02} UTC", minutes / 60, minutes % 60)
}

fn kernel_wake(task: &AgentTaskExecution, kind: &str, label: String, now: u64) -> AgentWake {
    let id = format!("wake-{:032x}", rand::random::<u128>());
    AgentWake {
        registration_id: format!("completion-{id}"),
        id,
        task_id: task.task_id.clone(),
        room_id: task.room_id.clone(),
        agent_id: task.agent_id.clone(),
        kind: kind.into(),
        label,
        state: String::new(),
        created_at_ms: now,
        verified_at_ms: None,
        next_due_ms: None,
        interval_ms: None,
        summary: String::new(),
        detail: serde_json::Value::Null,
        command: vec![],
        match_text: None,
        matched_at_ms: None,
        pid: None,
        exit_code: None,
        fire_count: 0,
        missed_fires: 0,
        last_fired_at_ms: None,
        last_sequence: None,
        last_delivery: None,
        last_delivered_at_ms: None,
        last_acknowledged_at_ms: None,
        alerted_sequence: None,
    }
}

impl KernelRuntimeOwnedState {
    /// Nothing will start a turn of `agent`: it runs no turn and has none queued, none of its tasks
    /// works, waits or is blocked, the owner did not stop it, and no wake of it is live.
    pub(super) fn agent_has_nothing_armed(
        &self,
        room: &str,
        agent: &str,
    ) -> Result<bool, DaemonError> {
        let session = self.session_store.get_session(room)?;
        if self
            .prompt_state_owner
            .active_prompt_for_agent(&session, agent)
            .is_some()
            || session
                .queued_prompts_for_agent(agent)
                .is_some_and(|queued| !queued.is_empty())
        {
            return Ok(false);
        }
        let store = &self.durable_state_store;
        let tasks = store.agent_tasks(Some(room), Some(agent))?;
        if tasks.iter().any(|task| {
            matches!(
                task.state,
                ExecutionState::Working | ExecutionState::Waiting | ExecutionState::Blocked
            )
        }) || tasks.last().is_some_and(|task| task.stopped_by_owner())
        {
            return Ok(false);
        }
        Ok(!store
            .agent_wakes(Some(room), Some(agent))?
            .iter()
            .any(|wake| matches!(wake.state.as_str(), "scheduled" | "starting" | "running")))
    }

    /// Cell 4: a message waits for its recipient's next turn, and an idle agent with nothing armed
    /// has none coming. Its queued messages are released, so the next delivery starts its turn
    /// with them. True when any were.
    pub(super) fn release_queued_for_stranded(
        &self,
        room: &str,
        agent: &str,
    ) -> Result<bool, DaemonError> {
        let store = &self.durable_state_store;
        let queued = store.agent_queued_messages(room, Some(agent))?;
        if queued.is_empty() || !self.agent_has_nothing_armed(room, agent)? {
            return Ok(false);
        }
        let mut released = false;
        for event in queued {
            // The recipient read it, or the person cancelled it, since the look.
            released |= store
                .agent_lifecycle(Operation::ReleaseQueuedMessage {
                    room: room.into(),
                    agent: agent.into(),
                    sequence: event.sequence,
                })
                .is_ok();
        }
        Ok(released)
    }

    /// The provider process of `run` and the processes of its own the kernel tracks.
    fn provider_pids_of_run(&self, run: &str) -> std::collections::BTreeSet<u32> {
        self.provider_process_tracking
            .read()
            .processes
            .values()
            .filter(|process| process.owner_provider_run_ids.iter().any(|id| id == run))
            .filter_map(|process| process.pid)
            .collect()
    }

    /// Cells 1 and 2, as the agent's turn ends and before the ledger settles it: what would leave
    /// the agent stranded gets a wake, and the agent is told. Never refuses the end of the turn.
    pub(super) fn arm_wakes_at_turn_end(
        &self,
        room: &str,
        agent: &str,
        prompt: &str,
        run: &str,
        answer: &str,
    ) {
        if let Err(error) = self.try_arm_wakes_at_turn_end(room, agent, prompt, run, answer) {
            tracing::warn!(%error, room_id=%room, agent_id=%agent, prompt_id=%prompt,
                "row 554: a turn ended without the wakes the kernel could not arm for it");
        }
    }

    fn try_arm_wakes_at_turn_end(
        &self,
        room: &str,
        agent: &str,
        prompt: &str,
        run: &str,
        answer: &str,
    ) -> Result<(), DaemonError> {
        let store = &self.durable_state_store;
        let Some(task) = store.agent_turn_task(room, agent, prompt)? else {
            return Ok(());
        };
        // A yield is the agent's own choice of what to wait for: the kernel adds nothing to it.
        if task.state != ExecutionState::Working || task.wait.is_some() {
            return Ok(());
        }
        let nothing_armed = !task.obligations.iter().any(|o| o.needs_live_source());
        let now = crate::session::unix_epoch_ms();
        let mut told = Vec::new();

        let covered: std::collections::BTreeSet<u32> = store
            .agent_wakes(Some(room), Some(agent))?
            .iter()
            .filter(|wake| matches!(wake.state.as_str(), "scheduled" | "starting" | "running"))
            .flat_map(|wake| {
                let adopted = serde_json::from_value::<AdoptedDetail>(wake.detail.clone())
                    .ok()
                    .map(|detail| detail.pid);
                wake.pid.into_iter().chain(adopted)
            })
            .collect();
        let found = adopted::left_running(run, &self.provider_pids_of_run(run), &covered);
        for background in found.iter().take(adopted::MAX_ADOPTED) {
            let mut wake = kernel_wake(
                &task,
                adopted::KIND,
                format!("background command: {}", background.command),
                now,
            );
            wake.state = "starting".into();
            wake.pid = Some(background.pid);
            wake.detail = serde_json::to_value(AdoptedDetail {
                pid: background.pid,
                start_ticks: background.start_ticks,
                command: background.command.clone(),
            })
            .map_err(|_| crate::durable_state::agent_lifecycle::error("adopted wake detail"))?;
            match self.arm_wake_source(&task, &wake) {
                Ok(()) => told.push(format!(
                    "your turn ended with `{}` still running and nothing watching it; the kernel adopted it as a wake, so you will be woken when it ends (its exit code is not known to the kernel)",
                    background.command
                )),
                Err(error) => tracing::warn!(%error, command=%background.command,
                    "row 554: a background command could not be adopted"),
            }
        }
        if found.len() > adopted::MAX_ADOPTED {
            told.push(format!(
                "{} more commands were left running and were not adopted",
                found.len() - adopted::MAX_ADOPTED
            ));
        }

        if nothing_armed {
            if let Some(phrase) = stated_wait(answer) {
                let due = now + DEFAULT_WAIT_TIMEOUT_MS;
                let mut wake =
                    kernel_wake(&task, "timer", format!("{BACKSTOP_LABEL}: {phrase}"), now);
                wake.state = "scheduled".into();
                wake.next_due_ms = Some(due);
                match self.arm_wake_source(&task, &wake) {
                    Ok(()) => told.push(format!(
                        "your turn ended saying \"{phrase}\" with nothing armed; the kernel armed a backstop timer for {} (in {} minutes) and will wake you then if nothing else does",
                        clock_utc(due),
                        DEFAULT_WAIT_TIMEOUT_MS / 60_000
                    )),
                    Err(error) => tracing::warn!(%error, "row 554: a backstop timer could not be armed"),
                }
            }
        }

        for line in told {
            self.record_notice_for_agent(
                room,
                None,
                Some(agent),
                self.attachment_store.list_session_attachment_ids(room),
                format!("Kernel: {line}."),
            );
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::stated_wait;

    #[test]
    fn a_stated_wait_is_the_sentence_that_says_it() {
        assert_eq!(
            stated_wait("Pushed the PR. Waiting for the review.").as_deref(),
            Some("Waiting for the review")
        );
        assert_eq!(
            stated_wait("Done.\nI will wake me when the build is green, awaiting the farm")
                .as_deref(),
            Some("I will wake me when the build is green, awaiting the farm")
        );
    }

    #[test]
    fn an_answer_that_waits_for_nothing_states_no_wait() {
        assert_eq!(stated_wait("All done, nothing to wait for."), None);
        assert_eq!(
            stated_wait("The fix is in. I am not waiting for anything."),
            None
        );
        assert_eq!(stated_wait(""), None);
    }

    #[test]
    fn only_the_end_of_a_long_answer_is_read() {
        let long = format!("Waiting for the review. {}", "All done. ".repeat(300));
        assert_eq!(stated_wait(&long), None);
    }
}
