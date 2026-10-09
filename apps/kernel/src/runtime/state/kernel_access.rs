use super::KernelRuntimeState;
use crate::error::DaemonError;
use crate::local::*;
use crate::runtime::kernel_access::{
    error,
    process::{self, ProcessIdentity},
    Grant,
};
use crate::session::{RuntimeInteraction, RuntimeInteractionChoice};
use std::time::{Duration, Instant};

impl KernelRuntimeState {
    fn audit_access(
        &self,
        grant: &KernelAccessGrant,
        outcome: &str,
        reason: Option<&str>,
    ) -> Result<(), DaemonError> {
        self.owned.durable_state_store.append_event(
            "kernel_access.grant",
            Some(grant.grant_id.clone()),
            serde_json::json!({ "outcome": outcome, "grant": grant, "reason": reason }),
        )?;
        Ok(())
    }

    pub(crate) fn revoke_kernel_access(
        &self,
        owner: Option<&str>,
        id: Option<&str>,
        reason: &str,
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
            })
            .map(|(id, _)| id.clone())
            .collect::<Vec<_>>();
        let count = ids.len();
        let revoked = ids
            .into_iter()
            .filter_map(|id| state.grants.remove(&id))
            .collect::<Vec<_>>();
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
        for grant in revoked {
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
        Ok(count + sudo_count)
    }

    fn cancel_access_prompt(&self, grant: &KernelAccessGrant, action: &str) {
        let _ = self.owned.timeout_runtime_interaction(
            crate::runtime::kernel_access::ACCESS_INTERACTION_SCOPE,
            &format!("{}-{action}", grant.grant_id),
        );
    }

    /// Called by both the pump and each request/delivery. Revocations do not wait for inbound traffic.
    pub(crate) fn sweep_kernel_access(&self) {
        self.sweep_sudo();
        let mut state = self
            .owned
            .kernel_access
            .lock()
            .expect("access state poisoned");
        let now = Instant::now();
        let expired = state
            .grants
            .iter()
            .filter_map(|(id, grant)| {
                let reason = if now >= grant.deadline {
                    Some("expiry")
                } else if !grant.holder.alive() {
                    Some("process_exit")
                } else {
                    None
                };
                reason.map(|reason| (id.clone(), reason))
            })
            .collect::<Vec<_>>();
        let removed = expired
            .into_iter()
            .filter_map(|(id, reason)| state.grants.remove(&id).map(|grant| (grant, reason)))
            .collect::<Vec<_>>();
        drop(state);
        for (grant, reason) in removed {
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
        let mut grants: Vec<_> = self
            .owned
            .kernel_access
            .lock()
            .expect("access state poisoned")
            .grants
            .values()
            .filter(|grant| grant.holder.contains(peer))
            .cloned()
            .collect();
        // A process's own grant takes precedence over an inherited grant for
        // all requests. Authority is local-kernel-wide.
        grants.sort_by_key(|grant| grant.holder != *peer);
        grants
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
            process::holder(&peer, request.holder_pid).map_err(|e| error(e.to_string()))?;
        let config = self
            .owned
            .config_projection
            .snapshot()
            .user_config
            .kernel_access;
        let minutes = request
            .lifetime_minutes
            .unwrap_or(config.grant_default_minutes);
        if minutes == 0 || minutes > config.grant_max_minutes {
            return Err(error("requested lifetime exceeds kernel access policy"));
        }
        let id = format!("access-{:016x}", rand::random::<u64>());
        let key = (holder.pid, holder.start, holder.executable.clone());
        let summary = KernelAccessGrant {
            grant_id: id.clone(),
            owner_user_id: self.owned.config_projection.local_owner_user_id(),
            holder_pid: holder.pid,
            holder_executable: holder.executable.clone(),
            lifetime_minutes: minutes,
            expires_at_ms: 0,
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
        let minutes = result?;
        if !admitted || state.generation != generation || !holder.contains(&peer) {
            self.audit_access(&summary, "revoked", Some("authorization_invalidated"))?;
            return Err(error(
                "access request invalidated before approval completed",
            ));
        }
        let mut summary = summary;
        summary.lifetime_minutes = minutes;
        summary.expires_at_ms =
            crate::session::unix_epoch_ms().saturating_add(u64::from(minutes) * 60_000);
        self.audit_access(&summary, "granted", None)?;
        state.grants.insert(
            id,
            Grant {
                summary: summary.clone(),
                holder,
                deadline: Instant::now() + Duration::from_secs(u64::from(minutes) * 60),
                notice: Instant::now()
                    + Duration::from_secs(
                        u64::from(minutes.saturating_sub(config.grant_extend_notice_minutes)) * 60,
                    ),
                notice_sent: false,
            },
        );
        Ok(summary)
    }

    async fn access_decision(
        &self,
        grant: &KernelAccessGrant,
        holder: &ProcessIdentity,
        action: &str,
        timeout_sec: u64,
    ) -> Result<u32, DaemonError> {
        self.audit_access(grant, "requested", Some(action))?;
        let remaining = grant
            .expires_at_ms
            .saturating_sub(crate::session::unix_epoch_ms())
            .div_ceil(60_000);
        let title = if action == "grant" {
            "Grant external agent access".to_owned()
        } else {
            format!("External agent access expires in {remaining} minutes. Extend?")
        };
        let interaction = RuntimeInteraction::for_kernel_operation(
            format!("{}-{action}", grant.grant_id), format!("access-{action}:{}", grant.grant_id),
            title,
            format!("OS-verified external agent {} (pid {}) requests {} access to the whole LOCAL kernel for {} minutes. Critical approvals, secret reads, and remote kernels remain unavailable. Only this process and its OS descendants will have access. Chariox agents it spawns receive no grant.",
                crate::runtime::kernel_access::requester::display_executable(&grant.holder_executable), grant.holder_pid, action, grant.lifetime_minutes),
            vec![RuntimeInteractionChoice::new("refuse", "Refuse", "refuse", None),
                RuntimeInteractionChoice::new("approve", "Approve", "approve", None).requiring_passkey()])
            .with_timeout_sec(timeout_sec)
            .with_requester(crate::runtime::kernel_access::requester::project(holder));
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
                let minutes = answer
                    .reply
                    .as_deref()
                    .and_then(|reply| reply.parse::<u32>().ok())
                    .unwrap_or(grant.lifetime_minutes);
                if minutes == 0
                    || minutes
                        > self
                            .owned
                            .config_projection
                            .snapshot()
                            .user_config
                            .kernel_access
                            .grant_max_minutes
                {
                    return Err(error("approval lifetime exceeds the current policy"));
                }
                Ok(minutes)
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
                    if grant.notice_sent || Instant::now() < grant.notice {
                        return None;
                    }
                    grant.notice_sent = true;
                    Some(grant.clone())
                })
                .collect::<Vec<_>>()
        };
        for grant in notices {
            let runtime = self.clone();
            tokio::spawn(async move {
                let seconds = grant
                    .deadline
                    .saturating_duration_since(Instant::now())
                    .as_secs()
                    .max(1);
                let Ok(minutes) = runtime
                    .access_decision(&grant.summary, &grant.holder, "extension", seconds)
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
                if let Some(current) = state.grants.get_mut(&grant.summary.grant_id) {
                    let minutes = minutes.min(config.grant_max_minutes);
                    current.summary.lifetime_minutes = minutes;
                    current.summary.expires_at_ms =
                        crate::session::unix_epoch_ms().saturating_add(u64::from(minutes) * 60_000);
                    current.deadline =
                        Instant::now() + Duration::from_secs(u64::from(minutes) * 60);
                    current.notice = Instant::now()
                        + Duration::from_secs(
                            u64::from(minutes.saturating_sub(config.grant_extend_notice_minutes))
                                * 60,
                        );
                    current.notice_sent = false;
                    let _ = runtime.audit_access(&current.summary, "extended", None);
                }
            });
        }
    }
}

mod policy;
#[cfg(test)]
pub(super) mod test_support;
