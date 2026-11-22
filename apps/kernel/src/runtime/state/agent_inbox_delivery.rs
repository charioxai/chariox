//! MP-08 / MP-09 / MP-10 / MP-11 A02: durable intent before official provider I/O.
use super::*;
use crate::durable_state::agent_lifecycle::{self as ledger, ExecutionState, Operation, Outcome};
/// Rows one wake may carry beyond its lead; later rows get the next wake.
const MAX_WAKE_BATCH: usize = 63;
/// How long a steer waits for the running turn's provider lane.
const STEER_LANE_WAIT: std::time::Duration = std::time::Duration::from_secs(2);

fn names_task(event: &ledger::InboxEvent, task: &str) -> bool {
    event.payload["task_id"].as_str() == Some(task)
        || event.payload["task_ids"]
            .as_array()
            .is_some_and(|ids| ids.iter().any(|id| id.as_str() == Some(task)))
}

enum WakeDisposition {
    Retire,
    Hold,
    Deliver,
}
/// What the states of the tasks an event names say about its wake: the
/// source occurrence and a finished or blocked task keep their outcome,
/// so the automatic wake is retired; a proven startup refusal keeps its
/// answer for an explicit Resume.
fn wake_disposition(
    event: &ledger::InboxEvent,
    tasks: &[ledger::AgentTaskExecution],
) -> WakeDisposition {
    // A peer event names the turn that created its persistent follow,
    // not unfinished work whose wake expires when that turn ends. Work the ended turn left
    // running reports its end to the agent, not to the turn.
    if event.kind == "peer_event" || event.payload["after_turn"] == true {
        return WakeDisposition::Deliver;
    }
    let mut ids = Vec::new();
    if let Some(id) = event.payload["task_id"].as_str() {
        ids.push(id);
    }
    if let Some(more) = event.payload["task_ids"].as_array() {
        ids.extend(more.iter().filter_map(|v| v.as_str()));
    }
    if !ids.is_empty()
        && ids.iter().all(|id| {
            tasks
                .iter()
                .find(|t| t.task_id == *id)
                .is_none_or(|t| matches!(t.state, ExecutionState::Done | ExecutionState::Cancelled))
        })
    {
        return WakeDisposition::Retire;
    }
    let waiting = |id: &&str| {
        tasks
            .iter()
            .any(|t| t.task_id == *id && t.state == ExecutionState::Waiting)
    };
    if ids.iter().any(|id| {
        tasks
            .iter()
            .any(|t| t.task_id == *id && t.state == ExecutionState::Blocked)
    }) && !ids.iter().any(waiting)
    {
        if event.attempted_at_ms.is_some()
            && event.submit_epoch.is_none()
            && tasks.iter().any(|task| {
                task.state == ExecutionState::Blocked
                    && task.wait.is_some()
                    && ids.contains(&task.task_id.as_str())
            })
        {
            return WakeDisposition::Hold;
        }
        return WakeDisposition::Retire;
    }
    WakeDisposition::Deliver
}
impl KernelRuntimeState {
    pub(super) fn agent_inbox_waits_for_vault(&self, room: &str, agent: &str) -> bool {
        if !self
            .owned
            .agent_store
            .get_agent(agent)
            .is_ok_and(|agent| agent.remote_execution().is_none())
        {
            return false;
        }
        self.owned
            .session_store
            .get_session(room)
            .is_ok_and(|session| {
                session.active_interactions().iter().any(|card| {
                    card.agent_id() == Some(agent)
                        && card
                            .choices()
                            .iter()
                            .any(|choice| choice.vault_unlock().is_some())
                })
            })
    }

    /// The prompt carries these events' text, so they count as read: a turn
    /// that never opens its inbox must not hand the rows back to pending.
    pub(super) fn mark_prompt_events_shown(
        &self,
        room: &str,
        agent: &str,
        sequences: &[u64],
    ) -> Result<(), DaemonError> {
        if sequences.is_empty() {
            return Ok(());
        }
        self.owned
            .durable_state_store
            .agent_lifecycle(Operation::MarkShown {
                room: room.into(),
                agent: agent.into(),
                sequences: sequences.to_vec(),
            })?;
        Ok(())
    }

    /// A message from the parent to its sub-agent, alone in its wake, as the
    /// prompt text: the sub-agent starts on its task without reading its inbox.
    pub(super) fn parent_brief(
        &self,
        agent: &str,
        event: &ledger::InboxEvent,
        batch: &[ledger::InboxEvent],
    ) -> Option<String> {
        if event.kind != "message" || event.reply_requested || !batch.is_empty() {
            return None;
        }
        let message = event.payload["message"].as_str()?;
        let target = self.owned.agent_store.get_agent(agent).ok()?;
        let parent = self
            .owned
            .agent_store
            .get_agent(
                target
                    .parent_agent_id()
                    .filter(|id| *id == event.source_id)?,
            )
            .ok()?;
        let name = parent.alias().unwrap_or(parent.agent_ref());
        Some(super::agent_inbox_wake_text::brief_text(
            name,
            event.sequence,
            message,
        ))
    }

