//! Keeps account usage current through the providers' official tools.
//!
//! Codex reads its app-server without a turn every 15 minutes, every 5 minutes
//! while an agent runs on the account, and on request when a reading is older
//! than 5 minutes. Claude has no usage read without a model turn, so it is read
//! (one minimal turn) only while the kernel holds prompt work (a turn in progress
//! or queued) for an agent on the account, at most every 5 minutes. An attached
//! run that finished its prompt is idle: the account keeps its last observation
//! from turn output.
//! Reads run outside turns, are single-flight per account and back off after a
//! failure, preserving the last observation.
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use rand::Rng;
use tokio::sync::{watch, Notify};

use crate::account_profile::{
    ProviderAccountAuthState, ProviderAccountProfile, ProviderAccountProfileOrigin,
    ProviderAccountProfileRegistry, ProviderAccountUsageAvailability, ProviderAccountUsageSnapshot,
};
use crate::provider::ProviderUsageReadError;

pub(crate) const USAGE_REFRESH_INTERVAL: Duration = Duration::from_secs(15 * 60);
pub(crate) const RUNNING_USAGE_REFRESH_INTERVAL: Duration = Duration::from_secs(5 * 60);
pub(crate) const ON_DEMAND_USAGE_MAX_AGE: Duration = Duration::from_secs(5 * 60);
const SCHEDULER_TICK: Duration = Duration::from_secs(15);
/// Clients wait about five seconds for an answer; a slower read finishes in
/// the background and the next request sees it.
const ON_DEMAND_WAIT: Duration = Duration::from_secs(4);
const FAILURE_BACKOFF_BASE: Duration = Duration::from_secs(60);
const FAILURE_BACKOFF_CAP: Duration = Duration::from_secs(60 * 60);
/// Reads start this fraction of the interval early, spread per account, so the
/// reading is never older than the interval and accounts do not move in step.
const EARLY_START_FRACTION: std::ops::RangeInclusive<f64> = (1.0 / 15.0)..=(1.0 / 8.0);

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct UsageAccountKey {
    pub(crate) owner_user_id: String,
    pub(crate) provider: String,
    pub(crate) profile_id: String,
}

/// One provider's read-only account usage source.
pub(crate) trait ProviderUsageReader: Send + Sync {
    fn read(
        &self,
        account: &UsageAccountKey,
    ) -> Result<ProviderAccountUsageSnapshot, ProviderUsageReadError>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum UsageRefreshOutcome {
    Refreshed,
    Failed,
    /// Another read of this account was already running.
    Joined,
    /// A completed read installed a retry delay after this candidate was selected.
    BackingOff,
    /// The read would cost a model turn and the account had no prompt work left
    /// when it was about to start.
    Idle,
}

/// Accounts as a (provider, profile id) pair.
pub(crate) type AccountSet = HashSet<(String, String)>;

struct AccountSchedule {
    in_flight: bool,
    failures: u32,
    no_read_before_ms: u64,
    early_start: f64,
}

impl Default for AccountSchedule {
    fn default() -> Self {
        Self {
            in_flight: false,
            failures: 0,
            no_read_before_ms: 0,
            early_start: rand::thread_rng().gen_range(EARLY_START_FRACTION),
        }
    }
}

pub(crate) struct ProviderUsageRefresher {
    registry: ProviderAccountProfileRegistry,
    reader: Box<dyn ProviderUsageReader>,
    running_accounts: Box<dyn Fn() -> AccountSet + Send + Sync>,
    /// Accounts with a turn in progress or queued prompts. Only these may pay
    /// for a read that costs a model turn.
    prompting_accounts: Box<dyn Fn() -> AccountSet + Send + Sync>,
    now_ms: Box<dyn Fn() -> u64 + Send + Sync>,
    schedules: Mutex<HashMap<UsageAccountKey, AccountSchedule>>,
    finished: Notify,
}

impl ProviderUsageRefresher {
    pub(crate) fn new(
        registry: ProviderAccountProfileRegistry,
        reader: Box<dyn ProviderUsageReader>,
        running_accounts: Box<dyn Fn() -> AccountSet + Send + Sync>,
        prompting_accounts: Box<dyn Fn() -> AccountSet + Send + Sync>,
    ) -> Arc<Self> {
        Arc::new(Self {
            registry,
            reader,
            running_accounts,
            prompting_accounts,
            now_ms: Box::new(crate::session::unix_epoch_ms),
            schedules: Mutex::new(HashMap::new()),
            finished: Notify::new(),
        })
    }

