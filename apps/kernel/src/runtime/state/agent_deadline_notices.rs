//! TL1 / TL2: a time limit is announced before it ends and again once it has ended, in the
//! Room where its owner and the agent's parents read, and to the agent itself while it has work.
use super::*;
use crate::durable_state::agent_lifecycle::{occurrence, ExecutionState, Operation};
use crate::local::{AgentDeadline, AgentDeadlineState};

/// How long before the limit each warning goes out, longest first.
const WARNING_LEVELS_MS: [u64; 2] = [10 * 60_000, 60_000];
/// The most sub-agents a notice names; the rest are counted.
const NAMED_CHILDREN: usize = 8;

/// What a limit stops, read before the cleanup removes it.
#[derive(Debug, Default, Clone)]
pub(super) struct DeadlineWork {
    pub active_prompt: Option<String>,
    pub queued_prompts: usize,
    pub wakes: usize,
    pub tasks: usize,
}

/// A notice about the time limit reaches an agent in the middle of its turn: an active turn takes only
/// urgent rows, and a limit that ends it is worth nothing after the fact.
fn urgent(
    mut event: crate::durable_state::agent_lifecycle::InboxEvent,
) -> crate::durable_state::agent_lifecycle::InboxEvent {
    event.urgent = true;
    event
}

fn clock(deadline_ms: u64) -> String {
    i64::try_from(deadline_ms)
        .ok()
        .and_then(chrono::DateTime::from_timestamp_millis)
        .map_or_else(
            || format!("{deadline_ms} ms"),
            |time| time.format("%Y-%m-%d %H:%M:%S UTC").to_string(),
        )
}

fn minutes(level_ms: u64) -> String {
    match level_ms / 60_000 {
        1 => "1 minute".into(),
        minutes => format!("{minutes} minutes"),
    }
}

fn plural(count: usize, one: &str, many: &str) -> String {
    format!("{count} {}", if count == 1 { one } else { many })
}

impl KernelRuntimeState {
    /// One warning per limit and level, 10 minutes and 1 minute before it ends. A limit set
    /// closer than a level gets the nearest level only.
    pub(super) fn warn_of_deadlines(&self, deadlines: &[AgentDeadline], now: u64) {
        for deadline in deadlines
            .iter()
            .filter(|deadline| deadline.state == AgentDeadlineState::Armed)
            .filter(|deadline| deadline.deadline_ms > now)
        {
            let left = deadline.deadline_ms - now;
            let Some(level) = WARNING_LEVELS_MS
                .into_iter()
                .filter(|level| left <= *level)
                .min()
            else {
                continue;
            };
            let key = (
                deadline.session_id.clone(),
                deadline.agent_id.clone(),
                deadline.deadline_ms,
                level,
            );
            if !self.sent_warning(&key, true) {
                continue;
            }
            if let Err(error) = self.send_deadline_warning(deadline, level) {
                tracing::warn!(%error, agent_id=%deadline.agent_id,
                    "the time limit warning is retried at the next tick");
                self.sent_warning(&key, false);
            }
        }
    }

    /// Records (`mark`) or forgets a sent warning; with `mark`, true when it was not sent yet.
    fn sent_warning(&self, key: &(String, String, u64, u64), mark: bool) -> bool {
        let mut sent = self
            .owned
            .deadline_warnings_sent
            .lock()
            .expect("deadline warnings poisoned");
        if mark {
            sent.insert(key.clone())
        } else {
            sent.remove(key)
        }
    }

