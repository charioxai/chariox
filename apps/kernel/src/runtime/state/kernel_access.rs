use super::KernelRuntimeState;
use crate::error::DaemonError;
use crate::local::KernelAccessTerm;
use crate::local::*;
use crate::runtime::kernel_access::{
    error,
    process::{self, ProcessIdentity},
    AccessState, Grant, GrantClock, OwedEnd, Refusal,
};
use crate::session::{RuntimeInteraction, RuntimeInteractionChoice};
use std::time::{Duration, Instant};

mod holder_exit;

impl KernelRuntimeState {
    fn audit_access(
        &self,
        grant: &KernelAccessGrant,
        outcome: &str,
        reason: Option<&str>,
    ) -> Result<(), DaemonError> {
        self.audit_access_of_holder(grant, None, outcome, reason)
    }

    /// `holder` is recorded with a grant so that a restart can find the same
    /// process again (see `restore_kernel_access_grants`).
    fn audit_access_of_holder(
        &self,
        grant: &KernelAccessGrant,
        holder: Option<&ProcessIdentity>,
        outcome: &str,
        reason: Option<&str>,
    ) -> Result<(), DaemonError> {
        if outcome == "granted" {
            self.owned.durable_state_store.outside_mutate(
                crate::durable_state::agent_lifecycle::outside::Mutation::Subscribe {
                    open_decisions: vec![],
                    baseline: vec![],
                    id: grant.grant_id.clone(),
                    home: true,
                    targets: Vec::new(),
                },
            )?;
        }
        if matches!(outcome, "revoked" | "expired") {
            return self.end_grant(OwedEnd {
                grant: grant.clone(),
                outcome: outcome.to_string(),
                reason: reason.unwrap_or(outcome).to_string(),
                recorded: false,
            });
        }
        self.owned.durable_state_store.append_event(
            "kernel_access.grant",
            Some(grant.grant_id.clone()),
            serde_json::json!({ "outcome": outcome, "grant": grant, "holder": holder, "reason": reason }),
        )?;
        Ok(())
    }

    /// Records the end, then closes the grant's inbox. The end is written
    /// first: it is what a restart reads, so an inbox that cannot be closed
    /// never brings a revoked grant back. A step that fails stays owed and is
    /// tried again by the next revoke or sweep, so neither a lost revocation
    /// nor an open inbox outlives a storage failure.
    fn end_grant(&self, mut owed: OwedEnd) -> Result<(), DaemonError> {
        let id = owed.grant.grant_id.clone();
        self.owned
            .kernel_access
            .lock()
            .expect("access state poisoned")
            .owed_ends
            .retain(|known| known.grant.grant_id != id);
        let mut result = Ok(());
        if !owed.recorded {
            result = self
                .owned
                .durable_state_store
                .append_event(
                    "kernel_access.grant",
                    Some(id.clone()),
                    serde_json::json!({
                        "outcome": owed.outcome,
                        "grant": owed.grant,
                        "holder": None::<ProcessIdentity>,
                        "reason": owed.reason,
                    }),
                )
                .map(|_| ());
            owed.recorded = result.is_ok();
        }
        if result.is_ok() {
            result = self.owned.durable_state_store.outside_mutate(
                crate::durable_state::agent_lifecycle::outside::Mutation::Unsubscribe {
                    id,
                    targets: Vec::new(),
                    all: true,
                    reason: owed.reason.clone(),
                },
            );
        }
        if result.is_err() {
            self.owned
                .kernel_access
                .lock()
                .expect("access state poisoned")
                .owed_ends
                .push(owed);
        }
        result
    }

