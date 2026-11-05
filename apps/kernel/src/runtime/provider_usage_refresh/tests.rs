use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::time::Duration;

use crate::account_profile::{
    ProviderAccountAuthState, ProviderAccountProfileRegistry, ProviderAccountUsageAvailability,
    ProviderAccountUsageMeter, ProviderAccountUsageMeterKind, ProviderAccountUsageMeterScope,
    ProviderAccountUsageMeterState, ProviderAccountUsageSnapshot,
};
use crate::provider::ProviderUsageReadError;

use super::*;

const MINUTE_MS: u64 = 60_000;

fn meter(
    id: &str,
    percent: f64,
    observed_at_ms: u64,
    resets_at_ms: u64,
) -> ProviderAccountUsageMeter {
    ProviderAccountUsageMeter {
        meter_id: format!("rate_limit/{id}"),
        label: id.to_string(),
        service_id: None,
        kind: ProviderAccountUsageMeterKind::RollingLimit,
        scope: ProviderAccountUsageMeterScope::Account,
        used_percent: Some(percent),
        used: None,
        remaining: None,
        total: None,
        unit: None,
        window_duration_minutes: Some(300),
        resets_at_ms: Some(resets_at_ms),
        state: ProviderAccountUsageMeterState::Healthy,
        source: "provider".to_string(),
        observed_at_ms,
    }
}

fn snapshot(percent: f64, observed_at_ms: u64) -> ProviderAccountUsageSnapshot {
    ProviderAccountUsageSnapshot {
        profile_id: String::new(),
        provider: "claude".to_string(),
        availability: ProviderAccountUsageAvailability::Available,
        meters: vec![meter(
            "five_hour",
            percent,
            observed_at_ms,
            observed_at_ms + 5 * 60 * MINUTE_MS,
        )],
        observed_at_ms: Some(observed_at_ms),
        source: "provider".to_string(),
        management_url: None,
    }
}

/// Answers each read from a script and counts them.
struct ScriptedReader {
    answers: Mutex<VecDeque<Result<f64, ProviderUsageReadError>>>,
    clock: Arc<AtomicU64>,
    reads: Arc<AtomicUsize>,
}

impl ProviderUsageReader for ScriptedReader {
    fn read(
        &self,
        _account: &UsageAccountKey,
    ) -> Result<ProviderAccountUsageSnapshot, ProviderUsageReadError> {
        self.reads.fetch_add(1, Ordering::SeqCst);
        let answer = self
            .answers
            .lock()
            .unwrap()
            .pop_front()
            .expect("scripted answer");
        answer.map(|percent| snapshot(percent, self.clock.load(Ordering::SeqCst)))
    }
}

struct Fixture {
    refresher: Arc<ProviderUsageRefresher>,
    registry: ProviderAccountProfileRegistry,
    key: UsageAccountKey,
    t0: u64,
    clock: Arc<AtomicU64>,
    reads: Arc<AtomicUsize>,
    running: Arc<AtomicBool>,
    _root: crate::test_support::TestWorktree,
}

impl Fixture {
    fn new(answers: Vec<Result<f64, ProviderUsageReadError>>) -> Self {
        let root = crate::test_support::TestWorktree::new("usage-refresh");
        let registry =
            ProviderAccountProfileRegistry::open(root.path().join("accounts.json")).unwrap();
        let profile = registry.create_managed("local", "claude", "Work").unwrap();
        registry
            .update_observation(
                "local",
                "claude",
                &profile.profile_id,
                ProviderAccountAuthState::Authenticated,
                Some("work@example.test".into()),
                None,
                None,
                None,
            )
            .unwrap();
        let t0 = crate::session::unix_epoch_ms();
        let clock = Arc::new(AtomicU64::new(t0));
        let reads = Arc::new(AtomicUsize::new(0));
        let running = Arc::new(AtomicBool::new(false));
        let reader = ScriptedReader {
            answers: Mutex::new(answers.into()),
            clock: Arc::clone(&clock),
            reads: Arc::clone(&reads),
        };
        let running_flag = Arc::clone(&running);
        let refresher = ProviderUsageRefresher::new(
            registry.clone(),
            Box::new(reader),
            Box::new(move |_| running_flag.load(Ordering::SeqCst)),
        );
        Self {
            refresher,
            key: UsageAccountKey {
                owner_user_id: "local".into(),
                provider: "claude".into(),
                profile_id: profile.profile_id,
            },
            registry,
            t0,
            clock,
            reads,
            running,
            _root: root,
        }
    }