    fn send_deadline_warning(
        &self,
        deadline: &AgentDeadline,
        level_ms: u64,
    ) -> Result<(), DaemonError> {
        let agent = self.owned.agent_store.get_agent(&deadline.agent_id)?;
        let name = agent.alias().unwrap_or_else(|| agent.id());
        let text = format!(
            "Agent {name}'s time limit ends in about {} (at {}). Then its running turn, queued prompts and waits stop. To keep it working, the owner or its parent extends the limit: `agent deadline {name} <duration|none>`.",
            minutes(level_ms),
            clock(deadline.deadline_ms),
        );
        // The agent hears of it while it has work; an idle one is not woken for it.
        let open_task = self
            .owned
            .durable_state_store
            .agent_tasks(Some(&deadline.session_id), Some(&deadline.agent_id))?
            .into_iter()
            .find(|task| !matches!(task.state, ExecutionState::Done | ExecutionState::Cancelled));
        if let Some(task) = open_task {
            self.owned
                .durable_state_store
                .agent_lifecycle(Operation::Occur(urgent(occurrence(
                    &deadline.session_id,
                    &deadline.agent_id,
                    &format!("deadline:{}", deadline.agent_id),
                    &format!("{}:{level_ms}", deadline.deadline_ms),
                    "time_limit_warning",
                    serde_json::json!({
                        "task_id": task.task_id,
                        "deadline_ms": deadline.deadline_ms,
                        "message": text,
                    }),
                ))))?;
        }
        self.owned.record_notice_for_agent(
            &deadline.session_id,
            None,
            Some(&deadline.agent_id),
            self.owned
                .attachment_store
                .list_session_attachment_ids(&deadline.session_id),
            text,
        );
        Ok(())
    }

    /// What the limit of `agent` is about to stop.
    pub(super) fn deadline_work(&self, room: &str, agent: &str) -> DeadlineWork {
        let mut work = DeadlineWork::default();
        if let Ok(session) = self.owned.session_store.get_session(room) {
            let (active, queued) = self.owned.prompt_state_owner.state_parts(&session, agent);
            work.active_prompt = active.map(|prompt| prompt.id().to_string());
            work.queued_prompts = queued.len();
        }
        work.wakes = self
            .owned
            .unsettled_agent_wakes(room, agent)
            .map_or(0, |wakes| wakes.len());
        work.tasks = self
            .owned
            .durable_state_store
            .agent_tasks(Some(room), Some(agent))
            .map_or(0, |tasks| {
                tasks
                    .iter()
                    .filter(|task| {
                        !matches!(task.state, ExecutionState::Done | ExecutionState::Cancelled)
                    })
                    .count()
            });
        work
    }

    /// What the limit stops, as the first cleanup attempt found it. Later attempts see less,
    /// because earlier ones already removed queued prompts and ended the turn.
    pub(super) fn deadline_work_at_reach(&self, deadline: &AgentDeadline) -> DeadlineWork {
        if deadline.stopped_at_ms.is_some() {
            // A finished cleanup is not announced again.
            return self.deadline_work(&deadline.session_id, &deadline.agent_id);
        }
        let key = (
            deadline.session_id.clone(),
            deadline.agent_id.clone(),
            deadline.deadline_ms,
        );
        self.owned
            .deadline_work_found
            .lock()
            .expect("deadline work poisoned")
            .entry(key)
            .or_insert_with(|| self.deadline_work(&deadline.session_id, &deadline.agent_id))
            .clone()
    }

    /// Drops the retained summaries of limits that are no longer waiting for their notice.
    pub(super) fn forget_deadline_work(&self, deadlines: &[AgentDeadline]) {
        self.owned
            .deadline_work_found
            .lock()
            .expect("deadline work poisoned")
            .retain(|(room, agent, deadline_ms), _| {
                deadlines.iter().any(|deadline| {
                    deadline.stopped_at_ms.is_none()
                        && deadline.deadline_ms == *deadline_ms
                        && &deadline.session_id == room
                        && &deadline.agent_id == agent
                })
            });
    }

    /// TL2: the Room is told once, when the limit's cleanup finished, what stopped and what did not.
    pub(super) fn announce_deadline_stop(&self, deadline: &AgentDeadline, work: &DeadlineWork) {
        let Ok(agent) = self.owned.agent_store.get_agent(&deadline.agent_id) else {
            return;
        };
        let name = agent.alias().unwrap_or_else(|| agent.id()).to_string();
        let mut stopped = Vec::new();
        if let Some(prompt) = &work.active_prompt {
            stopped.push(format!("its running turn ({prompt})"));
        }
        if work.queued_prompts > 0 {
            stopped.push(plural(
                work.queued_prompts,
                "queued prompt",
                "queued prompts",
            ));
        }
        if work.wakes > 0 {
            stopped.push(plural(work.wakes, "wait", "waits"));
        }
        let stopped = if stopped.is_empty() {
            "It had no running turn, queued prompt or wait.".to_string()
        } else {
            format!("Stopped: {}.", stopped.join(", "))
        };
        let children = self.deadline_children_line(&deadline.agent_id, &name);
        self.tell_children_of_stop(deadline, &name);
        self.owned.record_notice_entry(
            {
                let mut notice = crate::history::SessionHistoryEntry::notice(
                    &deadline.session_id,
                    None,
                    Some(&deadline.agent_id),
                    format!(
                        "Agent {name}'s time limit ended at {}. {stopped} {children} To continue the same agent with its context, the owner or its parent sets a new limit with `agent deadline {name} <duration|none>`, then prompts it.",
                        clock(deadline.deadline_ms),
                    ),
                );
                notice.notice_reason =
                    Some(crate::history::SessionHistoryNoticeReason::AgentDeadlineReached);
                notice
            },
            self.owned
                .attachment_store
                .list_session_attachment_ids(&deadline.session_id),
        );
    }

