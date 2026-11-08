//! Peers across kernels: an agent here follows an ordinary agent of a room on another kernel of
//! its owner. The followed kernel keeps the follower and owes it one outcome per follow (turn
//! ended, turn failed or source lost) in a durable outbox; the following kernel turns a
//! received outcome into the same `peer_event` a follow of a peer on this kernel gives.
use super::*;
use crate::durable_state::agent_lifecycle::{
    self as ledger, peer_follows::PeerOutboxRow, Operation, Outcome, Registration,
};
use crate::transport::relay_peer::{
    PeerAgentAddress, PeerAgentOutcome, PeerAgentStamp, PeerOutcomeAck, RelayPeerRequest,
    RelayPeerResponse, RELAY_PEER_PROTOCOL_VERSION as VERSION,
};

/// The first retry of an outcome the other kernel did not take, doubled up to the ceiling.
const RETRY_FIRST_MS: u64 = 5_000;
const RETRY_CEILING_MS: u64 = 10 * 60 * 1_000;

fn ack(ack: PeerOutcomeAck) -> RelayPeerResponse {
    RelayPeerResponse::PeerAgentOutcomeAck {
        protocol_version: VERSION,
        ack,
    }
}

/// The source a follower here records for an agent on another kernel: where it runs, fully.
fn peer_source(stamp: &PeerAgentStamp) -> String {
    format!("{}/{}/{}", stamp.kernel_id, stamp.room_id, stamp.agent_id)
}

/// The stamp back out of a follower's source, for its inbox view.
pub(crate) fn peer_source_stamp(source: &str) -> Option<PeerAgentStamp> {
    let mut parts = source.splitn(3, '/');
    let (kernel, room, agent) = (parts.next()?, parts.next()?, parts.next()?);
    (!kernel.is_empty() && !room.is_empty() && !agent.is_empty()).then(|| PeerAgentStamp {
        kernel_id: kernel.into(),
        room_id: room.into(),
        agent_id: agent.into(),
    })
}

fn valid_id(id: &str) -> bool {
    !id.trim().is_empty() && id.len() <= 128
}

impl KernelRuntimeState {
    /// Followed side: `kernel` follows, or stops following, an agent of a room of `owner` here.
    pub(super) fn receive_peer_follow(
        &self,
        kernel: &str,
        owner: &str,
        request: RelayPeerRequest,
    ) -> Result<RelayPeerResponse, DaemonError> {
        match request {
            RelayPeerRequest::SubscribePeerAgent {
                protocol_version: VERSION,
                request_id: _,
                sender: _,
                target,
                subscription_id,
                generation,
            } => {
                if !valid_id(&subscription_id) {
                    return Err(ledger::error("invalid peer subscription id"));
                }
                let target = match self.peer_agent_target(owner, &target) {
                    Ok(target) => target,
                    Err(refusal) => {
                        return Ok(RelayPeerResponse::PeerAgentRefused {
                            protocol_version: VERSION,
                            refusal,
                        })
                    }
                };
                let Outcome::PeerFollowed(followed) = self
                    .owned
                    .durable_state_store
                    .agent_lifecycle(Operation::FollowPeer {
                        kernel: kernel.into(),
                        owner: owner.into(),
                        subscription: subscription_id.clone(),
                        room: target.session_id().into(),
                        agent: target.id().into(),
                        generation,
                    })?
                else {
                    unreachable!()
                };
                Ok(RelayPeerResponse::PeerAgentSubscribed {
                    protocol_version: VERSION,
                    subscription_id,
                    generation: followed.generation,
                    target: self.peer_agent_stamp(&target),
                })
            }
            // No stamp: the follow it ends must be the sending kernel's own.
            RelayPeerRequest::UnsubscribePeerAgent {
                protocol_version: VERSION,
                subscription_id,
                generation,
            } => {
                self.owned
                    .durable_state_store
                    .agent_lifecycle(Operation::UnfollowPeer {
                        kernel: kernel.into(),
                        subscription: subscription_id,
                        generation,
                    })?;
                Ok(ack(PeerOutcomeAck::Accepted))
            }
            _ => Err(ledger::error("peer agent protocol version mismatch")),
        }
    }