    /// Accounts the kernel can read: signed in or not yet checked, on a
    /// provider with an account usage source.
    fn enrolled(
        &self,
        owner_user_id: Option<&str>,
        provider: Option<&str>,
        prompting_accounts: &AccountSet,
    ) -> Vec<ProviderAccountProfile> {
        let provider = provider.and_then(crate::provider::canonical_provider_family);
        self.registry
            .list_all()
            .unwrap_or_default()
            .into_iter()
            .filter(|profile| {
                owner_user_id.is_none_or(|owner| profile.owner_user_id == owner)
                    && match profile.provider.as_str() {
                        "codex" => true,
                        // Every Claude read costs a model turn.
                        "claude" => prompting_accounts
                            .contains(&(profile.provider.clone(), profile.profile_id.clone())),
                        _ => false,
                    }
                    && provider.is_none_or(|provider| profile.provider == provider)
                    && match profile.auth_state {
                        ProviderAccountAuthState::Authenticated => true,
                        // Startup reserves native defaults nobody has linked a login to.
                        ProviderAccountAuthState::Unknown => {
                            profile.origin != ProviderAccountProfileOrigin::Default
                        }
                        _ => false,
                    }
            })
            .collect()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<UsageAccountKey, AccountSchedule>> {
        self.schedules
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }

    /// Accounts whose reading has reached the interval of their state.
    pub(crate) fn due_accounts(&self, now_ms: u64) -> Vec<UsageAccountKey> {
        // Read external projections before locking refresh bookkeeping so a
        // contended source cannot delay account requests waiting on this lock.
        let running_accounts = (self.running_accounts)();
        let prompting_accounts = (self.prompting_accounts)();
        let enrolled = self.enrolled(None, None, &prompting_accounts);
        let mut schedules = self.lock();
        enrolled
            .into_iter()
            .filter_map(|profile| {
                let key = key_of(&profile);
                let schedule = schedules.entry(key.clone()).or_default();
                let account = (key.provider.clone(), key.profile_id.clone());
                let interval = if running_accounts.contains(&account)
                    || prompting_accounts.contains(&account)
                {
                    RUNNING_USAGE_REFRESH_INTERVAL
                } else {
                    USAGE_REFRESH_INTERVAL
                };
                let wait_ms = interval.mul_f64(1.0 - schedule.early_start).as_millis() as u64;
                let due = reading_age_ms(&profile.usage, now_ms).is_none_or(|age| age >= wait_ms);
                (due && !schedule.in_flight && now_ms >= schedule.no_read_before_ms).then_some(key)
            })
            .collect()
    }

    /// Accounts of one owner whose reading is older than the on-demand age.
    pub(crate) fn stale_accounts(
        &self,
        owner_user_id: &str,
        provider: Option<&str>,
        now_ms: u64,
    ) -> Vec<UsageAccountKey> {
        let enrolled = self.enrolled(Some(owner_user_id), provider, &(self.prompting_accounts)());
        let mut schedules = self.lock();
        enrolled
            .into_iter()
            .filter_map(|profile| {
                let key = key_of(&profile);
                let schedule = schedules.entry(key.clone()).or_default();
                let stale = reading_age_ms(&profile.usage, now_ms)
                    .is_none_or(|age| age > ON_DEMAND_USAGE_MAX_AGE.as_millis() as u64);
                (stale && now_ms >= schedule.no_read_before_ms).then_some(key)
            })
            .collect()
    }

    /// Reads one account. A read already running for it is joined, not repeated.
    pub(crate) fn refresh_account(
        &self,
        key: &UsageAccountKey,
        now_ms: u64,
    ) -> UsageRefreshOutcome {
        // The account can have gone idle since it was selected; check again
        // before a read that is paid for with a model turn.
        if read_costs_a_turn(&key.provider)
            && !(self.prompting_accounts)()
                .contains(&(key.provider.clone(), key.profile_id.clone()))
        {
            return UsageRefreshOutcome::Idle;
        }
        {
            let mut schedules = self.lock();
            let schedule = schedules.entry(key.clone()).or_default();
            if schedule.in_flight {
                return UsageRefreshOutcome::Joined;
            }
            if now_ms < schedule.no_read_before_ms {
                return UsageRefreshOutcome::BackingOff;
            }
            schedule.in_flight = true;
        }
        let result = self
            .reader
            .read(key)
            .and_then(|usage| self.record(key, usage));
        let finished_at_ms = (self.now_ms)();
        let outcome = {
            let mut schedules = self.lock();
            let schedule = schedules.entry(key.clone()).or_default();
            schedule.in_flight = false;
            schedule.early_start = rand::thread_rng().gen_range(EARLY_START_FRACTION);
            match result {
                Ok(()) => {
                    schedule.failures = 0;
                    schedule.no_read_before_ms = 0;
                    UsageRefreshOutcome::Refreshed
                }
                Err(error) => {
                    schedule.failures = schedule.failures.saturating_add(1);
                    let wait = failure_wait(schedule.failures, error.retry_after);
                    schedule.no_read_before_ms =
                        finished_at_ms.saturating_add(wait.as_millis() as u64);
                    let failures = schedule.failures;
                    drop(schedules);
                    crate::logging::warn_with_fields(
                        "daemon.provider",
                        "provider usage refresh failed; keeping the last reading",
                        serde_json::json!({
                            "provider": key.provider,
                            "account_profile": key.profile_id,
                            "failures": failures,
                            "retry_in_secs": wait.as_secs(),
                            "error": error.message,
                        }),
                    );
                    UsageRefreshOutcome::Failed
                }
            }
        };
        self.finished.notify_waiters();
        outcome
    }

    fn record(
        &self,
        key: &UsageAccountKey,
        usage: ProviderAccountUsageSnapshot,
    ) -> Result<(), ProviderUsageReadError> {
        if usage.availability == ProviderAccountUsageAvailability::Unavailable
            || usage.meters.is_empty()
        {
            return Err(ProviderUsageReadError::new(
                "the provider returned no usage meters",
            ));
        }
        self.registry
            .update_usage(
                &key.owner_user_id,
                &key.provider,
                &key.profile_id,
                usage.with_reading_source("refresh"),
            )
            .map(|_| ())
            .map_err(ProviderUsageReadError::from)
    }

    /// Refreshes the accounts that are due, each on its own thread.
    pub(crate) async fn refresh_due(self: &Arc<Self>) {
        let due = self.due_accounts(crate::session::unix_epoch_ms());
        self.refresh_all(due).await;
    }

    /// Refreshes the owner's stale accounts and waits a bounded time for them,
    /// so the answer to the asker carries the new reading when the provider is quick.
    pub(crate) async fn refresh_stale_for_request(
        self: &Arc<Self>,
        owner_user_id: &str,
        provider: Option<&str>,
    ) {
        let stale = self.stale_accounts(owner_user_id, provider, crate::session::unix_epoch_ms());
        let _ = tokio::time::timeout(ON_DEMAND_WAIT, async {
            self.refresh_all(stale.clone()).await;
            self.wait_until_idle(&stale).await;
        })
        .await;
    }

    /// A read another caller started is waited for, not repeated.
    async fn wait_until_idle(&self, accounts: &[UsageAccountKey]) {
        loop {
            let finished = self.finished.notified();
            tokio::pin!(finished);
            finished.as_mut().enable();
            let busy = {
                let schedules = self.lock();
                accounts.iter().any(|key| {
                    schedules
                        .get(key)
                        .is_some_and(|schedule| schedule.in_flight)
                })
            };
            if !busy {
                return;
            }
            finished.await;
        }
    }

    async fn refresh_all(self: &Arc<Self>, accounts: Vec<UsageAccountKey>) {
        let reads = accounts.into_iter().map(|key| {
            let refresher = Arc::clone(self);
            tokio::task::spawn_blocking(move || {
                refresher.refresh_account(&key, crate::session::unix_epoch_ms())
            })
        });
        for read in reads.collect::<Vec<_>>() {
            let _ = read.await;
        }
    }

    pub(crate) async fn run(self: Arc<Self>, mut shutdown: watch::Receiver<bool>) {
        let mut tick = tokio::time::interval(SCHEDULER_TICK);
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            tokio::select! {
                _ = shutdown.wait_for(|stopped| *stopped) => return,
                _ = tick.tick() => {}
            }
            tokio::select! {
                _ = shutdown.wait_for(|stopped| *stopped) => return,
                () = self.refresh_due() => {}
            }
        }
    }
}

fn read_costs_a_turn(provider: &str) -> bool {
    provider == "claude"
}

fn key_of(profile: &ProviderAccountProfile) -> UsageAccountKey {
    UsageAccountKey {
        owner_user_id: profile.owner_user_id.clone(),
        provider: profile.provider.clone(),
        profile_id: profile.profile_id.clone(),
    }
}

/// Age of the newest reading on the account; none when nothing was ever read.
fn reading_age_ms(usage: &ProviderAccountUsageSnapshot, now_ms: u64) -> Option<u64> {
    usage
        .observed_at_ms
        .into_iter()
        .chain(usage.meters.iter().map(|meter| meter.observed_at_ms))
        .max()
        .map(|observed_at_ms| now_ms.saturating_sub(observed_at_ms))
}

/// Doubling wait after each failure, never shorter than the provider asked.
fn failure_wait(failures: u32, retry_after: Option<Duration>) -> Duration {
    let doubled = FAILURE_BACKOFF_BASE
        .saturating_mul(1u32 << failures.saturating_sub(1).min(16))
        .min(FAILURE_BACKOFF_CAP);
    let wait = doubled.max(retry_after.unwrap_or_default());
    wait + wait.mul_f64(rand::thread_rng().gen_range(0.0..0.1))
}

mod sources;
#[cfg(test)]
mod tests;