    fn deadline_children_line(&self, agent: &str, name: &str) -> String {
        let children = self
            .owned
            .agent_store
            .list_agents()
            .into_iter()
            .filter(|child| child.parent_agent_id() == Some(agent))
            .collect::<Vec<_>>();
        if children.is_empty() {
            return format!("{name} has no sub-agents.");
        }
        let named = children
            .iter()
            .take(NAMED_CHILDREN)
            .map(|child| {
                let state = if self
                    .agent_work_remains(child.session_id(), child.id())
                    .unwrap_or(false)
                {
                    "working"
                } else {
                    "idle"
                };
                format!("{} ({state})", child.alias().unwrap_or_else(|| child.id()))
            })
            .collect::<Vec<_>>()
            .join(", ");
        let more = children.len().saturating_sub(NAMED_CHILDREN);
        format!(
            "Its sub-agents were not stopped: {named}{}.",
            if more > 0 {
                format!(" and {more} more")
            } else {
                String::new()
            }
        )
    }

    /// Whether the agent's own limit ended its run.
    fn limit_ended(&self, agent: &str, room: &str) -> bool {
        self.owned
            .durable_state_store
            .agent_deadline(room, agent)
            .ok()
            .flatten()
            .is_some_and(|deadline| deadline.state == AgentDeadlineState::Reached)
    }

    /// The nearest agent above `agent` that still runs: not archived, its own limit not ended.
    /// Worked out when asked, so extending a stopped ancestor gives the role back.
    fn living_ancestor(
        &self,
        agent: &crate::agent::AgentInstance,
    ) -> Option<crate::agent::AgentInstance> {
        let mut next = agent.parent_agent_id().map(str::to_owned);
        while let Some(id) = next {
            let Ok(parent) = self.owned.agent_store.get_agent(&id) else {
                break;
            };
            if !parent.archived() && !self.limit_ended(parent.id(), parent.session_id()) {
                return Some(parent);
            }
            next = parent.parent_agent_id().map(str::to_owned);
        }
        None
    }

    /// TL4: who hears from `agent` now that a parent above it has stopped. The nearest agent above
    /// it that still runs, or the owner who reads the Room when there is none.
    pub(super) fn receiver_line(&self, agent: &crate::agent::AgentInstance) -> String {
        match self.living_ancestor(agent) {
            Some(parent) => format!(
                "Send your results and questions to {} with chariox.send_agent_message.",
                parent.alias().unwrap_or_else(|| parent.id())
            ),
            None => "No agent above you runs now: the owner reads the Room, so say what you need in your final answer.".to_string(),
        }
    }

    /// Row 754 (D189): a time limit is the parent agent's setting, so an ended limit tells the
    /// PARENT agent, as one urgent event naming the two things it can do: extend the limit with
    /// `agent deadline` and prompt the agent again, or let it stay stopped. Never a person card:
    /// a top-level agent with no parent raises nothing here — its Room notice is its whole story,
    /// and the owner's external agent may extend or clear the limit itself (D190b).
    pub(super) async fn tell_parent_limit_ended(&self, room: &str, agent: &str) {
        let deadline = self
            .owned
            .durable_state_store
            .agent_deadline(room, agent)
            .ok()
            .flatten()
            .filter(|deadline| {
                deadline.state == AgentDeadlineState::Reached
                    || deadline.deadline_ms <= crate::session::unix_epoch_ms()
            });
        let Some(deadline) = deadline else {
            return;
        };
        if let Err(error) = self.send_limit_ended_notice(&deadline).await {
            tracing::warn!(%error, agent_id=%agent,
                "the parent's time limit notice is retried at the next refusal");
        }
    }

