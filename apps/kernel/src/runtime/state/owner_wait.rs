//! A tool that needs a person's answer on a card never holds the provider's tool call past the
//! provider's own timeout. The call waits at most `IN_CALL` (60 s); then it returns `waiting_for_owner`
//! with the card still open, and the answer reaches the asking agent as an event of an
//! `owner_wait` wake, like any other source the agent can keep working beside or yield on.
//! A card the kernel withdraws on its own timeout is `timed_out`, never a denial.
use super::wake_driver::{Cancel, Liveness, Recovery, WakeDriver, WakeOccurrence};
use super::*;
use crate::durable_state::agent_lifecycle::{AgentWake, WakeEnd};

pub(super) const KIND: &str = "owner_wait";
/// The one window a tool call waits for the owner, the same as enrollment's: past it the call
/// returns and the answer arrives as an event.
pub(super) const IN_CALL: Duration = super::kernel_enrollment::ANSWER_WINDOW;
/// The error code a tool turns into its `waiting_for_owner` result.
pub(super) const WAITING_FOR_OWNER: &str = "waiting_for_owner";

/// How the person's card ended.
pub(super) enum OwnerAnswer {
    Chose(String),
    /// The card closed without a decision: `why` says how, `expired` that nobody answered in time.
    Unanswered {
        why: String,
        expired: bool,
    },
    /// The kernel stopped under the card (a restart or a hot reload): nothing was decided.
    Interrupted,
}

/// What the answer did, as the asking agent reads it.
pub(super) struct OwnerOutcome {
    pub(super) status: &'static str,
    pub(super) message: String,
}

pub(super) struct OwnerWait {
    pub(super) room: String,
    /// The asking agent; without a running turn the answer is read by asking again.
    pub(super) agent: Option<String>,
    pub(super) card_id: String,
    /// Names the wait in the agent's wake list, for example "Machine acquisition".
    pub(super) label: String,
    pub(super) lifetime: Duration,
    /// `None` waits for the whole card: for callers no provider tool call bounds.
    pub(super) in_call: Option<Duration>,
}

pub(super) enum Waited {
    Ended(OwnerOutcome),
    /// The card is still open; the message tells the agent where the answer arrives.
    Waiting(String),
}

impl KernelRuntimeState {
    /// Waits for `receiver` (the card `wait.card_id`) and applies the answer with `settle`, in this
    /// call or, once `wait.in_call` has passed, in the background.
    pub(super) async fn wait_for_owner(
        &self,
        wait: OwnerWait,
        mut receiver: tokio::sync::oneshot::Receiver<PendingInteractionResolution>,
        settle: impl FnOnce(&KernelRuntimeState, OwnerAnswer) -> OwnerOutcome + Send + 'static,
    ) -> Result<Waited, DaemonError> {
        let asked = std::time::Instant::now();
        let deadline = tokio::time::Instant::now() + wait.lifetime;
        // A call the provider drops while it still waits leaves no card nobody will act on.
        let card = super::runtime_vault_unlock_state::CloseInteractionOnDrop {
            state: self.clone(),
            session_id: wait.room.clone(),
            interaction_id: wait.card_id.clone(),
        };
        let in_call = wait.in_call.unwrap_or(wait.lifetime).min(wait.lifetime);
        if let Ok(resolution) = tokio::time::timeout(in_call, &mut receiver).await {
            return Ok(Waited::Ended(settle(self, answer(resolution, asked))));
        }
        if in_call >= wait.lifetime {
            self.timeout_runtime_interaction(&wait.room, &wait.card_id)
                .await?;
            return Ok(Waited::Ended(settle(self, expired(asked))));
        }
        let registration = match &wait.agent {
            Some(agent) => self.arm_owner_wait(&wait, agent)?,
            None => None,
        };
        let message = waiting_message(registration.as_ref().map(|(_, id)| id.as_str()));
        let state = self.clone();
        let (room, card_id) = (wait.room.clone(), wait.card_id.clone());
        tokio::spawn(async move {
            let _card = card;
            let answer = match tokio::time::timeout_at(deadline, receiver).await {
                Ok(resolution) => answer(resolution, asked),
                Err(_) => {
                    let _ = state.timeout_runtime_interaction(&room, &card_id).await;
                    expired(asked)
                }
            };
            // A card that ended while the kernel was stopping was not decided: it is the stop.
            let answer = match answer {
                OwnerAnswer::Unanswered { .. }
                    if state
                        .owned
                        .kernel_stopping
                        .load(std::sync::atomic::Ordering::SeqCst) =>
                {
                    OwnerAnswer::Interrupted
                }
                other => other,
            };
            // The kernel is going away: the next kernel finds this wake still running, ends it as
            // lost ("withdrawn by the restart") and tells the agent; this one says nothing.
            if matches!(answer, OwnerAnswer::Interrupted) {
                return;
            }
            let outcome = settle(&state, answer);
            if let Some(wake) = registration.as_ref().map(|(wake, _)| wake) {
                state.report_owner_wait(wake, &card_id, outcome);
            }
        });
        Ok(Waited::Waiting(message))
    }