    /// Finishes the ends that storage refused earlier, for the grants a revoke
    /// names (all of them when it names none). Returns how many it finished.
    fn settle_owed_ends(
        &self,
        owner: Option<&str>,
        id: Option<&str>,
    ) -> (usize, Option<DaemonError>) {
        let owed = {
            let mut state = self
                .owned
                .kernel_access
                .lock()
                .expect("access state poisoned");
            let (matching, rest) = std::mem::take(&mut state.owed_ends)
                .into_iter()
                .partition::<Vec<_>, _>(|owed| {
                    id.is_none_or(|id| id == owed.grant.grant_id)
                        && owner.is_none_or(|owner| owner == owed.grant.owner_user_id)
                });
            state.owed_ends = rest;
            matching
        };
        let mut settled = 0;
        let mut first_error = None;
        for owed in owed {
            match self.end_grant(owed) {
                Ok(()) => settled += 1,
                Err(error) => {
                    first_error.get_or_insert(error);
                }
            }
        }
        (settled, first_error)
    }

    pub(crate) fn revoke_kernel_access(
        &self,
        owner: Option<&str>,
        id: Option<&str>,
        reason: &str,
    ) -> Result<usize, DaemonError> {
        self.revoke_access(owner, id, reason, false)
    }

    /// The kernel stops. Holders waiting on events receive a shutdown
    /// reason; sudo windows and timed grants end as always, but a grant that
    /// lasts until its holder exits is kept for the next start. Returns how
    /// many grants and windows it ended or released.
    pub(crate) fn end_kernel_access_for_shutdown(&self) -> Result<usize, DaemonError> {
        let kept = {
            let mut state = self
                .owned
                .kernel_access
                .lock()
                .expect("access state poisoned");
            state.stopping = true;
            state
                .grants
                .values()
                .filter(|grant| grant.survives_restart())
                .count()
        };
        Ok(self.revoke_access(None, None, "kernel_shutdown", true)? + kept)
    }

    /// Whether the kernel has begun to stop (event waits end their poll).
    pub(crate) fn kernel_access_stopping(&self) -> bool {
        self.owned
            .kernel_access
            .lock()
            .expect("access state poisoned")
            .stopping
    }

    /// The start after a stop: grants that last until their holder exits come
    /// back with their id, so the holder's event inbox goes on. A holder that
    /// is gone, or whose pid was reused, ends its grant now.
    pub(crate) fn restore_kernel_access_grants(&self) -> Result<usize, DaemonError> {
        let mut restored = 0;
        for (id, summary, holder) in recorded_until_exit_grants(&self.owned.durable_state_store)? {
            match holder.filter(ProcessIdentity::alive) {
                Some(holder) => {
                    self.owned
                        .kernel_access
                        .lock()
                        .expect("access state poisoned")
                        .grants
                        .insert(
                            id,
                            Grant {
                                summary,
                                holder,
                                clock: None,
                            },
                        );
                    self.owned
                        .kernel_access
                        .lock()
                        .expect("access state poisoned")
                        .holder_changed
                        .notify_one();
                    restored += 1;
                }
                None => self.audit_access(&summary, "revoked", Some("process_exit"))?,
            }
        }
        Ok(restored)
    }