    pub(super) async fn deliver_agent_inbox(
        &self,
        room: &str,
        agent: &str,
    ) -> Result<(), DaemonError> {
        if self.owned.agent_store.get_agent(agent)?.archived() {
            return Ok(());
        }
        let tasks = self
            .owned
            .durable_state_store
            .agent_tasks(Some(room), Some(agent))?;
        let session = self.owned.session_store.get_session(room)?;
        let active = self
            .owned
            .prompt_state_owner
            .active_prompt_for_agent(&session, agent);
        if active.is_none()
            && self
                .owned
                .prompt_state_owner
                .peek_next_queued_prompt(&session, agent)
                .is_some()
        {
            return Ok(());
        }
        // Checks use a separate tool-free conversation. Keep ordinary work in
        // the inbox until its agent can handle it in a work turn.
        if active.as_ref().is_some_and(|prompt| {
            prompt.prompt_origin() == crate::session::PromptOrigin::CompletionCheck
        }) {
            return Ok(());
        }
        // A04 causal fence: during sudo-bound work only that task's correlated
        // wakes are delivered (never by steering); other events stay pending
        // visibly and run as regular turns once the window ends.
        let work = self.owned.sudo_work_task(room, agent);
        if let Some(work) = work.as_deref() {
            if active.is_some() {
                return Ok(());
            }
            self.defer_unrelated_sudo_event(room, agent, work)?;
        }
        let event = loop {
            let front = if active.is_some() {
                self.owned
                    .durable_state_store
                    .agent_urgent_delivery_front(room, agent)?
            } else {
                self.owned.durable_state_store.agent_work_delivery_front(
                    room,
                    agent,
                    work.as_deref(),
                )?
            };
            let Some(event) = front else {
                return Ok(());
            };
            if event.state == "pending" && event.kind != "message" {
                match wake_disposition(&event, &tasks) {
                    WakeDisposition::Retire => {
                        self.owned
                            .durable_state_store
                            .agent_lifecycle(Operation::Expire {
                                room: room.into(),
                                agent: agent.into(),
                                sequence: event.sequence,
                            })?;
                        continue;
                    }
                    WakeDisposition::Hold => match self.urgent_past_held_wake(
                        room,
                        agent,
                        &event,
                        &tasks,
                        work.as_deref(),
                    )? {
                        Some(urgent) => break urgent,
                        None => return Ok(()),
                    },
                    WakeDisposition::Deliver => {}
                }
            }
            break event;
        };
        if event.state != "pending" {
            return Ok(());
        }
        // An explicitly stopped agent stays stopped: messages, urgent ones too,
        // stay pending and visible until the owner prompts it again.
        if active.is_none()
            && event.kind == "message"
            && tasks.last().is_some_and(|t| t.stopped_by_owner())
        {
            return self.hold_message_for_stopped_agent(room, agent, &event);
        }
        if active.is_none()
            && event.kind != "message"
            && tasks.iter().any(|t| {
                (event.payload["task_id"].as_str() == Some(t.task_id.as_str())
                    || event.payload["task_ids"].as_array().is_some_and(|ids| {
                        ids.iter().any(|id| id.as_str() == Some(t.task_id.as_str()))
                    }))
                    && (t.pending_prompt_id.is_some()
                        || session
                            .queued_prompts_for_agent(agent)
                            .is_some_and(|q| q.iter().any(|p| p.id() == t.prompt_id)))
            })
        {
            return Ok(());
        }
        if active.is_some() && (!event.urgent || event.attempted_at_ms.is_some()) {
            return Ok(());
        }
        if let child_finish_window::Window::Open { left, first } =
            self.sibling_window(room, agent, &event)
        {
            if first {
                self.deliver_when_window_ends(room, agent, left);
            }
            return Ok(());
        }
        let target = self.owned.agent_store.get_agent(agent)?;
        let leased = target.remote_execution().is_some();
        // Take the provider lane before choosing the batch. Other arrivals while
        // the turn holds the lane remain pending until the same delivery opportunity.
        let steer_lane = if let Some(active) = active.as_ref().filter(|_| !leased) {
            let Some(run) = self.owned.provider_store.get_run_for_agent(room, agent) else {
                self.defer_inbox_steer(room, agent, event.sequence)?;
                return Ok(());
            };
            let Ok(lane) = tokio::time::timeout(
                STEER_LANE_WAIT,
                self.provider_runtime_lanes.acquire(run.id()),
            )
            .await
            else {
                self.schedule_agent_lifecycle_sweep();
                return Ok(());
            };
            let session = self.owned.session_store.get_session(room)?;
            let same_turn = self
                .owned
                .prompt_state_owner
                .active_prompt_for_agent(&session, agent)
                .is_some_and(|current| current.id() == active.id());
            let still_first = self
                .owned
                .durable_state_store
                .agent_urgent_delivery_front(room, agent)?
                .is_some_and(|front| front.sequence == event.sequence && front.state == "pending");
            if !same_turn || !still_first {
                self.schedule_agent_lifecycle_sweep();
                return Ok(());
            }
            Some((run.id().to_owned(), lane))
        } else {
            None
        };
        let batch = self.delivery_batch(
            room,
            agent,
            &event,
            &tasks,
            &session,
            active.is_some(),
            work.as_deref(),
        )?;
        let through = batch.last().map_or(event.sequence, |last| last.sequence);
        let attachment = self.ensure_agent_message_attachment(room, &target)?;
        let workflow = match tasks.iter().find(|t| {
            t.state == ExecutionState::Waiting
                && (event.payload["task_id"].as_str() == Some(t.task_id.as_str())
                    || event.payload["task_ids"].as_array().is_some_and(|ids| {
                        ids.iter().any(|id| id.as_str() == Some(t.task_id.as_str()))
                    }))
        }) {
            Some(task) => self.owned.agent_workflow_task_context(task)?,
            None => None,
        };
        let workflow_attachment = workflow
            .as_ref()
            .map(|(run, _)| crate::scheduler::runtime::workflow_prompt_source_attachment_id(run));
        let prompt_id = event.transport_prompt_id(leased);
        let after = self
            .owned
            .durable_state_store
            .agent_read_cursor(&event, work.as_deref())?;
        let (text, shown) = match self.parent_brief(agent, &event, &batch) {
            Some(brief) => (brief, vec![event.sequence]),
            None => {
                let wake = self.inline_wake(room, agent, &event, &batch, after)?;
                (wake.text, wake.shown)
            }
        };
        let attachments = std::iter::once(&event)
            .chain(&batch)
            .map(|row| {
                row.payload
                    .get("attachments")
                    .cloned()
                    .map(serde_json::from_value::<Vec<_>>)
                    .transpose()
                    .map_err(|_| ledger::error("corrupt message attachments"))
            })
            .collect::<Result<Vec<_>, _>>()?
            .into_iter()
            .flatten()
            .flatten()
            .collect::<Vec<_>>();
        let prompt = crate::session::PromptQueueItem::new(
            &prompt_id,
            workflow_attachment.as_deref().unwrap_or(&attachment),
            agent,
            text,
            crate::session::PromptStatus::Queued,
        )
        .with_durable_operation(&prompt_id, format!("event:{}", event.sequence))
        .with_attachments(attachments);
        let prompt = match workflow {
            Some((run, node)) => prompt.with_workflow_context(run, node),
            None => prompt,
        };
        if let Some(active) = active.as_ref().filter(|_| leased) {
            return self
                .steer_leased_inbox_event(
                    room,
                    agent,
                    event.sequence,
                    through,
                    active,
                    prompt,
                    work,
                    &shown,
                )
                .await;
        }
        let steer = if let Some((run, _lane)) = &steer_lane {
            match self.prepare_local_active_prompt_steer_dispatch(room, &prompt) {
                Ok(Some(dispatch)) if &dispatch.provider_run_id == run => Some(dispatch),
                _ => {
                    self.defer_inbox_steer(room, agent, event.sequence)?;
                    return Ok(());
                }
            }
        } else {
            None
        };
        let run = steer.as_ref().map(|d| d.provider_run_id.clone());
        let target_prompt = steer
            .as_ref()
            .and_then(|d| d.target_active_prompt_id.clone());
        let waiting = self.owned.waiting_tasks(room, agent);
        let Outcome::Event(attempt) =
            self.owned
                .durable_state_store
                .agent_lifecycle(Operation::Attempt {
                    room: room.into(),
                    agent: agent.into(),
                    sequence: event.sequence,
                    prompt: prompt_id.clone(),
                    target: target_prompt,
                    run,
                    now: crate::session::unix_epoch_ms(),
                    work,
                    through,
                })?
        else {
            unreachable!()
        };
        self.owned.record_woken_waits(&waiting, &attempt);
        self.mark_prompt_events_shown(room, agent, &shown)?;
        if let Some(dispatch) = steer {
            if !self
                .owned
                .prompt_dispatch_matches_active_prompt(&dispatch)
                .unwrap_or(false)
            {
                self.record_agent_delivery_receipt(room, agent, event.sequence, "rejected")?;
                self.deliver_skipped_steer_again(room, agent, event.sequence, &dispatch, None);
                return Ok(());
            }

            // The provider reads the wake as a user message of its running
            // turn. Recorded before the write, the way a queued steer is, the
            // transcript observer knows the message as kernel-owned instead of
            // importing it, and the answer after it, as an external turn.
            if let Some(active_prompt_id) = dispatch.target_active_prompt_id.as_deref() {
                let sequence = self.owned.append_steering_prompt_history(
                    &dispatch.session_id,
                    &dispatch.provider_run_id,
                    active_prompt_id,
                    &dispatch.source_attachment_id,
                    &dispatch.agent_id,
                    &dispatch.prompt_id,
                    &dispatch.prompt,
                    &dispatch.attachments,
                    dispatch.prompt_origin,
                )?;
                self.owned.echo_steering_prompt_to_other_attachments(
                    &dispatch.session_id,
                    &dispatch.provider_run_id,
                    &dispatch.agent_id,
                    &dispatch.prompt_id,
                    &dispatch.source_attachment_id,
                    &dispatch.source_attachment_id,
                    &dispatch.prompt,
                    &dispatch.attachments,
                    dispatch.prompt_origin,
                    Some(sequence),
                );
            }
            let result = self
                .enqueue_prompt_dispatch_with_acceptance(&dispatch)
                .await;
            self.record_agent_event_dispatch_result(&dispatch, &result)?;
            // The shared dispatch checks the target turn again at the native write: a turn that
            // ended in between took nothing, and the row is pending again.
            if self.inbox_row_is_pending(room, agent, event.sequence)? {
                let error = result.err().map(|error| error.to_string());
                self.deliver_skipped_steer_again(
                    room,
                    agent,
                    event.sequence,
                    &dispatch,
                    error.as_deref(),
                );
                return Ok(());
            }
            result?;
        } else {
            let result = Box::pin(self.submit_prepared_prompt_with_queue_policy(
                crate::app::KernelPreparedPromptSubmission {
                    session_id: room.into(),
                    prompt,
                    force_queue: false,
                    refresh_projection: true,
                },
                false,
            ))
            .await;
            match result {
                Ok(mut submission) => {
                    // A10: home ordering is already committed; the worker's exact
                    // receipt settles this attempt through the leased dispatch.
                    if let Some(dispatch) = submission.remote_dispatch.take() {
                        self.spawn_remote_prompt_dispatch(dispatch);
                    }
                    if let Some(dispatch) = submission.dispatch.take() {
                        // The caller can hold another run's lane (a child's settling turn wakes
                        // its parent), and a holder of the recipient's lane can wait for that
                        // one: a held recipient lane is waited for outside the caller.
                        match self
                            .provider_runtime_lanes
                            .try_acquire(&dispatch.provider_run_id)
                        {
                            Some(_permit) => self.dispatch_agent_event_prompt(dispatch).await?,
                            None => self.spawn_agent_event_dispatch(dispatch),
                        }
                    }
                }
                // The agent took a turn between the check and the start: the event stays pending
                // and goes into its next turn. Not a failed start, and nobody is told to retry.
                Err(DaemonError::LocalTransport { message, .. })
                    if message == crate::runtime::prompt_state::TARGET_BUSY_MESSAGE =>
                {
                    self.record_agent_delivery_receipt(room, agent, event.sequence, "rejected")?;
                }
                Err(e) => {
                    if let DaemonError::VaultUnlock {
                        code: "vault_unlock_declined",
                        retryable: false,
                        ..
                    } = &e
                    {
                        self.owned.block_vault_refused_inbox_start(
                            room,
                            agent,
                            event.sequence,
                            &e,
                        )?;
                    }
                    let state = ledger::delivery_receipt_state(None, false);
                    self.record_agent_delivery_receipt(room, agent, event.sequence, state)?;
                    if !crate::error::is_agent_deadline_refusal(&e) {
                        self.owned
                            .settle_failed_delegate_start(room, agent, &e.to_string())?;
                    }
                    self.publish_task_change(room);
                    return Err(e);
                }
            }
        }
        Ok(())
    }
    fn inbox_row_is_pending(
        &self,
        room: &str,
        agent: &str,
        sequence: u64,
    ) -> Result<bool, DaemonError> {
        Ok(self
            .owned
            .durable_state_store
            .agent_work_inbox(room, agent, sequence.saturating_sub(1), None)?
            .into_iter()
            .find(|row| row.sequence == sequence)
            .is_some_and(|row| row.state == "pending"))
    }