    /// Following side: an outcome a followed agent's kernel owed this kernel. The same
    /// delivery twice is one inbox row; a follow that is not here, or not live, takes nothing.
    pub(super) async fn receive_peer_agent_outcome(
        &self,
        kernel: &str,
        owner: &str,
        request: RelayPeerRequest,
    ) -> Result<RelayPeerResponse, DaemonError> {
        let RelayPeerRequest::DeliverPeerAgentOutcome {
            protocol_version: VERSION,
            delivery_id: _,
            outcome:
                PeerAgentOutcome::Event {
                    subscription_id,
                    source,
                    occurrence_id,
                    event,
                },
        } = request
        else {
            return Err(ledger::error(
                "this kernel takes only follow events from another kernel",
            ));
        };
        if source.kernel_id != kernel || !valid_id(&occurrence_id) {
            return Ok(ack(PeerOutcomeAck::UnknownSubscription));
        }
        let store = &self.owned.durable_state_store;
        let source_id = peer_source(&source);
        let Some(registration) = store.agent_registration(&subscription_id)? else {
            return Ok(ack(PeerOutcomeAck::UnknownSubscription));
        };
        let Some(task) = store.agent_task(&registration.task_id)? else {
            return Ok(ack(PeerOutcomeAck::UnknownSubscription));
        };
        let room_owned = self
            .owned
            .session_store
            .get_session(&task.room_id)
            .is_ok_and(|room| room.owner_user_id() == owner);
        // A follow whose answer is still on its way names only the kernel it follows.
        let followed_kernel = peer_source_stamp(&registration.source_id)
            .is_some_and(|followed| followed.kernel_id == kernel);
        if !registration.peer || !followed_kernel || !room_owned {
            return Ok(ack(PeerOutcomeAck::UnknownSubscription));
        }
        if store.agent_source_occurred(&task.room_id, &source_id, &occurrence_id)? {
            return Ok(ack(PeerOutcomeAck::Duplicate));
        }
        if !registration.live {
            return Ok(ack(PeerOutcomeAck::UnknownSubscription));
        }
        let lost = event == crate::transport::relay_peer_agents::PeerEventKind::SourceLost;
        if lost != occurrence_id.starts_with(ledger::peer_follows::SOURCE_LOST_OCCURRENCE) {
            return Err(ledger::error("peer outcome names another occurrence kind"));
        }
        store.agent_lifecycle(Operation::SourceOutcome {
            public_answer: None,
            room: task.room_id.clone(),
            source: source_id,
            occurrence: occurrence_id,
            success: event == crate::transport::relay_peer_agents::PeerEventKind::TurnEnded,
            now: crate::session::unix_epoch_ms(),
        })?;
        if let Err(error) = Box::pin(self.deliver_agent_inbox(&task.room_id, &task.agent_id)).await
        {
            tracing::warn!(%error, agent_id = %task.agent_id,
                "peer event is durable and pending delivery");
        }
        Ok(ack(PeerOutcomeAck::Accepted))
    }

    /// Following side: the calling agent's task follows `agent` of `room` on `kernel`. The other
    /// kernel holds the follower first, so a turn that ends right after is not missed.
    pub(super) async fn subscribe_peer_on_other_kernel(
        &self,
        actor: &str,
        task_id: &str,
        prompt_id: &str,
        kernel: &str,
        room: &str,
        agent: &str,
    ) -> Result<serde_json::Value, DaemonError> {
        let sender = self.owned.agent_store.get_agent(actor)?;
        let subscription = format!("subscription-{:032x}", rand::random::<u128>());
        // Only occurrences after this follow starts reach it.
        let source_cursor = self.owned.durable_state_store.agent_source_cursor()?;
        // The follow is held here before the other kernel knows of it, so a turn that ends
        // before its answer arrives is recorded and reaches the follow once its source is known.
        let pending = PeerAgentStamp {
            kernel_id: kernel.into(),
            room_id: room.trim().into(),
            agent_id: subscription.clone(),
        };
        self.register_peer_follow(
            task_id,
            prompt_id,
            subscription.clone(),
            &pending,
            source_cursor,
        )?;
        let followed = self
            .follow_peer_agent(
                &sender,
                kernel,
                PeerAgentAddress {
                    room: room.trim().into(),
                    agent: agent.trim().trim_start_matches('@').into(),
                },
                subscription.clone(),
            )
            .await;
        let target = match followed {
            Ok(target) => target,
            Err(error) => {
                self.owned
                    .durable_state_store
                    .agent_lifecycle(Operation::Unsubscribe {
                        task: task_id.into(),
                        prompt: prompt_id.into(),
                        registration: subscription,
                    })?;
                return Err(error);
            }
        };
        let source = peer_source(&target);
        let registered = self.register_peer_follow(
            task_id,
            prompt_id,
            subscription.clone(),
            &target,
            source_cursor,
        );
        if let Err(error) = registered {
            if let Err(unfollow) = self
                .unfollow_peer_agent(&sender, kernel, &subscription)
                .await
            {
                tracing::warn!(%unfollow, %kernel, "peer follow left on the other kernel");
            }
            return Err(error);
        }
        Ok(serde_json::json!({
            "registration_id": subscription,
            "source_id": source,
            "kernel": target.kernel_id,
            "room": target.room_id,
            "agent": target.agent_id,
        }))
    }