    fn revoke_access(
        &self,
        owner: Option<&str>,
        id: Option<&str>,
        reason: &str,
        keep_for_restart: bool,
    ) -> Result<usize, DaemonError> {
        let sudo_result = self.revoke_sudo(owner, id, reason);
        let sudo_count = sudo_result.as_ref().copied().unwrap_or(0);
        let mut state = self
            .owned
            .kernel_access
            .lock()
            .expect("access state poisoned");
        let ids = state
            .grants
            .iter()
            .filter(|(key, grant)| {
                id.is_none_or(|id| id == key.as_str())
                    && owner.is_none_or(|owner| owner == grant.summary.owner_user_id)
                    && !(keep_for_restart && grant.survives_restart())
            })
            .map(|(id, _)| id.clone())
            .collect::<Vec<_>>();
        let count = ids.len();
        let revoked = ids
            .into_iter()
            .filter_map(|id| state.grants.remove(&id))
            .collect::<Vec<_>>();
        for grant in &revoked {
            state.remember_ended(&grant.holder);
        }
        let pending_keys = state
            .pending
            .iter()
            .filter(|(_, grant)| {
                id.is_none_or(|id| id == grant.grant_id)
                    && owner.is_none_or(|owner| owner == grant.owner_user_id)
            })
            .map(|(key, _)| key.clone())
            .collect::<Vec<_>>();
        let pending = pending_keys
            .into_iter()
            .filter_map(|key| state.pending.remove(&key))
            .collect::<Vec<_>>();
        // Targeted revocation invalidates only its own pending requests. The
        // generation fence is reserved for a kernel-wide reset/shutdown.
        if id.is_none() && owner.is_none() {
            state.generation = state.generation.wrapping_add(1);
        }
        drop(state);
        let mut audit_error = sudo_result.err();
        let (settled, owed_error) = self.settle_owed_ends(owner, id);
        if audit_error.is_none() {
            audit_error = owed_error;
        }
        for grant in revoked {
            self.retire_notification_grants(Some(&grant.summary.grant_id));
            if let Err(error) = self.audit_access(&grant.summary, "revoked", Some(reason)) {
                audit_error.get_or_insert(error);
            }
        }
        for grant in pending {
            self.cancel_access_prompt(&grant, "grant");
        }
        // Extension prompts for revoked grants are removed in the same mutation.
        let prompts = self
            .owned
            .pending_interactions
            .write()
            .iter()
            .filter_map(|(key, pending)| {
                let prompt = pending.passkey_prompt.as_ref()?;
                (prompt.kind == PasskeyPromptKind::AccessExtension
                    && owner.is_none_or(|owner| {
                        pending.kernel_operation_owner.as_deref() == Some(owner)
                    })
                    && id.is_none_or(|id| key == &format!("{id}-extension")))
                .then(|| (pending.session_id.clone(), key.clone()))
            })
            .collect::<Vec<_>>();
        for (session, interaction) in prompts {
            let _ = self
                .owned
                .timeout_runtime_interaction(&session, &interaction);
        }
        if let Some(error) = audit_error {
            return Err(error);
        }
        Ok(count + sudo_count + settled)
    }

    fn cancel_access_prompt(&self, grant: &KernelAccessGrant, action: &str) {
        let _ = self.owned.timeout_runtime_interaction(
            crate::runtime::kernel_access::ACCESS_INTERACTION_SCOPE,
            &format!("{}-{action}", grant.grant_id),
        );
    }

    /// Called by both the pump and each request/delivery. Revocations do not wait for inbound traffic.
    pub(crate) fn sweep_kernel_access(&self) {
        self.sweep_kernel_access_at(Instant::now());
    }

    fn sweep_kernel_access_at(&self, now: Instant) {
        self.sweep_sudo();
        let mut state = self
            .owned
            .kernel_access
            .lock()
            .expect("access state poisoned");
        state.remote_requesters.retain(|_, remote| remote.live(now));
        let expired = state
            .grants
            .iter()
            .filter_map(|(id, grant)| Some((id.clone(), grant.ended_reason(now)?)))
            .collect::<Vec<_>>();
        let removed = expired
            .into_iter()
            .filter_map(|(id, reason)| state.grants.remove(&id).map(|grant| (grant, reason)))
            .collect::<Vec<_>>();
        for (grant, _) in &removed {
            state.remember_ended(&grant.holder);
        }
        drop(state);
        let _ = self.settle_owed_ends(None, None);
        for (grant, reason) in removed {
            self.retire_notification_grants(Some(&grant.summary.grant_id));
            self.cancel_access_prompt(&grant.summary, "extension");
            let _ = self.audit_access(
                &grant.summary,
                if reason == "expiry" {
                    "expired"
                } else {
                    "revoked"
                },
                Some(reason),
            );
        }
    }

