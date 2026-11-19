//! Claude runs turns no prompt asked for: when a `run_in_background` command ends, it starts a
//! turn of its own to report it. The kernel adopts that turn as the agent's running turn with a
//! task, so the turn's tools act for it and its end settles that task like any other turn's.
//!
//! One record per agent (prompt_state `ProviderOwnTurn`) follows the turn; each transition is one
//! step under the prompt state owner's lock and carries the turn's run and binding. Driven by each
//! batch's `ProviderOwnTurnSignal` and by prompt settlement:
//! - T1 no prompt active or queued -> Adopted at once, under the binding chosen from the batch
//!   that first reports the turn (its ended background calls, read before its history is applied);
//! - T2 the prompt before it is active but its provider turn already ended (its result was read)
//!   -> Pending, that prompt settles first, then the settlement adopts the turn with the binding
//!   it was recorded with, so the turn's output is its own;
//! - T3/T4 a prompt still queued or being delivered -> Pending with its binding; adopted with
//!   that binding when the agent has no prompt left, by the settlement or a later batch;
//! - T5 a prompt is written into a pending held turn -> Folded inside the held segment; its
//!   steering boundary starts its answer without ending the notification turn or its output;
//! - T6 the turn ends while Pending -> None, logged; the prompt it waited for holds its output,
//!   unless the turn is bound to a held task (below);
//! - T7/T8 an Adopted turn ends, is cancelled or its process exits -> the ordinary settlement
//!   completes or fails the adopted prompt;
//! - T9 a turn that starts and ends within one batch is adopted, then completed by that batch.
//!
//! A child held for its background calls has a task already: its notification turn runs under
//! the prompt of the held task whose call ended (binding Held), so the one task carries the
//! turn's tools and its finish. The turn gets a resumed boundary in history, so the task's answer
//! reads every segment of its prompt and no reply to a prompt in between. A pending held turn
//! gets that boundary at its first batch (one step on the record: `open_pending_held_segment`),
//! so its output is the held task's even before adoption. A held task whose turn never runs as
//! its own (T5 folded, T6, T9 still pending) finishes at its result, and only after the batch
//! that dropped it is in history: `follow_provider_own_turn` hands that release to its caller.
//! Any other adopted turn (binding Fresh) gets its own task and its own history boundary, which
//! its answer is read from.
use super::*;
use crate::provider::ProviderOwnTurnSignal;
use crate::runtime::prompt_state::{
    OwnTurnBinding, PendingOwnTurn, PendingTurnOutput, ProviderTurnAdoption,
};

/// What history shows where a turn the provider started on its own begins.
const PROVIDER_OWN_TURN_TEXT: &str = "Background command finished";

/// A held task whose notification turn left the record before it was adopted. It finishes once
/// the batch that ended the turn is in history, so its answer holds the turn's output.
#[derive(Debug)]
pub(super) struct HeldRelease {
    agent_id: String,
    provider_run_id: String,
    prompt_id: String,
    folded_prompt: Option<Box<crate::session::PromptQueueItem>>,
}

impl KernelRuntimeState {
    /// Follows one batch's report of the provider's own turn, before the batch's output is
    /// recorded, so that output belongs to the turn it reports. A held task the turn's end
    /// releases is returned: the caller releases it after the batch is flushed to history.
    pub(super) async fn follow_provider_own_turn(
        &self,
        session_id: &str,
        provider_run_id: &str,
        batch: &crate::provider::ProviderPromptSignalBatch,
    ) -> Result<Option<HeldRelease>, DaemonError> {
        let owned = &self.owned;
        let run = owned.ensure_provider_run_in_session(session_id, provider_run_id)?;
        let Some(agent_id) = run.agent_instance_id() else {
            return Ok(None);
        };
        let session = owned.session_store.get_session(session_id)?;
        // Consume the recorded delivery edge before this batch's output. A prompt delivered
        // before the own turn began is not a fold into that turn.
        self.fold_delivered_prompt_into_pending_held_turn(&session, agent_id, provider_run_id)?;
        let signal = batch.provider_own_turn;
        match signal {
            ProviderOwnTurnSignal::None if batch.prompt_completed => {
                // Cancellation/process exit ends the turn without Claude's own result.
                return self.end_provider_own_turn(session_id, agent_id, provider_run_id);
            }
            ProviderOwnTurnSignal::None => return Ok(None),
            ProviderOwnTurnSignal::Ended => {
                return self.end_provider_own_turn(session_id, agent_id, provider_run_id);
            }
            ProviderOwnTurnSignal::Running | ProviderOwnTurnSignal::StartedAndEnded => {}
        }
        let ended_now = crate::app::ended_background_call_ids(&batch.chunks);
        self.adopt_provider_started_turn(session_id, provider_run_id, &ended_now)?;
        let preceding_turn_ended = owned
            .prompt_state_owner
            .active_prompt_for_agent(&session, agent_id)
            .is_some_and(|active| !active.delivery_pending())
            && (owned.prompt_completion_recorded(provider_run_id)
                || owned.prompt_completion_settlement_pending(provider_run_id));
        if preceding_turn_ended
            && owned
                .prompt_state_owner
                .provider_turn_pending(&session, agent_id, provider_run_id)
        {
            // T2: Claude starts its own turn only after the turn before it ended: that prompt's
            // output is complete, so it settles now and the new turn's output is the new turn's.
            // The settlement adopts the pending turn with its recorded binding.
            Box::pin(self.settle_owned_provider_prompt(
                session_id,
                provider_run_id,
                false,
                false,
                true,
            ))
            .await?;
        }
        if let Some(held_prompt) =
            owned
                .prompt_state_owner
                .open_pending_held_segment(&session, agent_id, provider_run_id)
        {
            // A held task's notification turn waits behind another prompt: its output is the
            // held task's, not that prompt's, from here on.
            let agent = owned.agent_store.get_agent(agent_id)?;
            owned.append_resumed_prompt_history(
                session_id,
                &self.ensure_agent_message_attachment(session_id, &agent)?,
                agent_id,
                PROVIDER_OWN_TURN_TEXT,
                &held_prompt,
                crate::session::unix_epoch_ms(),
            )?;
        }
        // A receipt can precede opening the held segment, which must come before the fold.
        self.fold_delivered_prompt_into_pending_held_turn(&session, agent_id, provider_run_id)?;
        if signal == ProviderOwnTurnSignal::StartedAndEnded
            && owned
                .prompt_state_owner
                .provider_turn_pending(&session, agent_id, provider_run_id)
        {
            // T9 with a prompt still ahead of it: the turn is over before it could be adopted.
            return self.end_provider_own_turn(session_id, agent_id, provider_run_id);
        }
        Ok(None)
    }