    /// The waiting message for a card already open with `card_id`, so asking again raises no second one.
    pub(super) fn open_owner_wait(&self, room: &str, card_id: &str) -> Option<String> {
        if !self
            .owned
            .pending_interactions
            .write()
            .contains_key(card_id)
        {
            return None;
        }
        let registration = self
            .owned
            .durable_state_store
            .agent_wakes(Some(room), None)
            .ok()?
            .into_iter()
            .find(|wake| {
                wake.kind == KIND
                    && matches!(wake.state.as_str(), "starting" | "running")
                    && wake.detail["interaction_id"] == card_id
            })
            .map(|wake| wake.registration_id);
        Some(waiting_message(registration.as_deref()))
    }

    /// The wake the answer arrives on, when the agent has a running turn to own it.
    fn arm_owner_wait(
        &self,
        wait: &OwnerWait,
        agent: &str,
    ) -> Result<Option<(String, String)>, DaemonError> {
        let Ok(task) = self.asking_task_for_turn(&wait.room, agent) else {
            return Ok(None);
        };
        let mut wake = super::tool_dispatch::agent_wakes::new_wake(
            &task,
            KIND,
            format!("{}: waiting for the owner", wait.label),
            crate::session::unix_epoch_ms(),
        );
        wake.state = "starting".into();
        wake.detail = serde_json::json!({ "interaction_id": wait.card_id });
        self.arm_wake_source(&task, &wake)?;
        self.wake_sink(KIND).verified(&wake.id)?;
        Ok(Some((wake.id, wake.registration_id)))
    }

    fn report_owner_wait(&self, wake: &str, card_id: &str, outcome: OwnerOutcome) {
        let sink = self.wake_sink(KIND);
        if !sink.awaits_occurrence(wake) {
            return;
        }
        let reported = sink.occurred(WakeOccurrence {
            wake_id: wake.into(),
            occurrence_id: format!("{wake}-answered"),
            kind: "owner_answer".into(),
            payload: serde_json::json!({
                "interaction_id": card_id,
                "status": outcome.status,
                "message": outcome.message,
            }),
            summary: Some(outcome.message),
            end: Some(WakeEnd::Succeeded),
        });
        if let Err(error) = reported {
            tracing::warn!(wake_id = wake, %error, "owner answer could not reach the agent");
        }
    }
}

impl KernelRuntimeState {
    /// The card of an `owner_wait` that a restart ended answers nothing any more: it leaves every
    /// client instead of staying as a card whose buttons do nothing.
    pub(super) fn withdraw_owner_wait_card(&self, wake: &AgentWake) {
        if wake.kind != KIND {
            return;
        }
        if let Some(card) = wake.detail["interaction_id"].as_str() {
            if let Err(error) = self.owned.timeout_runtime_interaction(&wake.room_id, card) {
                tracing::warn!(wake_id = %wake.id, %error, "restarted owner card could not be withdrawn");
            }
        }
    }
}

fn answer(
    resolution: Result<PendingInteractionResolution, tokio::sync::oneshot::error::RecvError>,
    asked: std::time::Instant,
) -> OwnerAnswer {
    // Only an answer is the owner's decision; every other end says what closed the card.
    match resolution {
        Ok(resolution) if resolution.status == "answered" => {
            OwnerAnswer::Chose(resolution.choice_id.unwrap_or_default())
        }
        Ok(resolution) => OwnerAnswer::Unanswered {
            why: match resolution.reply {
                Some(reason) => format!("it could not be answered: {reason}"),
                None => format!("nobody answered it within {} s", asked.elapsed().as_secs()),
            },
            expired: false,
        },
        Err(_) => OwnerAnswer::Unanswered {
            why: format!(
                "the kernel withdrew it after {} s, before anyone answered",
                asked.elapsed().as_secs()
            ),
            expired: false,
        },
    }
}

fn expired(asked: std::time::Instant) -> OwnerAnswer {
    OwnerAnswer::Unanswered {
        why: format!("nobody answered it within {} s", asked.elapsed().as_secs()),
        expired: true,
    }
}

fn waiting_message(registration: Option<&str>) -> String {
    match registration {
        Some(registration) => format!(
            "Waiting for the owner to answer the card in the Room. Keep working: the answer arrives in your inbox (registration {registration}); yield on it when you need it."
        ),
        None => "Waiting for the owner to answer the card in the Room. Ask again with the same request to read the answer.".into(),
    }
}

/// The background task that reports the answer ends with the kernel: after a restart the
/// card is gone, so the wait ends and the agent asks again.
pub(super) struct OwnerWaitDriver;

impl WakeDriver for OwnerWaitDriver {
    fn kind(&self) -> &'static str {
        KIND
    }
    fn arm(&self, _wake: &AgentWake) -> Result<(), DaemonError> {
        Ok(())
    }
    fn cancel(&self, _wake: &AgentWake) -> Result<Cancel, DaemonError> {
        Ok(Cancel::Done)
    }
    fn recover(&self, _wake: &AgentWake) -> Recovery {
        Recovery::Lost(
            "the owner's card was withdrawn by the restart and nothing was decided; call the tool again with the same request_id to raise it again".into(),
        )
    }
    fn liveness(&self, _wake: &AgentWake) -> Liveness {
        Liveness::Live
    }
}