    pub(crate) fn access_grants_for(&self, peer: &ProcessIdentity) -> Vec<Grant> {
        self.sweep_kernel_access();
        // Kernel-launched agents never inherit external authority.
        let Ok((kernel, _)) = process::inspect(std::process::id()) else {
            return Vec::new();
        };
        if kernel.contains(peer) {
            return Vec::new();
        }
        let Ok(chain) = process::ancestry(peer) else {
            return Vec::new();
        };
        let mut grants: Vec<_> = self
            .owned
            .kernel_access
            .lock()
            .expect("access state poisoned")
            .grants
            .values()
            .filter_map(|grant| {
                let distance = chain.iter().position(|p| *p == grant.holder)?;
                grant.holder.alive().then(|| (distance, grant.clone()))
            })
            .collect();
        // The nearest holder's grant comes first, the requester's own before
        // any inherited one, so separate invocations of one command series
        // keep the same grant and event inbox. Authority is local-kernel-wide.
        grants.sort_by_key(|(distance, _)| *distance);
        grants.into_iter().map(|(_, grant)| grant).collect()
    }

    /// True when this peer sits under a holder whose grant has ended.
    pub(crate) fn access_ended_for(&self, peer: &ProcessIdentity) -> bool {
        let Ok(chain) = process::ancestry(peer) else {
            return false;
        };
        let state = self
            .owned
            .kernel_access
            .lock()
            .expect("access state poisoned");
        chain.iter().any(|ancestor| state.ended.contains(ancestor))
    }

    pub(crate) fn access_grant_live(&self, id: &str, peer: &ProcessIdentity) -> bool {
        self.sweep_kernel_access();
        self.owned
            .kernel_access
            .lock()
            .expect("access state poisoned")
            .grants
            .get(id)
            .is_some_and(|grant| grant.holder.contains(peer))
    }

    pub(crate) fn access_grant_summary(&self, id: &str) -> Option<KernelAccessGrant> {
        self.owned
            .kernel_access
            .lock()
            .expect("access state poisoned")
            .grants
            .get(id)
            .map(|grant| grant.summary.clone())
    }

    pub(crate) fn list_kernel_access(&self, owner: &str) -> Vec<KernelAccessGrant> {
        self.sweep_kernel_access();
        self.owned
            .kernel_access
            .lock()
            .expect("access state poisoned")
            .grants
            .values()
            .filter(|grant| grant.summary.owner_user_id == owner)
            .map(|grant| grant.summary.clone())
            .collect()
    }

    /// Only Unix admission calls this, with an identity captured from the socket.
    pub(crate) async fn request_kernel_access(
        &self,
        peer: ProcessIdentity,
        request: RequestKernelAccessRequest,
    ) -> Result<KernelAccessGrant, DaemonError> {
        if process::inspect(std::process::id()).is_ok_and(|(kernel, _)| kernel.contains(&peer)) {
            return Err(error(
                "kernel-launched processes cannot request external access",
            ));
        }
        let holder =
            process::holder(&peer, request.holder_pid).map_err(|_| Refusal::NotCovered.error())?;
        let config = self
            .owned
            .config_projection
            .snapshot()
            .user_config
            .kernel_access;
        let term = request
            .term
            .unwrap_or(KernelAccessTerm::Minutes(config.grant_default_minutes));
        if let KernelAccessTerm::Minutes(minutes) = term {
            if minutes == 0 || minutes > config.grant_max_minutes {
                return Err(error("requested lifetime exceeds kernel access policy"));
            }
        }
        let id = format!("access-{:016x}", rand::random::<u64>());
        let key = (holder.pid, holder.start, holder.executable.clone());
        let summary = KernelAccessGrant {
            grant_id: id.clone(),
            owner_user_id: self.owned.config_projection.local_owner_user_id(),
            holder_pid: holder.pid,
            holder_executable: holder.executable.clone(),
            holder_harness: crate::runtime::kernel_access::requester::project(&holder)
                .provider_harness,
            term,
            expires_at_ms: None,
        };
        let generation = {
            let mut state = self
                .owned
                .kernel_access
                .lock()
                .expect("access state poisoned");
            if state
                .pending
                .keys()
                .any(|p| p.0 == holder.pid && p.1 == holder.start)
                || state.grants.values().any(|grant| grant.holder == holder)
            {
                return Err(error("this holder already has a grant or pending request"));
            }
            state.pending.insert(key.clone(), summary.clone());
            state.generation
        };
        let result = self
            .access_decision(
                &summary,
                &holder,
                "grant",
                u64::from(config.request_timeout_minutes) * 60,
            )
            .await;
        let mut state = self
            .owned
            .kernel_access
            .lock()
            .expect("access state poisoned");
        let admitted = state
            .pending
            .remove(&key)
            .is_some_and(|pending| pending.grant_id == summary.grant_id);
        let term = result?;
        if !admitted || state.generation != generation || !holder.contains(&peer) {
            // Ending the request also locks the access store to track owed audit work.
            drop(state);
            self.audit_access(&summary, "revoked", Some("authorization_invalidated"))?;
            return Err(error(
                "access request invalidated before approval completed",
            ));
        }
        let mut summary = summary;
        summary.term = term;
        let clock = match term {
            KernelAccessTerm::Minutes(minutes) => {
                summary.expires_at_ms = Some(
                    crate::session::unix_epoch_ms().saturating_add(u64::from(minutes) * 60_000),
                );
                Some(GrantClock::new(minutes, config.grant_extend_notice_minutes))
            }
            KernelAccessTerm::UntilHolderExits => None,
        };
        self.admit_grant(&mut state, id, summary.clone(), holder, clock)?;
        Ok(summary)
    }

