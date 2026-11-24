//! When an account whose usage was used up can run again, everything parked on
//! it goes on by itself: stopped agents the kernel was to resume at the
//! provider's reset, and held workflow runs (the App event pump retries those
//! as soon as admission stops refusing). Each Room hears of it once.

use super::usage_limit_stop_transitions::StopEvent;
use super::*;
use crate::agent::UsageLimitStopState;
use std::collections::BTreeSet;

/// How often the kernel compares which accounts are blocked.
const CHECK_EVERY_MS: u64 = 10_000;

/// The accounts the last check found blocked, by owner, provider and profile.
#[derive(Default)]
pub(super) struct UsageResetWatch {
    blocked: BTreeSet<(String, String, String)>,
    checked_ms: u64,
}

impl KernelRuntimeOwnedState {
    /// The accounts that were blocked at the last check and are not now.
    fn take_cleared_accounts(
        &self,
        now_ms: u64,
    ) -> Vec<crate::account_profile::ProviderAccountProfile> {
        let mut watch = self
            .usage_reset_watch
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if now_ms < watch.checked_ms.saturating_add(CHECK_EVERY_MS) {
            return Vec::new();
        }
        watch.checked_ms = now_ms;
        let Ok(profiles) = self.provider_account_profiles.list_all() else {
            return Vec::new();
        };
        let key = |profile: &crate::account_profile::ProviderAccountProfile| {
            (
                profile.owner_user_id.clone(),
                profile.provider.clone(),
                profile.profile_id.clone(),
            )
        };
        let blocked_now = profiles
            .iter()
            .filter(|profile| profile.blocked_until("default", now_ms).is_some())
            .map(key)
            .collect::<BTreeSet<_>>();
        let cleared = profiles
            .into_iter()
            .filter(|profile| {
                watch.blocked.contains(&key(profile)) && !blocked_now.contains(&key(profile))
            })
            .collect();
        watch.blocked = blocked_now;
        cleared
    }

    /// Workflow runs whose entry is held on `account`, by Room.
    fn held_entries_on(
        &self,
        account: &crate::account_profile::ProviderAccountProfile,
    ) -> BTreeMap<String, usize> {
        let mut held = BTreeMap::new();
        for session in self.session_store.list_sessions() {
            for run in session.workflow_runs() {
                let Some(node) = run.node_runs().first() else {
                    continue;
                };
                if run.status().is_terminal() || node.waiting_reason().is_none() {
                    continue;
                }
                let on_account = node.waiting_account() == Some(account.label.as_str());
                if on_account {
                    *held.entry(session.id().to_string()).or_default() += 1;
                }
            }
        }
        held
    }
}

impl KernelRuntimeState {
    /// Called by the scheduler every second; compares accounts every ten.
    pub(super) async fn tick_provider_usage_resets(&self, now_ms: u64) {
        for account in self.owned.take_cleared_accounts(now_ms) {
            self.resume_after_reset(&account, now_ms).await;
        }
    }

    async fn resume_after_reset(
        &self,
        account: &crate::account_profile::ProviderAccountProfile,
        now_ms: u64,
    ) {
        let mut resumed: BTreeMap<String, usize> = self.owned.held_entries_on(account);
        for agent in self.owned.agent_store.list_agents() {
            let Some(stop) = agent.usage_limit_stop().cloned() else {
                continue;
            };
            // The stop names the account as the run launched on it: its id, or the
            // `default` alias of the owner's. The alias is read in the stopped agent's own
            // account authority, so one owner's reset never moves another owner's stop.
            let agent_account_owner =
                self.provider_account_authority_owner_user_id(agent.owner_user_id());
            let stopped_on_account = crate::provider::canonical_provider_family(&stop.provider)
                == Some(account.provider.as_str())
                && agent_account_owner == account.owner_user_id
                && self
                    .owned
                    .provider_account_profiles
                    .find(
                        &agent_account_owner,
                        &account.provider,
                        &stop.account_profile,
                    )
                    .ok()
                    .flatten()
                    .is_some_and(|stopped| stopped.profile_id == account.profile_id);
            if !stopped_on_account || !matches!(stop.state, UsageLimitStopState::Armed { .. }) {
                continue;
            }
            let moved = self
                .apply_stop_event(
                    agent.id(),
                    &StopEvent::Reset {
                        selected: &stop,
                        now: now_ms,
                    },
                )
                .await;
            if matches!(moved, Ok(Some(_))) {
                *resumed.entry(agent.session_id().to_string()).or_default() += 1;
            }
        }
        for (session_id, count) in resumed {
            self.owned.record_notice(
                &session_id,
                None,
                self.owned
                    .attachment_store
                    .list_session_attachment_ids(&session_id),
                format!(
                    "{} reset: {count} {} resume{} by itself.",
                    account.label,
                    if count == 1 { "item" } else { "items" },
                    if count == 1 { "s" } else { "" },
                ),
            );
        }
    }
}