    /// Following side: an ended follow of a peer on another kernel ends there too. The follow
    /// here is already over; one the other kernel keeps only costs it an undeliverable outcome.
    pub(super) async fn end_peer_follow_on_other_kernel(&self, actor: &str, registration: &str) {
        let Ok(Some(held)) = self
            .owned
            .durable_state_store
            .agent_registration(registration)
        else {
            return;
        };
        let (Some(target), true) = (peer_source_stamp(&held.source_id), held.peer) else {
            return;
        };
        let Ok(sender) = self.owned.agent_store.get_agent(actor) else {
            return;
        };
        if let Err(error) = self
            .unfollow_peer_agent(&sender, &target.kernel_id, registration)
            .await
        {
            tracing::warn!(%error, kernel = %target.kernel_id, "peer follow left on the other kernel");
        }
    }

    /// Following side: `task` follows `target` under the subscription the other kernel holds.
    pub(super) fn register_peer_follow(
        &self,
        task: &str,
        prompt: &str,
        subscription: String,
        target: &PeerAgentStamp,
        source_cursor: u64,
    ) -> Result<String, DaemonError> {
        self.owned
            .durable_state_store
            .agent_lifecycle(Operation::Subscribe {
                task: task.into(),
                prompt: prompt.into(),
                registration: Registration {
                    id: subscription.clone(),
                    task_id: task.into(),
                    source_id: peer_source(target),
                    obligation_id: None,
                    source_cursor,
                    live: true,
                    peer: true,
                },
            })?;
        Ok(subscription)
    }

    /// The caller's running turn's task, which a follow it asks for belongs to.
    pub(super) fn peer_follow_task(
        &self,
        caller: &crate::agent::AgentInstance,
    ) -> Result<(String, String), DaemonError> {
        let session = self.owned.session_store.get_session(caller.session_id())?;
        let prompt = session
            .active_prompt_for_agent(caller.id())
            .ok_or_else(|| ledger::error("no live creating turn"))?;
        self.owned
            .durable_state_store
            .agent_tasks(Some(caller.session_id()), Some(caller.id()))?
            .into_iter()
            .find(|task| task.prompt_id == prompt.id())
            .map(|task| (task.task_id, prompt.id().to_owned()))
            .ok_or_else(|| ledger::error("no live creating turn"))
    }

    /// Followed side: a peer spawned for another kernel is followed by its creator from the start.
    pub(crate) fn follow_spawned_room_peer(
        &self,
        kernel: &str,
        owner: &str,
        follow: crate::transport::relay_peer::PeerFollow,
        agent: &crate::agent::AgentInstance,
    ) -> Result<(), DaemonError> {
        if !valid_id(&follow.subscription_id) {
            return Err(ledger::error("invalid peer subscription id"));
        }
        self.owned
            .durable_state_store
            .agent_lifecycle(Operation::FollowPeer {
                kernel: kernel.into(),
                owner: owner.into(),
                subscription: follow.subscription_id,
                room: agent.session_id().into(),
                agent: agent.id().into(),
                generation: follow.generation,
            })
            .map(|_| ())
    }