    /// Records a granted request and makes it live.
    fn admit_grant(
        &self,
        state: &mut AccessState,
        id: String,
        summary: KernelAccessGrant,
        holder: ProcessIdentity,
        clock: Option<GrantClock>,
    ) -> Result<(), DaemonError> {
        self.audit_access_of_holder(&summary, Some(&holder), "granted", None)?;
        state.grants.insert(
            id,
            Grant {
                summary,
                holder,
                clock,
            },
        );
        state.holder_changed.notify_one();
        Ok(())
    }

    async fn access_decision(
        &self,
        grant: &KernelAccessGrant,
        holder: &ProcessIdentity,
        action: &str,
        timeout_sec: u64,
    ) -> Result<KernelAccessTerm, DaemonError> {
        self.audit_access(grant, "requested", Some(action))?;
        let remaining = grant
            .expires_at_ms
            .unwrap_or_default()
            .saturating_sub(crate::session::unix_epoch_ms())
            .div_ceil(60_000);
        let requester = crate::runtime::kernel_access::requester::project(holder);
        let (title, message) = crate::runtime::kernel_access::requester::popup_copy(
            requester.provider_harness,
            action == "grant",
            remaining,
        );
        let interaction = RuntimeInteraction::for_kernel_operation(
            format!("{}-{action}", grant.grant_id),
            format!("access-{action}:{}", grant.grant_id),
            title,
            message,
            vec![
                RuntimeInteractionChoice::new("refuse", "Refuse", "refuse", None),
                RuntimeInteractionChoice::new("approve", "Approve", "approve", None)
                    .requiring_passkey(),
            ],
        )
        .with_timeout_sec(timeout_sec)
        .with_requester(requester);
        let rx = self
            .create_kernel_operation_interaction(
                crate::runtime::kernel_access::ACCESS_INTERACTION_SCOPE,
                &grant.owner_user_id,
                interaction,
            )
            .await?;
        let mut rx = rx;
        let answer = tokio::time::timeout(Duration::from_secs(timeout_sec), async {
            let mut tick = tokio::time::interval(Duration::from_millis(100));
            loop {
                tokio::select! {
                    answer = &mut rx => break answer,
                    _ = tick.tick() => {
                        if !holder.alive() {
                            self.cancel_access_prompt(grant, action);
                        }
                    }
                }
            }
        })
        .await;
        match answer {
            Ok(Ok(answer)) if answer.choice_id.as_deref() == Some("approve") => {
                // The answer path already checked the reply against policy.
                match answer.reply.as_deref().filter(|reply| !reply.is_empty()) {
                    // A plain Approve resolves to the choice's reply, not a lifetime override.
                    None | Some("approve") => Ok(grant.term),
                    Some(reply) => KernelAccessTerm::from_reply(reply)
                        .ok_or_else(|| error("approval term is not recognised")),
                }
            }
            _ => {
                self.audit_access(grant, "refused", Some("refused_or_timeout"))?;
                Err(error(
                    "access request refused or expired; answer the popup in a Chariox terminal",
                ))
            }
        }
    }

