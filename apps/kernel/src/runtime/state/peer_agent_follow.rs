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
/// How long a follow waits for the retry of a spawn whose answer was lost.
const UNKNOWN_PEER_FOLLOW_MS: u64 = 10 * 60 * 1_000;
/// How often Cloud's kernel directory is read while this kernel follows a peer on another one.
const KERNEL_DIRECTORY_READ_MS: u64 = 60 * 1_000;
/// How long one outcome waits for the following kernel's answer before it is retried later.
const OUTCOME_ANSWER_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);
/// How many following kernels one delivery pass reaches at once.
const OUTCOME_KERNELS_AT_ONCE: usize = 8;

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
            .is_ok_and(|room| self.acts_for_room_owner(&room, owner));
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
        room: &str,
        agent: &str,
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
                room: room.into(),
                agent: agent.into(),
                generation: follow.generation,
            })
            .map(|_| ())
    }

    /// Followed side: asks for every outcome that is due to go out, off the caller's path. One
    /// delivery pass runs at a time and asking while it runs makes it run once more, so a kernel
    /// that does not answer delays only its own outcomes, never a sweep or a settlement.
    pub(super) fn spawn_peer_agent_outcome_delivery(&self) {
        let Some(run) = self.peer_outcome_deliveries.ask() else {
            return;
        };
        let state = self.clone();
        tokio::spawn(async move { state.run_peer_agent_outcome_deliveries(run).await });
    }

    /// Followed side: this kernel's relay connection is back, so the kernels it owes outcomes
    /// can be reached again: every owed outcome goes out now, not when its retry falls due.
    pub(crate) fn peer_links_restored(&self) {
        self.peer_outcomes_owed_now
            .store(true, std::sync::atomic::Ordering::SeqCst);
        self.spawn_peer_agent_outcome_delivery();
    }

    async fn run_peer_agent_outcome_deliveries(&self, mut run: lifecycle_sweep_flight::SweepRun) {
        loop {
            let now = crate::session::unix_epoch_ms();
            let owed_now = self
                .peer_outcomes_owed_now
                .swap(false, std::sync::atomic::Ordering::SeqCst);
            let due_by = if owed_now { i64::MAX as u64 } else { now };
            if let Err(error) = self.deliver_peer_agent_outcomes_due_by(due_by, now).await {
                tracing::warn!(%error, "peer outcomes retained for the next delivery");
            }
            if !run.asked_again() {
                break;
            }
        }
    }

    /// One pass: each following kernel gets its outcomes in order, several kernels at once. A
    /// kernel that does not take one within the answer time keeps it and the rest for a later
    /// pass, so it costs the pass one wait at most and the other kernels none.
    async fn deliver_peer_agent_outcomes_due_by(
        &self,
        due_by: u64,
        now: u64,
    ) -> Result<(), DaemonError> {
        use futures_util::StreamExt;
        let mut by_kernel = BTreeMap::<String, Vec<PeerOutboxRow>>::new();
        for row in self
            .owned
            .durable_state_store
            .peer_agent_outbox_due(due_by, now)?
        {
            by_kernel
                .entry(row.subscriber_kernel_id.clone())
                .or_default()
                .push(row);
        }
        let failures = futures_util::stream::iter(by_kernel.into_values())
            .map(|rows| self.deliver_peer_agent_outcomes_to_kernel(rows, now))
            .buffer_unordered(OUTCOME_KERNELS_AT_ONCE)
            .filter_map(|delivered| std::future::ready(delivered.err()))
            .collect::<Vec<_>>()
            .await;
        failures.into_iter().next().map_or(Ok(()), Err)
    }

    async fn deliver_peer_agent_outcomes_to_kernel(
        &self,
        rows: Vec<PeerOutboxRow>,
        now: u64,
    ) -> Result<(), DaemonError> {
        let store = &self.owned.durable_state_store;
        let kernel_id = self.owned.config_projection.snapshot().daemon_id;
        for row in rows {
            let delivery = row.delivery_id.clone();
            let answered = tokio::time::timeout(
                OUTCOME_ANSWER_TIMEOUT,
                self.peer_agent_link(
                    &row.owner,
                    &row.subscriber_kernel_id.clone(),
                    outcome_request(&kernel_id, row.clone()),
                ),
            )
            .await;
            if let Ok(Ok(RelayPeerResponse::PeerAgentOutcomeAck { ack, .. })) = answered {
                store.agent_lifecycle(Operation::PeerOutboxDelivered { delivery })?;
                // A follow goes on until its follower says it is over; a follower that no longer
                // holds it must not be owed the next turn end.
                if ack == PeerOutcomeAck::UnknownSubscription {
                    store.agent_lifecycle(Operation::UnfollowPeer {
                        kernel: row.subscriber_kernel_id.clone(),
                        subscription: row.subscription_id.clone(),
                        generation: i64::MAX as u64,
                    })?;
                }
                continue;
            }
            store.agent_lifecycle(Operation::PeerOutboxRetry {
                delivery,
                now,
                at: now.saturating_add(
                    RETRY_FIRST_MS
                        .saturating_mul(1 << row.attempts.min(16))
                        .min(RETRY_CEILING_MS),
                ),
            })?;
            return Ok(());
        }
        Ok(())
    }

    /// Delivers what is due and waits until no delivery pass runs.
    #[cfg(test)]
    pub(crate) async fn deliver_peer_agent_outcomes(&self) {
        self.spawn_peer_agent_outcome_delivery();
        self.peer_agent_outcome_deliveries_settled().await;
    }

    #[cfg(test)]
    pub(crate) async fn peer_agent_outcome_deliveries_settled(&self) {
        while !self.peer_outcome_deliveries.is_idle() {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    }

    /// Following side: asks for the follows that can no longer be answered to be settled, off the
    /// caller's path. One pass runs at a time and asking while it runs makes it run once more, so a
    /// Cloud that does not answer delays only this pass, never a sweep.
    pub(super) fn spawn_unpaired_peer_follow_settlement(&self, now: u64) {
        let Some(mut run) = self.unpaired_peer_follow_settlements.ask() else {
            return;
        };
        let state = self.clone();
        tokio::spawn(async move {
            let mut now = now;
            loop {
                if let Err(error) = state.settle_unpaired_peer_follows(now).await {
                    tracing::warn!(%error, "peer follows of a former owner retained for the next sweep");
                }
                if !run.asked_again() {
                    break;
                }
                now = now.max(crate::session::unix_epoch_ms());
            }
        });
    }

    /// Waits until no settlement pass runs.
    #[cfg(test)]
    pub(crate) async fn unpaired_peer_follow_settlements_settled(&self) {
        while !self.unpaired_peer_follow_settlements.is_idle() {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    }

    /// Following side: this kernel speaks for its owner to the owner's other kernels. Once it no
    /// longer does (unlinked from the owner's account, or signed in as another owner), or once
    /// the followed kernel was unlinked from the owner, no outcome reaches its follows of peers
    /// there again: each is a lost source.
    async fn settle_unpaired_peer_follows(&self, now: u64) -> Result<(), DaemonError> {
        let config = self.owned.config_projection.snapshot();
        // The same owner the sending side vouches for (`peer_agent_caller`).
        let owner = config
            .cloud_relay
            .as_ref()
            .map(|profile| profile.user_id.clone())
            .filter(|owner| !owner.is_empty());
        let store = &self.owned.durable_state_store;
        // A follow of a peer in another room here needs no owner to speak for.
        let follows = store
            .live_peer_registrations()?
            .into_iter()
            .filter_map(|registration| {
                let kernel = peer_source_stamp(&registration.source_id)?.kernel_id;
                (kernel != config.daemon_id).then_some((registration, kernel))
            })
            .collect::<Vec<_>>();
        if follows.is_empty() {
            return Ok(());
        }
        let unlinked = self.unlinked_kernels(now).await;
        for (registration, kernel) in follows {
            let Some(task) = store.agent_task(&registration.task_id)? else {
                continue;
            };
            let paired = !unlinked.contains(&kernel)
                && self
                    .owned
                    .session_store
                    .get_session(&task.room_id)
                    .is_ok_and(|room| {
                        owner
                            .as_deref()
                            .is_some_and(|owner| self.acts_for_room_owner(&room, owner))
                    });
            if paired {
                continue;
            }
            store.agent_lifecycle(Operation::SourceOutcome {
                public_answer: None,
                room: task.room_id.clone(),
                source: registration.source_id,
                occurrence: format!(
                    "{}unpaired-{}",
                    ledger::peer_follows::SOURCE_LOST_OCCURRENCE,
                    registration.id
                ),
                success: false,
                now,
            })?;
            if let Err(error) =
                Box::pin(self.deliver_agent_inbox(&task.room_id, &task.agent_id)).await
            {
                tracing::warn!(%error, agent_id = %task.agent_id,
                    "lost peer source is durable and pending delivery");
            }
        }
        Ok(())
    }

    /// The kernels Cloud's directory lists as unlinked from their owner (`REVOKED`); a stopped
    /// kernel is listed `OFFLINE` and keeps its followers. Read with this kernel's credential at
    /// most once a minute; an unreadable directory keeps the last answer.
    async fn unlinked_kernels(&self, now: u64) -> BTreeSet<String> {
        let last = self
            .unlinked_kernels
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        if let Some((read_at, unlinked)) = &last {
            if now.saturating_sub(*read_at) < KERNEL_DIRECTORY_READ_MS {
                return unlinked.clone();
            }
        }
        let profile = self
            .owned
            .config_projection
            .snapshot()
            .cloud_relay
            .filter(|profile| profile.kernel_credential.is_some());
        let Some(profile) = profile else {
            return BTreeSet::new();
        };
        let directory =
            match crate::runtime::cloud_api_client::get_cloud_kernel_directory(&profile).await {
                Ok(directory) => directory,
                Err(error) => {
                    tracing::warn!(%error, "kernel directory unreadable; peer follows kept");
                    return last.map(|(_, unlinked)| unlinked).unwrap_or_default();
                }
            };
        let unlinked = directory["targets"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|target| target["status"] == "REVOKED")
            .filter_map(|target| target["daemonId"].as_str().map(str::to_owned))
            .collect::<BTreeSet<_>>();
        *self
            .unlinked_kernels
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some((now, unlinked.clone()));
        unlinked
    }

    /// Following side: a follow held for a spawn whose answer was lost waits for the retry that
    /// names its peer. One still waiting ten minutes after this kernel life first saw it is
    /// dropped; no retry is coming.
    pub(super) fn drop_unknown_peer_follows(&self, now: u64) -> Result<(), DaemonError> {
        let store = &self.owned.durable_state_store;
        let mut first_seen = self
            .unknown_peer_follows
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut waiting = BTreeSet::new();
        for registration in store.live_peer_registrations()? {
            let unknown_peer = peer_source_stamp(&registration.source_id)
                .is_some_and(|source| source.agent_id == registration.id);
            if !unknown_peer {
                continue;
            }
            let seen = *first_seen.entry(registration.id.clone()).or_insert(now);
            let task = store.agent_task(&registration.task_id)?;
            let (Some(task), true) = (task, now.saturating_sub(seen) >= UNKNOWN_PEER_FOLLOW_MS)
            else {
                waiting.insert(registration.id);
                continue;
            };
            match store.agent_lifecycle(Operation::Unsubscribe {
                task: task.task_id,
                prompt: task.prompt_id,
                registration: registration.id.clone(),
            }) {
                Ok(_) => tracing::warn!(registration = %registration.id,
                    "peer follow dropped: the spawn it was held for was not retried"),
                Err(error) => {
                    tracing::warn!(%error, registration = %registration.id,
                        "peer follow of an unanswered spawn retained for the next sweep");
                    waiting.insert(registration.id);
                }
            }
        }
        first_seen.retain(|id, _| waiting.contains(id));
        Ok(())
    }

    /// Followed side: a followed agent or room that is gone tells its followers so. An archived
    /// agent is gone for them too: it runs no turn, and an unarchived one takes new follows.
    pub(super) fn settle_lost_peer_follows(&self, now: u64) -> Result<(), DaemonError> {
        let store = &self.owned.durable_state_store;
        for (room, agent) in store.peer_followed_agents()? {
            let present = self.owned.session_store.get_session(&room).is_ok()
                && self
                    .owned
                    .agent_store
                    .get_agent(&agent)
                    .is_ok_and(|found| found.session_id() == room && !found.archived());
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