    /// The provider result ends a pending or folded notification turn. Take its binding once;
    /// the caller finishes its held task only after this batch is in history.
    fn end_provider_own_turn(
        &self,
        session_id: &str,
        agent_id: &str,
        provider_run_id: &str,
    ) -> Result<Option<HeldRelease>, DaemonError> {
        let owned = &self.owned;
        let session = owned.session_store.get_session(session_id)?;
        let Some(PendingOwnTurn {
            provider_run_id: pending_run,
            binding,
            output,
        }) = owned
            .prompt_state_owner
            .end_provider_turn(&session, agent_id, provider_run_id)
        else {
            return Ok(None);
        };
        crate::logging::info_with_fields(
            "runtime.agent_lifecycle",
            "the provider's own turn ended before it was adopted; its held task finishes after history is flushed",
            serde_json::json!({
                "session_id": session_id,
                "agent_id": agent_id,
                "provider_run_id": pending_run,
                "held_prompt_id": match &binding {
                    OwnTurnBinding::Held { prompt_id } => Some(prompt_id),
                    OwnTurnBinding::Fresh => None,
                },
            }),
        );
        Ok(match binding {
            OwnTurnBinding::Held { prompt_id } => Some(HeldRelease {
                agent_id: agent_id.to_string(),
                provider_run_id: pending_run,
                prompt_id,
                folded_prompt: match output {
                    PendingTurnOutput::Folded { into, .. } => Some(into),
                    PendingTurnOutput::BehindPrompt | PendingTurnOutput::HeldSegment => None,
                },
            }),
            OwnTurnBinding::Fresh => None,
        })
    }

    /// Finishes the held task of a notification turn that ended before adoption, now that the
    /// batch that ended it is in history. A task no longer held (settled meanwhile) is left as
    /// it is.
    pub(super) async fn release_held_after_own_turn(
        &self,
        session_id: &str,
        release: HeldRelease,
    ) -> Result<(), DaemonError> {
        if let Some(prompt) = release.folded_prompt {
            // The steering boundary kept the notification inside the held answer. Its ending
            // batch is now in history; a resumed boundary gives later output to this prompt alone.
            self.owned.append_resumed_prompt_history(
                session_id,
                prompt.source_attachment_id(),
                &release.agent_id,
                prompt.prompt(),
                prompt.id(),
                crate::session::unix_epoch_ms(),
            )?;
        }
        for task in self.owned.held_tasks_after_notification(
            session_id,
            &release.agent_id,
            &release.provider_run_id,
        )? {
            if task.prompt_id == release.prompt_id {
                Box::pin(self.release_held_task(&task)).await?;
            }
        }
        Ok(())
    }

    /// A settlement may have left the agent with no prompt while its provider runs a turn of
    /// its own that waited for it: that turn becomes the agent's running turn now, under the
    /// binding it was recorded with.
    pub(super) fn adopt_pending_provider_turn(
        &self,
        session_id: &str,
        provider_run_id: &str,
    ) -> Result<(), DaemonError> {
        let owned = &self.owned;
        let run = owned.ensure_provider_run_in_session(session_id, provider_run_id)?;
        let Some(agent_id) = run.agent_instance_id() else {
            return Ok(());
        };
        let session = owned.session_store.get_session(session_id)?;
        if !owned
            .prompt_state_owner
            .provider_turn_pending(&session, agent_id, provider_run_id)
        {
            return Ok(());
        }
        self.adopt_provider_started_turn(session_id, provider_run_id, &BTreeSet::new())
    }