    /// A steer whose target turn ended before the native write took nothing: its row is
    /// pending again and goes to the agent's next turn, started now if the agent is idle.
    fn deliver_skipped_steer_again(
        &self,
        room: &str,
        agent: &str,
        sequence: u64,
        dispatch: &crate::app::KernelPromptDispatch,
        error: Option<&str>,
    ) {
        crate::logging::warn_with_fields(
            "runtime.agent_inbox",
            "inbox steer skipped before native write; event is pending and delivered again",
            serde_json::json!({
                "session_id": room,
                "agent_id": agent,
                "sequence": sequence,
                "dispatch_prompt_id": dispatch.prompt_id,
                "target_active_prompt_id": dispatch.target_active_prompt_id,
                "provider_run_id": dispatch.provider_run_id,
                "error": error.map(crate::secret_redaction::redact_secrets),
            }),
        );
        self.spawn_agent_inbox_delivery(room.to_owned(), agent.to_owned(), sequence);
    }

    fn defer_inbox_steer(&self, room: &str, agent: &str, sequence: u64) -> Result<(), DaemonError> {
        self.owned
            .durable_state_store
            .agent_lifecycle(Operation::Defer {
                room: room.into(),
                agent: agent.into(),
                sequence,
                now: crate::session::unix_epoch_ms(),
            })?;
        Ok(())
    }