    fn at(&self, minutes: f64) -> u64 {
        let now = self.t0 + (minutes * MINUTE_MS as f64) as u64;
        self.clock.store(now, Ordering::SeqCst);
        now
    }

    fn usage(&self) -> ProviderAccountUsageSnapshot {
        self.registry
            .get("local", "claude", &self.key.profile_id)
            .unwrap()
            .usage
    }
}

#[test]
fn an_account_never_read_is_due_at_once_and_the_reading_carries_source_and_time() {
    crate::test_support::isolated_env_test!();
    let f = Fixture::new(vec![Ok(5.0)]);
    assert_eq!(f.refresher.due_accounts(f.at(0.0)), vec![f.key.clone()]);

    assert_eq!(
        f.refresher.refresh_account(&f.key, f.at(0.0)),
        UsageRefreshOutcome::Refreshed
    );

    let usage = f.usage();
    assert_eq!(usage.meters[0].used_percent, Some(5.0));
    assert_eq!(usage.meters[0].source, "refresh");
    assert_eq!(usage.meters[0].observed_at_ms, f.t0);
    assert_eq!(usage.source, "refresh");
}

#[test]
fn an_idle_account_is_read_again_within_fifteen_minutes_and_not_before_twelve() {
    crate::test_support::isolated_env_test!();
    let f = Fixture::new(vec![Ok(5.0)]);
    f.refresher.refresh_account(&f.key, f.at(0.0));

    assert!(f.refresher.due_accounts(f.at(1.0)).is_empty());
    assert!(f.refresher.due_accounts(f.at(12.0)).is_empty());
    // The early start is at most 1/8 of the interval, so 14m01s is always past it.
    assert_eq!(f.refresher.due_accounts(f.at(14.02)), vec![f.key.clone()]);
}

#[test]
fn an_account_with_a_running_agent_is_read_every_five_minutes() {
    crate::test_support::isolated_env_test!();
    let f = Fixture::new(vec![Ok(5.0)]);
    f.refresher.refresh_account(&f.key, f.at(0.0));

    assert!(f.refresher.due_accounts(f.at(4.0)).is_empty());
    f.running.store(true, Ordering::SeqCst);
    assert!(f.refresher.due_accounts(f.at(4.0)).is_empty());
    assert_eq!(f.refresher.due_accounts(f.at(4.7)), vec![f.key.clone()]);
    f.running.store(false, Ordering::SeqCst);
    assert!(f.refresher.due_accounts(f.at(4.7)).is_empty());
}

#[test]
fn a_request_renews_only_readings_older_than_five_minutes() {
    crate::test_support::isolated_env_test!();
    let f = Fixture::new(vec![Ok(5.0)]);
    assert_eq!(
        f.refresher.stale_accounts("local", None, f.at(0.0)),
        vec![f.key.clone()]
    );
    f.refresher.refresh_account(&f.key, f.at(0.0));

    assert!(f
        .refresher
        .stale_accounts("local", None, f.at(4.9))
        .is_empty());
    assert_eq!(
        f.refresher
            .stale_accounts("local", Some("claude"), f.at(5.1)),
        vec![f.key.clone()]
    );
    assert!(f
        .refresher
        .stale_accounts("local", Some("codex"), f.at(5.1))
        .is_empty());
    assert!(f
        .refresher
        .stale_accounts("someone-else", None, f.at(5.1))
        .is_empty());
}