    /// One durable urgent event per ended limit, to the nearest living ancestor: a reload, a
    /// restart or a repeated refusal re-reads the same occurrence instead of raising another.
    pub(super) async fn send_limit_ended_notice(
        &self,
        deadline: &AgentDeadline,
    ) -> Result<(), DaemonError> {
        let child = self.owned.agent_store.get_agent(&deadline.agent_id)?;
        let Some(parent) = self.living_ancestor(&child) else {
            return Ok(());
        };
        let source = format!("deadline:{}", deadline.agent_id);
        let occurrence_id = format!("{}:ended", deadline.deadline_ms);
        let already_told = self
            .owned
            .durable_state_store
            .agent_inbox_occurrence(&deadline.session_id, &parent.id(), &occurrence_id)?
            .into_iter()
            .any(|event| event.source_id == source);
        if !already_told {
            let name = child.alias().unwrap_or_else(|| child.id()).to_string();
            let text = format!(
                "Your agent {name}'s time limit ended at {}. It stopped and takes no new work. You own its limit: extend it with `agent deadline {name} <duration|none>` and prompt it again, or let it stay stopped. Its sub-agents keep their own work.",
                clock(deadline.deadline_ms),
            );
            let event = urgent(occurrence(
                &deadline.session_id,
                &parent.id(),
                &source,
                &occurrence_id,
                "child_time_limit_ended",
                serde_json::json!({
                    "deadline_ms": deadline.deadline_ms,
                    "child_agent_id": deadline.agent_id,
                    "message": text,
                }),
            ));
            self.owned
                .durable_state_store
                .agent_lifecycle(Operation::Occur(event))?;
        }
        // Boxed: the delivery of this notice can itself be refused by the parent's own ended
        // limit, which walks further up instead of recursing here.
        Box::pin(self.deliver_agent_inbox(&deadline.session_id, &parent.id())).await?;
        Ok(())
    }

    /// Whether a limit that ended stands between `agent` and its parent.
    pub(super) fn parent_limit_ended(&self, agent: &crate::agent::AgentInstance) -> bool {
        agent.parent_agent_id().is_some_and(|parent| {
            self.owned
                .agent_store
                .get_agent(parent)
                .is_ok_and(|parent| self.limit_ended(parent.id(), parent.session_id()))
        })
    }

    /// TL4: a sub-agent keeps working when its parent's limit ends, and is told who hears it now.
    fn tell_children_of_stop(&self, deadline: &AgentDeadline, parent_name: &str) {
        for child in self
            .owned
            .agent_store
            .list_agents()
            .into_iter()
            .filter(|child| child.parent_agent_id() == Some(deadline.agent_id.as_str()))
        {
            let open_task = self
                .owned
                .durable_state_store
                .agent_tasks(Some(child.session_id()), Some(child.id()))
                .ok()
                .and_then(|tasks| {
                    tasks.into_iter().find(|task| {
                        !matches!(task.state, ExecutionState::Done | ExecutionState::Cancelled)
                    })
                });
            let Some(task) = open_task else { continue };
            let text = format!(
                "Your parent {parent_name}'s time limit ended at {}. It stopped and no longer receives messages. You keep running. {}",
                clock(deadline.deadline_ms),
                self.receiver_line(&child),
            );
            if let Err(error) = self
                .owned
                .durable_state_store
                .agent_lifecycle(Operation::Occur(urgent(occurrence(
                    child.session_id(),
                    child.id(),
                    &format!("deadline:{}", deadline.agent_id),
                    &format!("{}:parent-ended", deadline.deadline_ms),
                    "parent_time_limit_ended",
                    serde_json::json!({
                        "task_id": task.task_id,
                        "parent_agent_id": deadline.agent_id,
                        "message": text,
                    }),
                ))))
            {
                tracing::warn!(%error, agent_id=%child.id(),
                    "a sub-agent was not told that its parent's limit ended");
            }
        }
    }
}