    /// The caller holds the dispatch's run lane.
    async fn dispatch_agent_event_prompt(
        &self,
        dispatch: crate::app::KernelPromptDispatch,
    ) -> Result<(), DaemonError> {
        let result = self
            .enqueue_prompt_dispatch_with_acceptance(&dispatch)
            .await;
        self.record_agent_event_dispatch_result(&dispatch, &result)?;
        // The receipt above is exact; the shared failure path then
        // frees the active prompt the way a spawned dispatch does.
        if let Err(error) = result {
            self.fail_prompt_dispatch(dispatch, error).await?;
        }
        Ok(())
    }

    fn spawn_agent_event_dispatch(&self, dispatch: crate::app::KernelPromptDispatch) {
        let state = self.clone();
        tokio::spawn(async move {
            let _permit = state
                .provider_runtime_lanes
                .acquire(&dispatch.provider_run_id)
                .await;
            let prompt_id = dispatch.prompt_id.clone();
            if let Err(error) = state.dispatch_agent_event_prompt(dispatch).await {
                tracing::warn!(%error, %prompt_id,
                    "inbox dispatch result retained for the inbox sweep");
            }
        });
    }

    /// The rows one wake carries with its lead row, nothing waiting for more:
    /// a steer wake every pending urgent row, an idle or queued wake every
    /// pending row. Rows whose task finished are retired on the way, and the
    /// batch ends where the lead's own rules would hold a row back.
    #[allow(clippy::too_many_arguments)]
    fn delivery_batch(
        &self,
        room: &str,
        agent: &str,
        lead: &ledger::InboxEvent,
        tasks: &[ledger::AgentTaskExecution],
        session: &crate::session::RuntimeSession,
        steer: bool,
        work: Option<&str>,
    ) -> Result<Vec<ledger::InboxEvent>, DaemonError> {
        let stopped = tasks.last().is_some_and(|t| t.stopped_by_owner());
        let mut batch = Vec::new();
        for next in self
            .owned
            .durable_state_store
            .agent_delivery_batch(lead, steer, work)?
        {
            if batch.len() >= MAX_WAKE_BATCH {
                break;
            }
            if next.kind == "message" {
                if stopped && !steer {
                    break;
                }
            } else {
                match wake_disposition(&next, tasks) {
                    WakeDisposition::Retire => {
                        self.owned
                            .durable_state_store
                            .agent_lifecycle(Operation::Expire {
                                room: room.into(),
                                agent: agent.into(),
                                sequence: next.sequence,
                            })?;
                        continue;
                    }
                    WakeDisposition::Hold => break,
                    WakeDisposition::Deliver => {}
                }
                if !steer
                    && tasks.iter().any(|t| {
                        names_task(&next, &t.task_id)
                            && (t.pending_prompt_id.is_some()
                                || session
                                    .queued_prompts_for_agent(agent)
                                    .is_some_and(|q| q.iter().any(|p| p.id() == t.prompt_id)))
                    })
                {
                    break;
                }
            }
            batch.push(next);
        }
        Ok(batch)
    }
    /// What the sender is told when message `sequence` waits for the owner's next prompt to
    /// `agent`, which the owner stopped: no turn is coming, so it must not wait for an answer.
    pub(super) fn held_for_stopped_agent_note(
        &self,
        room: &str,
        agent: &str,
        sequence: u64,
    ) -> Result<Option<String>, DaemonError> {
        let session = self.owned.session_store.get_session(room)?;
        let store = &self.owned.durable_state_store;
        let waits = self
            .owned
            .prompt_state_owner
            .active_prompt_for_agent(&session, agent)
            .is_none()
            && store
                .agent_tasks(Some(room), Some(agent))?
                .last()
                .is_some_and(|task| task.stopped_by_owner())
            && store
                .agent_work_inbox(room, agent, 0, None)?
                .iter()
                .any(|event| event.sequence == sequence && event.state == "pending");
        Ok(waits.then(|| {
            let name = self.agent_notice_name(agent);
            format!(
                "{name} was stopped by its owner and runs nothing until the owner's next prompt to it; this message is held until then. Do not wait for an answer."
            )
        }))
    }
    /// The urgent row that starts a turn while the front is a wake held for its blocked task's
    /// Resume: the held wake keeps its answer for the owner, and an urgent message, or an urgent
    /// wake of other work, does not wait for that answer. A sudo-bound window keeps its fence.
    fn urgent_past_held_wake(
        &self,
        room: &str,
        agent: &str,
        held: &ledger::InboxEvent,
        tasks: &[ledger::AgentTaskExecution],
        work: Option<&str>,
    ) -> Result<Option<ledger::InboxEvent>, DaemonError> {
        if work.is_some() {
            return Ok(None);
        }
        let Some(urgent) = self
            .owned
            .durable_state_store
            .agent_urgent_delivery_front(room, agent)?
            .filter(|urgent| {
                urgent.state == "pending"
                    && (urgent.kind == "message"
                        || matches!(
                            self.wake_disposition(urgent, tasks),
                            WakeDisposition::Deliver
                        ))
            })
        else {
            return Ok(None);
        };
        crate::logging::info_with_fields(
            "runtime.agent_inbox",
            "urgent inbox event passes a wake held for its blocked task's Resume",
            serde_json::json!({
                "session_id": room,
                "agent_id": agent,
                "sequence": urgent.sequence,
                "held_sequence": held.sequence,
                "held_task_ids": tasks
                    .iter()
                    .filter(|t| t.state == ExecutionState::Blocked && names_task(held, &t.task_id))
                    .map(|t| t.task_id.as_str())
                    .collect::<Vec<_>>(),
            }),
        );
        Ok(Some(urgent))
    }
    /// Marks the oldest unrelated pending event deferred once and says so.
    /// The owner sees once that a message waits for the agent they stopped.
    fn hold_message_for_stopped_agent(
        &self,
        room: &str,
        agent: &str,
        event: &ledger::InboxEvent,
    ) -> Result<(), DaemonError> {
        if event.attempted_at_ms.is_some() {
            return Ok(());
        }
        self.owned
            .durable_state_store
            .agent_lifecycle(Operation::Defer {
                room: room.into(),
                agent: agent.into(),
                sequence: event.sequence,
                now: crate::session::unix_epoch_ms(),
            })?;
        let name = self.agent_notice_name(agent);
        self.owned.record_notice_for_agent(
            room,
            None,
            Some(agent),
            self.owned.attachment_store.list_session_attachment_ids(room),
            format!(
                "A message from {} is waiting for {name}, which you stopped. It is delivered after your next prompt to {name}.",
                self.agent_notice_name(&event.source_id)
            ),
        );
        Ok(())
    }
    fn defer_unrelated_sudo_event(
        &self,
        room: &str,
        agent: &str,
        work: &str,
    ) -> Result<(), DaemonError> {
        let Some(front) = self
            .owned
            .durable_state_store
            .agent_delivery_front(room, agent)?
        else {
            return Ok(());
        };
        let correlated = ledger::work_correlated(&front, work);
        if correlated || front.state != "pending" || front.attempted_at_ms.is_some() {
            return Ok(());
        }
        self.owned
            .durable_state_store
            .agent_lifecycle(Operation::Defer {
                room: room.into(),
                agent: agent.into(),
                sequence: front.sequence,
                now: crate::session::unix_epoch_ms(),
            })?;
        self.owned.record_notice_for_agent(
            room,
            None,
            Some(agent),
            self.owned.attachment_store.list_session_attachment_ids(room),
            format!(
                "Deferred inbox event {} from {}: agent {agent} is running sudo-bound work for its owner. It is delivered as a regular turn once that work ends, its window expires or it is revoked.",
                front.sequence, front.source_id
            ),
        );
        Ok(())
    }
    /// A10: the worker's exact acceptance is the leased turn's provider
    /// dispatch, so it binds the task's run and consumes a retained wait the
    /// way local dispatch does immediately before provider I/O.
    pub(super) fn bind_leased_agent_task(
        &self,
        dispatch: &crate::app::KernelRemotePromptDispatch,
        worker_run: &str,
    ) {
        if !self.owned.config_projection.snapshot().room_agent_tools {
            return;
        }
        let result = (|| {
            self.owned
                .durable_state_store
                .agent_lifecycle(Operation::Begin {
                    owner: self
                        .owned
                        .session_store
                        .get_session(&dispatch.session_id)?
                        .owner_user_id()
                        .into(),
                    room: dispatch.session_id.clone(),
                    agent: dispatch.agent_id.clone(),
                    prompt: dispatch.prompt_id.clone(),
                    run: Some(crate::provider::projected_leased_provider_run_id(
                        &dispatch.leased_agent_id,
                        worker_run,
                    )),
                    now: crate::session::unix_epoch_ms(),
                })
        })();
        if let Err(error) = result {
            crate::logging::warn_with_fields(
                "daemon.agent_lifecycle",
                "MP-08/MP-09/MP-10/MP-11 A10: accepted leased turn left its task for sweep",
                serde_json::json!({"error": crate::secret_redaction::redact_secrets(&error.to_string())}),
            );
        }
    }
    /// A10: the leased dispatch's exact worker outcome is this attempt's
    /// receipt. Acceptance binds the projected worker run; a proven rejection
    /// returns the event to pending; a changed binding stays uncertain.
    pub(super) fn record_leased_event_receipt(
        &self,
        dispatch: &crate::app::KernelRemotePromptDispatch,
        worker_run: Option<&str>,
        state: &str,
    ) {
        let (room, agent) = (&dispatch.session_id, &dispatch.agent_id);
        let result = (|| {
            let Some(event) = self.owned.durable_state_store.agent_event_for_prompt(
                room,
                agent,
                &dispatch.prompt_id,
            )?
            else {
                return Ok(());
            };
            if !matches!(event.state.as_str(), "submitting" | "uncertain" | "blocked") {
                return Ok(());
            }
            if let Some(run) = worker_run.filter(|_| event.state == "submitting") {
                self.owned
                    .durable_state_store
                    .agent_lifecycle(Operation::BindSubmission {
                        room: room.clone(),
                        agent: agent.clone(),
                        sequence: event.sequence,
                        prompt: dispatch.prompt_id.clone(),
                        // Leased delivery never steers an active turn.
                        target: None,
                        run: crate::provider::projected_leased_provider_run_id(
                            &dispatch.leased_agent_id,
                            run,
                        ),
                        submit_epoch: 0,
                        now: crate::session::unix_epoch_ms(),
                    })?;
            }
            self.record_agent_delivery_receipt(room, agent, event.sequence, state)
        })();
        if let Err(error) = result {
            crate::logging::warn_with_fields(
                "daemon.agent_lifecycle",
                "MP-08/MP-09/MP-10/MP-11 A10: leased wake receipt retained for sweep reconciliation",
                serde_json::json!({"error": crate::secret_redaction::redact_secrets(&error.to_string())}),
            );
        }
    }
    pub(super) fn record_agent_delivery_receipt(
        &self,
        room: &str,
        agent: &str,
        sequence: u64,
        state: &str,
    ) -> Result<(), DaemonError> {
        let Outcome::Event(event) =
            self.owned
                .durable_state_store
                .agent_lifecycle(Operation::Receipt {
                    room: room.into(),
                    agent: agent.into(),
                    sequence,
                    state: state.into(),
                    now: crate::session::unix_epoch_ms(),
                })?
        else {
            unreachable!()
        };
        // A refused pre-I/O attempt remains retryable; only acceptance or an
        // uncertain receipt needs a client notice. Sweep surfaces bounded failures.
        if event.state == "pending" {
            self.owned.record_wake_refused(&event);
            return Ok(());
        }
        self.owned.record_notice_for_agent(
            room,
            None,
            Some(agent),
            self.owned
                .attachment_store
                .list_session_attachment_ids(room),
            if state == "failed" {
                ledger::opencode_steer_conflict_notice(event.sequence)
            } else {
                ledger::receipt_notice(&event)
            },
        );
        Ok(())
    }
    // Called by the shared dispatch path, including authorized substitute reruns.
    pub(super) fn record_agent_event_dispatch_result(
        &self,
        dispatch: &crate::app::KernelPromptDispatch,
        result: &Result<bool, DaemonError>,
    ) -> Result<(), DaemonError> {
        let Some(event) = self.owned.durable_state_store.agent_event_for_prompt(
            &dispatch.session_id,
            &dispatch.agent_id,
            &dispatch.prompt_id,
        )?
        else {
            return Ok(());
        };
        if event.state != "submitting" {
            return Ok(());
        }
        // The send boundary may have replaced dispatch's initial run. The
        // bound run, rather than that initial profile, defines receipt semantics.
        let structured = event
            .provider_run_id
            .as_deref()
            .map(|id| {
                self.owned.provider_store.get_run(id).map(|run| {
                    self.owned
                        .provider_store
                        .run_uses_structured_prompt_io(&run)
                })
            })
            .transpose()?
            .unwrap_or(false);
        let state = ledger::delivery_attempt_receipt_state(
            event.submit_epoch.is_some(),
            result,
            structured,
        );
        if state != "submitting" {
            self.record_agent_delivery_receipt(
                &dispatch.session_id,
                &dispatch.agent_id,
                event.sequence,
                state,
            )?;
        }
        Ok(())
    }
}