    /// Followed side: sends every outcome that is due to the kernel that follows. A kernel that
    /// does not answer gets it again later, until the outcome expires.
    pub(crate) async fn deliver_peer_agent_outcomes(&self) -> Result<(), DaemonError> {
        let now = crate::session::unix_epoch_ms();
        let store = &self.owned.durable_state_store;
        let kernel_id = self.owned.config_projection.snapshot().daemon_id;
        for row in store.peer_agent_outbox_due(now)? {
            let delivery = row.delivery_id.clone();
            let answered = self
                .peer_agent_link(
                    &row.owner,
                    &row.subscriber_kernel_id.clone(),
                    outcome_request(&kernel_id, row.clone()),
                )
                .await;
            let operation = match answered {
                Ok(RelayPeerResponse::PeerAgentOutcomeAck { .. }) => {
                    Operation::PeerOutboxDelivered { delivery }
                }
                _ => Operation::PeerOutboxRetry {
                    delivery,
                    now,
                    at: now.saturating_add(
                        RETRY_FIRST_MS
                            .saturating_mul(1 << row.attempts.min(16))
                            .min(RETRY_CEILING_MS),
                    ),
                },
            };
            store.agent_lifecycle(operation)?;
        }
        Ok(())
    }

    /// Followed side: this kernel's relay connection is back, so the kernels it owes outcomes
    /// can be reached again.
    pub(crate) async fn peer_links_restored(&self) {
        if let Err(error) = self.deliver_peer_agent_outcomes().await {
            tracing::warn!(%error, "peer outcomes retained for the next delivery");
        }
    }

    /// Delivers due outcomes off the caller's path; a settlement never waits on another kernel.
    pub(super) fn spawn_peer_agent_outcome_delivery(&self) {
        let state = self.clone();
        tokio::spawn(async move {
            if let Err(error) = state.deliver_peer_agent_outcomes().await {
                tracing::warn!(%error, "peer outcomes retained for the next delivery");
            }
        });
    }

    /// Followed side: a followed agent or room that is gone tells its followers so.
    pub(super) fn settle_lost_peer_follows(&self, now: u64) -> Result<(), DaemonError> {
        let store = &self.owned.durable_state_store;
        for (room, agent) in store.peer_followed_agents()? {
            let present = self.owned.session_store.get_session(&room).is_ok()
                && self
                    .owned
                    .agent_store
                    .get_agent(&agent)
                    .is_ok_and(|found| found.session_id() == room);
            if !present {
                store.agent_lifecycle(Operation::PeerSourceLost { room, agent, now })?;
            }
        }
        Ok(())
    }
}

fn outcome_request(kernel_id: &str, row: PeerOutboxRow) -> RelayPeerRequest {
    RelayPeerRequest::DeliverPeerAgentOutcome {
        protocol_version: VERSION,
        delivery_id: row.delivery_id,
        outcome: PeerAgentOutcome::Event {
            subscription_id: row.subscription_id,
            source: PeerAgentStamp {
                kernel_id: kernel_id.into(),
                room_id: row.target_room,
                agent_id: row.target_agent,
            },
            occurrence_id: row.occurrence_id,
            event: row.event,
        },
    }
}

#[cfg(test)]
impl KernelRuntimeState {
    /// Ends `agent`'s running turn in `room` the way a provider settlement does.
    pub(crate) async fn end_agent_turn_for_tests(
        &self,
        room: &str,
        agent: &str,
        cancelled: bool,
    ) -> Result<(), DaemonError> {
        let session = self.owned.session_store.get_session(room)?;
        let prompt = session
            .active_prompt_for_agent(agent)
            .ok_or_else(|| ledger::error("no running turn"))?
            .clone();
        let run = self
            .owned
            .provider_store
            .list_runs()
            .into_iter()
            .rev()
            .find(|run| run.session_id() == room && run.agent_instance_id() == Some(agent))
            .ok_or_else(|| ledger::error("turn has no run"))?
            .id()
            .to_owned();
        let Outcome::Settled { task, correction } = self
            .owned
            .durable_state_store
            .agent_lifecycle(Operation::Settle {
                room: room.into(),
                agent: agent.into(),
                prompt: prompt.id().into(),
                run: run.clone(),
                has_answer: !cancelled,
                cancelled,
                now: crate::session::unix_epoch_ms(),
            })?
        else {
            return Err(ledger::error("turn did not settle"));
        };
        self.owned
            .complete_local_prompt_without_advance(room, agent, Some(&run))?;
        self.finish_agent_task_settlement(Some((task, correction)), &prompt)
            .await
    }
}