#[test]
fn a_failed_read_keeps_the_last_reading_and_backs_off_with_doubling_waits() {
    crate::test_support::isolated_env_test!();
    let f = Fixture::new(vec![
        Ok(5.0),
        Err(ProviderUsageReadError::new("provider changed shape")),
        Err(ProviderUsageReadError::new("provider changed shape")),
        Ok(7.0),
    ]);
    f.refresher.refresh_account(&f.key, f.at(0.0));
    let before = f.usage();

    assert_eq!(
        f.refresher.refresh_account(&f.key, f.at(20.0)),
        UsageRefreshOutcome::Failed
    );
    assert_eq!(
        f.usage(),
        before,
        "a failed read must not touch the last reading"
    );
    assert_eq!(f.usage().meters[0].observed_at_ms, f.t0);

    // First failure waits about a minute, the second about two.
    assert!(f.refresher.due_accounts(f.at(20.5)).is_empty());
    assert!(f
        .refresher
        .stale_accounts("local", None, f.at(20.5))
        .is_empty());
    assert_eq!(f.refresher.due_accounts(f.at(21.2)), vec![f.key.clone()]);
    assert_eq!(
        f.refresher.refresh_account(&f.key, f.at(21.2)),
        UsageRefreshOutcome::Failed
    );
    assert!(f.refresher.due_accounts(f.at(22.5)).is_empty());
    assert_eq!(f.refresher.due_accounts(f.at(23.5)), vec![f.key.clone()]);

    assert_eq!(
        f.refresher.refresh_account(&f.key, f.at(23.5)),
        UsageRefreshOutcome::Refreshed
    );
    assert_eq!(f.usage().meters[0].used_percent, Some(7.0));
    assert_eq!(f.reads.load(Ordering::SeqCst), 4);
}

#[test]
fn a_selected_candidate_cannot_bypass_a_retry_delay_installed_by_another_reader() {
    crate::test_support::isolated_env_test!();
    let f = Fixture::new(vec![
        Err(ProviderUsageReadError::new("rate limited")
            .retry_after(Some(Duration::from_secs(60)))),
        Ok(7.0),
    ]);
    // Both callers select the account before either worker claims the read.
    let first = f.refresher.due_accounts(f.at(0.0));
    let second = f.refresher.stale_accounts("local", None, f.at(0.0));
    assert_eq!(first, vec![f.key.clone()]);
    assert_eq!(second, first);
    assert_eq!(
        f.refresher.refresh_account(&first[0], f.at(0.0)),
        UsageRefreshOutcome::Failed
    );
    let retry_at = f.refresher.lock()[&f.key].no_read_before_ms;

    assert_eq!(
        f.refresher.refresh_account(&second[0], f.at(0.1)),
        UsageRefreshOutcome::BackingOff
    );

    assert_eq!(f.reads.load(Ordering::SeqCst), 1);
    assert_eq!(f.refresher.lock()[&f.key].no_read_before_ms, retry_at);
    assert!(f.refresher.due_accounts(f.at(0.5)).is_empty());
    assert_eq!(
        f.refresher.refresh_account(&second[0], f.at(1.2)),
        UsageRefreshOutcome::Refreshed
    );
    assert_eq!(f.reads.load(Ordering::SeqCst), 2);
}

#[test]
fn a_wait_the_provider_names_is_honored_and_the_wait_is_capped_at_one_hour_without_one() {
    crate::test_support::isolated_env_test!();
    let f =
        Fixture::new(vec![Err(ProviderUsageReadError::new("rate limited")
            .retry_after(Some(Duration::from_secs(3_596))))]);

    f.refresher.refresh_account(&f.key, f.at(0.0));

    assert!(f.refresher.due_accounts(f.at(59.0)).is_empty());
    assert_eq!(f.refresher.due_accounts(f.at(66.0)), vec![f.key.clone()]);
    // Without a named wait the doubling stops at an hour (plus up to 10% spread).
    assert!((60..=66).contains(&(failure_wait(40, None).as_secs() / 60)));
}