impl KernelRuntimeOwnedState {
    /// The queued notification owns this delivery opportunity. Ordinary rows
    /// still pending beside it join when the prompt is promoted, and the
    /// durable prompt commit reserves their receipts before provider I/O.
    pub(super) fn add_pending_wakes_to_notification(
        &self,
        session: &crate::session::RuntimeSession,
        agent: &str,
        started: crate::session::PromptQueueItem,
    ) -> Result<crate::session::PromptQueueItem, DaemonError> {
        let notifications = started.queued_notifications();
        if !notifications
            .first()
            .is_some_and(|notification| notification.batch_instruction.is_some())
            || self.sudo_work_task(session.id(), agent).is_some()
        {
            return Ok(started);
        }

        let room = session.id();
        let store = &self.durable_state_store;
        let tasks = store.agent_tasks(Some(room), Some(agent))?;
        let can_resume_task = tasks
            .iter()
            .find(|task| task.prompt_id == started.id())
            .is_none_or(ledger::AgentTaskExecution::is_unstarted_prompt_task);
        let mut protected = Vec::new();
        for task in &tasks {
            if task.pending_prompt_id.is_some()
                || (task.state == ExecutionState::Waiting
                    && store.agent_task_workflow(&task.task_id)?.is_some())
            {
                protected.push(task.task_id.as_str());
            }
        }
        let eligible = |event: &ledger::InboxEvent| {
            event.state == "pending"
                && event.kind != "message"
                && !event.reply_requested
                && matches!(wake_disposition(event, &tasks), WakeDisposition::Deliver)
                && !protected.iter().any(|task| names_task(event, task))
                && (can_resume_task
                    || !tasks.iter().any(|task| {
                        task.state == ExecutionState::Waiting && names_task(event, &task.task_id)
                    }))
        };
        let Some(lead) = store.agent_delivery_front(room, agent)?.filter(eligible) else {
            return Ok(started);
        };
        let following = store.agent_delivery_batch(&lead, false, None)?;
        let batch = std::iter::once(lead)
            .chain(
                following
                    .into_iter()
                    .take_while(eligible)
                    .take(MAX_WAKE_BATCH),
            )
            .collect::<Vec<_>>();

        let mut combined = started.clone();
        combined.set_prompt(super::agent_inbox_wake_text::notification_batch(
            notifications,
            &batch,
        ));
        let combined =
            combined.with_inbox_wakes(batch.iter().map(|event| event.sequence).collect());
        if self.prompt_state_owner.replace_active_prompt_if_matches(
            session,
            agent,
            &started,
            combined.clone(),
        ) {
            Ok(combined)
        } else {
            Ok(started)
        }
    }