    pub(crate) fn pump_kernel_access(&self) {
        self.sweep_kernel_access();
        // Dead-man for sudo window timers: the kernel pump enforces and alerts too.
        self.pump_sudo_windows(None);
        let notices = {
            let mut state = self
                .owned
                .kernel_access
                .lock()
                .expect("access state poisoned");
            state
                .grants
                .values_mut()
                .filter_map(|grant| {
                    let clock = grant.clock.as_mut()?;
                    if clock.notice_sent || Instant::now() < clock.notice {
                        return None;
                    }
                    clock.notice_sent = true;
                    Some((grant.summary.clone(), grant.holder.clone(), clock.deadline))
                })
                .collect::<Vec<_>>()
        };
        for (summary, holder, deadline) in notices {
            let runtime = self.clone();
            tokio::spawn(async move {
                let seconds = deadline
                    .saturating_duration_since(Instant::now())
                    .as_secs()
                    .max(1);
                let Ok(KernelAccessTerm::Minutes(minutes)) = runtime
                    .access_decision(&summary, &holder, "extension", seconds)
                    .await
                else {
                    return;
                };
                runtime.sweep_kernel_access();
                let config = runtime
                    .owned
                    .config_projection
                    .snapshot()
                    .user_config
                    .kernel_access;
                let mut state = runtime
                    .owned
                    .kernel_access
                    .lock()
                    .expect("access state poisoned");
                if let Some(current) = state.grants.get_mut(&summary.grant_id) {
                    let Some(clock) = current.clock.as_mut() else {
                        return;
                    };
                    let minutes = minutes.min(config.grant_max_minutes);
                    current.summary.term = KernelAccessTerm::Minutes(minutes);
                    current.summary.expires_at_ms = Some(
                        crate::session::unix_epoch_ms().saturating_add(u64::from(minutes) * 60_000),
                    );
                    *clock = GrantClock::new(minutes, config.grant_extend_notice_minutes);
                    let _ = runtime.audit_access(&current.summary, "extended", None);
                }
            });
        }
    }
}

mod policy;
#[cfg(test)]
pub(super) mod test_support;

/// Grants that last until their holder exits and were never revoked, each
/// with the holder process recorded when it was granted.
fn recorded_until_exit_grants(
    store: &crate::durable_state::DurableKernelStateStore,
) -> Result<Vec<(String, KernelAccessGrant, Option<ProcessIdentity>)>, DaemonError> {
    let mut recorded = std::collections::BTreeMap::new();
    for event in store.load_events_by_kind("kernel_access.grant")? {
        let Some(id) = event.subject_id else { continue };
        match event.payload["outcome"].as_str() {
            Some("granted") => {
                if let Ok(grant) =
                    serde_json::from_value::<KernelAccessGrant>(event.payload["grant"].clone())
                {
                    let holder = serde_json::from_value(event.payload["holder"].clone()).ok();
                    recorded.insert(id, (grant, holder));
                }
            }
            Some("revoked" | "expired") => {
                recorded.remove(&id);
            }
            _ => {}
        }
    }
    Ok(recorded
        .into_iter()
        .filter(|(_, (grant, _))| grant.term == KernelAccessTerm::UntilHolderExits)
        .map(|(id, (grant, holder))| (id, grant, holder))
        .collect())
}

/// Ids of the grants whose holder is still running: their event inboxes must
/// outlive this start.
pub(crate) fn grant_inboxes_to_keep(
    store: &crate::durable_state::DurableKernelStateStore,
) -> Result<Vec<String>, DaemonError> {
    Ok(recorded_until_exit_grants(store)?
        .into_iter()
        .filter(|(_, _, holder)| holder.as_ref().is_some_and(ProcessIdentity::alive))
        .map(|(id, ..)| id)
        .collect())
}