#[test]
fn only_one_read_runs_per_account_at_a_time() {
    crate::test_support::isolated_env_test!();
    struct BlockedReader {
        entered: mpsc::SyncSender<()>,
        release: Mutex<mpsc::Receiver<()>>,
        reads: Arc<AtomicUsize>,
    }
    impl ProviderUsageReader for BlockedReader {
        fn read(
            &self,
            _account: &UsageAccountKey,
        ) -> Result<ProviderAccountUsageSnapshot, ProviderUsageReadError> {
            self.reads.fetch_add(1, Ordering::SeqCst);
            self.entered.send(()).unwrap();
            self.release.lock().unwrap().recv().unwrap();
            Ok(snapshot(1.0, crate::session::unix_epoch_ms()))
        }
    }
    let f = Fixture::new(vec![]);
    let (entered_tx, entered_rx) = mpsc::sync_channel(1);
    let (release_tx, release_rx) = mpsc::channel();
    let reads = Arc::new(AtomicUsize::new(0));
    let refresher = ProviderUsageRefresher::new(
        f.registry.clone(),
        Box::new(BlockedReader {
            entered: entered_tx,
            release: Mutex::new(release_rx),
            reads: Arc::clone(&reads),
        }),
        Box::new(|_| false),
    );
    let key = f.key.clone();
    let first = {
        let refresher = Arc::clone(&refresher);
        let key = key.clone();
        std::thread::spawn(move || refresher.refresh_account(&key, f.t0))
    };
    entered_rx.recv().unwrap();

    assert!(refresher.due_accounts(f.t0 + 60 * MINUTE_MS).is_empty());
    assert_eq!(
        refresher.refresh_account(&key, f.t0),
        UsageRefreshOutcome::Joined
    );
    release_tx.send(()).unwrap();

    assert_eq!(first.join().unwrap(), UsageRefreshOutcome::Refreshed);
    assert_eq!(reads.load(Ordering::SeqCst), 1);
}

#[test]
fn accounts_that_are_not_signed_in_are_not_read() {
    crate::test_support::isolated_env_test!();
    let f = Fixture::new(vec![]);
    f.registry
        .update_observation(
            "local",
            "claude",
            &f.key.profile_id,
            ProviderAccountAuthState::NotConfigured,
            None,
            None,
            None,
            None,
        )
        .unwrap();

    assert!(f.refresher.due_accounts(f.at(0.0)).is_empty());
    assert!(f
        .refresher
        .stale_accounts("local", None, f.at(0.0))
        .is_empty());
}

#[test]
fn a_window_past_its_reset_reads_as_not_measured_yet_until_a_new_reading() {
    crate::test_support::isolated_env_test!();
    let f = Fixture::new(vec![Ok(63.0), Ok(2.0)]);
    f.refresher.refresh_account(&f.key, f.at(0.0));
    let current = f
        .registry
        .get("local", "claude", &f.key.profile_id)
        .unwrap();
    assert_eq!(current.usage.meters[0].used_percent, Some(63.0));

    // The last reading is six hours old: its 5-hour window reset an hour ago.
    let six_hours = 6 * 60 * MINUTE_MS;
    f.registry
        .update_usage(
            "local",
            "claude",
            &f.key.profile_id,
            snapshot(63.0, f.t0 - six_hours),
        )
        .unwrap();

    let expired = f
        .registry
        .get("local", "claude", &f.key.profile_id)
        .unwrap();
    let meter = &expired.usage.meters[0];
    assert_eq!(
        meter.used_percent, None,
        "a stale percent must not be shown"
    );
    assert_eq!(meter.state, ProviderAccountUsageMeterState::Unknown);
    assert_eq!(meter.observed_at_ms, f.t0 - six_hours);
    let listed = f.registry.list("local", Some("claude")).unwrap();
    assert_eq!(listed[0].usage.meters[0].used_percent, None);
    let accounts = f.registry.usage_accounts("local", "machine").unwrap();
    assert_eq!(accounts[0].usage.meters[0].used_percent, None);
    assert_eq!(f.refresher.due_accounts(f.at(0.0)), vec![f.key.clone()]);

    f.refresher.refresh_account(&f.key, f.at(0.0));
    let renewed = f
        .registry
        .get("local", "claude", &f.key.profile_id)
        .unwrap();
    assert_eq!(renewed.usage.meters[0].used_percent, Some(2.0));
}