    fn adopt_provider_started_turn(
        &self,
        session_id: &str,
        provider_run_id: &str,
        ended_now: &BTreeSet<String>,
    ) -> Result<(), DaemonError> {
        let owned = &self.owned;
        let run = owned.ensure_provider_run_in_session(session_id, provider_run_id)?;
        let Some(agent_id) = run.agent_instance_id() else {
            return Ok(());
        };
        let session = owned.session_store.get_session(session_id)?;
        if owned
            .prompt_state_owner
            .provider_turn_running(&session, agent_id)
        {
            return Ok(());
        }
        let agent = owned.agent_store.get_agent(agent_id)?;
        // Used only when the turn is reported for the first time; a pending turn keeps its own.
        let binding = match owned.held_prompt_for_notification_turn(
            session_id,
            agent_id,
            provider_run_id,
            ended_now,
        )? {
            Some(prompt_id) => OwnTurnBinding::Held { prompt_id },
            None => OwnTurnBinding::Fresh,
        };
        let fresh_prompt = crate::session::PromptQueueItem::new(
            owned.session_store.reserve_prompt_id(),
            self.ensure_agent_message_attachment(session_id, &agent)?,
            agent_id,
            "",
            crate::session::PromptStatus::Running,
        );
        let (prompt, binding, segment_open) = match owned.prompt_state_owner.adopt_provider_turn(
            &session,
            fresh_prompt,
            provider_run_id,
            binding,
        ) {
            ProviderTurnAdoption::Pending { newly } => {
                if newly {
                    crate::logging::info_with_fields(
                        "runtime.agent_lifecycle",
                        "the provider started a turn on its own while an earlier prompt is still settling; it is adopted when that prompt leaves",
                        serde_json::json!({
                            "session_id": session_id,
                            "agent_id": agent_id,
                            "provider_run_id": provider_run_id,
                        }),
                    );
                }
                return Ok(());
            }
            ProviderTurnAdoption::Adopted {
                prompt,
                binding,
                segment_open,
            } => (*prompt, binding, segment_open),
        };
        let (active, queued) = owned.prompt_state_owner.state_parts(&session, agent_id);
        owned.mirror_prompt_owner_agent_state(session_id, agent_id, active, queued)?;
        owned.note_prompt_started(provider_run_id);
        let started_at_ms = crate::session::unix_epoch_ms();
        match &binding {
            // Its segment opened while the turn was pending.
            OwnTurnBinding::Held { .. } if segment_open => {}
            // A held child's task exists and stays Working; the turn resumes its prompt, and the
            // task's answer reads this segment too.
            OwnTurnBinding::Held { .. } => owned.append_resumed_prompt_history(
                session_id,
                prompt.source_attachment_id(),
                agent_id,
                PROVIDER_OWN_TURN_TEXT,
                prompt.id(),
                started_at_ms,
            )?,
            // Any other turn begins its own task, and its answer is the output after its own
            // boundary.
            OwnTurnBinding::Fresh => {
                owned.append_user_prompt_history(
                    session_id,
                    prompt.source_attachment_id(),
                    agent_id,
                    PROVIDER_OWN_TURN_TEXT,
                    &[],
                    crate::session::PromptOrigin::Notification,
                    Some(prompt.id()),
                    None,
                    None,
                    Some(started_at_ms),
                    None,
                )?;
                owned.note_started_prompt_sent_at(session_id, agent_id, started_at_ms)?;
                if owned.config_projection.snapshot().room_agent_tools {
                    owned.ensure_agent_task(
                        session_id,
                        agent_id,
                        &prompt,
                        Some(provider_run_id),
                    )?;
                }
            }
        }
        crate::logging::info_with_fields(
            "runtime.agent_lifecycle",
            "adopted a turn the provider started on its own",
            serde_json::json!({
                "session_id": session_id,
                "agent_id": agent_id,
                "prompt_id": prompt.id(),
                "provider_run_id": provider_run_id,
                "held_task": matches!(binding, OwnTurnBinding::Held { .. }),
            }),
        );
        Ok(())
    }
}

impl KernelRuntimeState {
    fn fold_delivered_prompt_into_pending_held_turn(
        &self,
        session: &crate::session::RuntimeSession,
        agent_id: &str,
        provider_run_id: &str,
    ) -> Result<(), DaemonError> {
        let owned = &self.owned;
        if let Some((held, prompt)) =
            owned
                .prompt_state_owner
                .take_folded_held_boundary(session, agent_id, provider_run_id)
        {
            owned.append_steering_prompt_history(
                session.id(),
                provider_run_id,
                &held,
                prompt.source_attachment_id(),
                agent_id,
                prompt.id(),
                prompt.prompt(),
                prompt.attachments(),
                prompt.prompt_origin(),
            )?;
        }
        Ok(())
    }
}