    /// Bind only after dispatch admission and immediately before provider I/O.
    pub(super) fn bind_agent_event_submission(
        &self,
        dispatch: &crate::app::KernelPromptDispatch,
    ) -> Result<(), DaemonError> {
        let Some(event) = self.durable_state_store.agent_event_for_prompt(
            &dispatch.session_id,
            &dispatch.agent_id,
            &dispatch.prompt_id,
        )?
        else {
            return Ok(());
        };
        self.durable_state_store
            .agent_lifecycle(Operation::BindSubmission {
                room: dispatch.session_id.clone(),
                agent: dispatch.agent_id.clone(),
                sequence: event.sequence,
                prompt: dispatch.prompt_id.clone(),
                target: dispatch.target_active_prompt_id.clone(),
                run: dispatch.provider_run_id.clone(),
                submit_epoch: self.provider_store.structured_submit_epoch(),
                now: crate::session::unix_epoch_ms(),
            })?;
        Ok(())
    }
}
#[cfg(test)]
impl KernelRuntimeState {
    /// The leased turn's public answer as its projection records it at home:
    /// under the worker's own run id (the dev-stub provider emits none).
    pub(crate) fn record_leased_answer_for_test(
        &self,
        room: &str,
        agent: &str,
        text: &str,
    ) -> Result<(), DaemonError> {
        let session = self.owned.session_store.get_session(room)?;
        let prompt = self
            .owned
            .prompt_state_owner
            .active_prompt_for_agent(&session, agent)
            .ok_or_else(|| ledger::error("no active leased prompt"))?;
        let remote = self
            .owned
            .agent_store
            .get_agent(agent)?
            .remote_execution()
            .cloned()
            .ok_or_else(|| ledger::error("agent is not leased"))?;
        let run = remote
            .active_worker_provider_run_id
            .clone()
            .ok_or_else(|| ledger::error("leased run not bound"))?;
        self.owned.append_operational_history_entry_with_context(
            &crate::history::SessionHistoryEntry::provider_output(
                room,
                &run,
                Some(agent),
                crate::terminal::TerminalOutputKind::ProviderOutput,
                None,
                text,
            ),
            crate::history::HistoryEventTurnContext {
                session_id: Some(room.into()),
                agent_id: Some(agent.into()),
                provider_run_id: Some(run.clone()),
                prompt_id: Some(prompt.id().into()),
                turn_id: Some(prompt.id().into()),
                ..Default::default()
            },
        );
        Ok(())
    }

    pub(crate) async fn deliver_agent_inbox_for_test(
        &self,
        room: &str,
        agent: &str,
    ) -> Result<(), DaemonError> {
        self.deliver_agent_inbox(room, agent).await
    }

    pub(crate) fn agent_notices_for_test(&self, room: &str, agent: &str) -> Vec<String> {
        self.notices_for_test(room, Some(agent))
    }

    pub(crate) fn session_notices_for_test(&self, room: &str) -> Vec<String> {
        self.notices_for_test(room, None)
    }

    fn notices_for_test(&self, room: &str, agent: Option<&str>) -> Vec<String> {
        self.owned
            .operational_history_store
            .load_session_history_entries(room, agent)
            .unwrap()
            .into_iter()
            .filter(|entry| entry.kind == crate::history::SessionHistoryEntryKind::Notice)
            .map(|entry| entry.text)
            .collect()
    }
}
