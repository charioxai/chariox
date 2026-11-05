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
        provider: "codex".to_string(),
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
    read_delay_ms: Arc<AtomicU64>,
}

impl ProviderUsageReader for ScriptedReader {
    fn read(
        &self,
        _account: &UsageAccountKey,
    ) -> Result<ProviderAccountUsageSnapshot, ProviderUsageReadError> {
        self.reads.fetch_add(1, Ordering::SeqCst);
        self.clock
            .fetch_add(self.read_delay_ms.load(Ordering::SeqCst), Ordering::SeqCst);
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
    read_delay_ms: Arc<AtomicU64>,
    running: Arc<AtomicBool>,
    _root: crate::test_support::TestWorktree,
}

impl Fixture {
    fn new(answers: Vec<Result<f64, ProviderUsageReadError>>) -> Self {
        Self::for_provider("codex", answers)
    }

    fn for_provider(provider: &str, answers: Vec<Result<f64, ProviderUsageReadError>>) -> Self {
        let root = crate::test_support::TestWorktree::new("usage-refresh");
        let registry =
            ProviderAccountProfileRegistry::open(root.path().join("accounts.json")).unwrap();
        let profile = registry.create_managed("local", provider, "Work").unwrap();
        registry
            .update_observation(
                "local",
                provider,
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
        let read_delay_ms = Arc::new(AtomicU64::new(0));
        let running = Arc::new(AtomicBool::new(false));
        let reader = ScriptedReader {
            answers: Mutex::new(answers.into()),
            clock: Arc::clone(&clock),
            reads: Arc::clone(&reads),
            read_delay_ms: Arc::clone(&read_delay_ms),
        };
        let running_flag = Arc::clone(&running);
        let running_key = (provider.to_string(), profile.profile_id.clone());
        let mut refresher = ProviderUsageRefresher::new(
            registry.clone(),
            Box::new(reader),
            Box::new(move || {
                if running_flag.load(Ordering::SeqCst) {
                    HashSet::from([running_key.clone()])
                } else {
                    HashSet::new()
                }
            }),
        );
        let refresh_clock = Arc::clone(&clock);
        Arc::get_mut(&mut refresher).unwrap().now_ms =
            Box::new(move || refresh_clock.load(Ordering::SeqCst));
        Self {
            refresher,
            key: UsageAccountKey {
                owner_user_id: "local".into(),
                provider: provider.into(),
                profile_id: profile.profile_id,
            },
            registry,
            t0,
            clock,
            reads,
            read_delay_ms,
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
            .get("local", &self.key.provider, &self.key.profile_id)
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
fn running_account_projection_is_read_before_the_schedule_lock() {
    crate::test_support::isolated_env_test!();
    let mut f = Fixture::new(vec![]);
    let target = Arc::new(Mutex::new(None::<std::sync::Weak<ProviderUsageRefresher>>));
    let source_target = Arc::clone(&target);
    Arc::get_mut(&mut f.refresher).unwrap().running_accounts = Box::new(move || {
        let refresher = source_target
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .upgrade()
            .unwrap();
        assert!(
            refresher.schedules.try_lock().is_ok(),
            "running-account projection must be read outside the schedule lock"
        );
        HashSet::new()
    });
    *target.lock().unwrap() = Some(Arc::downgrade(&f.refresher));

    assert_eq!(f.refresher.due_accounts(f.t0), vec![f.key.clone()]);
}

#[test]
fn kernel_running_account_projection_preserves_cadence_across_runtime_owners() {
    crate::test_support::isolated_env_test!();
    let f = Fixture::new(vec![Ok(5.0)]);
    f.refresher.refresh_account(&f.key, f.at(0.0));
    let runs = crate::runtime::projection::ProviderRunProjectionStore::default();
    let refresher = ProviderUsageRefresher::for_kernel(f.registry.clone(), runs.clone());
    let now = f.at(4.7);
    assert!(refresher.due_accounts(now).is_empty());

    let run = crate::provider::RuntimeProviderRun::from_control_capability_inference(
        "running-account",
        "room".into(),
        Some("agent".into()),
        "codex".into(),
    );
    let mut value = serde_json::to_value(run).unwrap();
    value["account_profile"] = serde_json::json!(f.key.profile_id);
    value["owner_user_id"] = serde_json::json!("other-owner");
    // The runtime owner can be a Cloud principal while the account belongs
    // to the local authority. Cadence follows the selected provider/profile.
    let mut run: crate::provider::RuntimeProviderRun = serde_json::from_value(value).unwrap();
    runs.update(run.clone());
    assert_eq!(refresher.due_accounts(now), vec![f.key.clone()]);
    run.mark_running();
    runs.update(run.clone());
    assert_eq!(refresher.due_accounts(now), vec![f.key.clone()]);
    run.mark_ended();
    runs.update(run);
    assert!(refresher.due_accounts(now).is_empty());
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
            .stale_accounts("local", Some("codex"), f.at(5.1)),
        vec![f.key.clone()]
    );
    assert!(f
        .refresher
        .stale_accounts("local", Some("claude"), f.at(5.1))
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
        Err(ProviderUsageReadError::new("rate limited").retry_after(Some(Duration::from_secs(60)))),
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
fn a_slow_failure_starts_the_retry_delay_when_the_reader_finishes() {
    crate::test_support::isolated_env_test!();
    let f = Fixture::new(vec![Err(
        ProviderUsageReadError::new("rate limited").retry_after(Some(Duration::from_secs(60)))
    )]);
    f.read_delay_ms.store(10_000, Ordering::SeqCst);

    assert_eq!(
        f.refresher.refresh_account(&f.key, f.at(0.0)),
        UsageRefreshOutcome::Failed
    );

    let retry_at = f.refresher.lock()[&f.key].no_read_before_ms;
    assert!(
        retry_at >= f.t0 + 70_000,
        "the promised wait must start after the ten-second read finishes"
    );
    assert!(f.refresher.due_accounts(f.at(69.0 / 60.0)).is_empty());
    assert!(f
        .refresher
        .stale_accounts("local", None, f.at(69.0 / 60.0))
        .is_empty());
    assert_eq!(
        f.refresher.due_accounts(f.at(77.0 / 60.0)),
        vec![f.key.clone()]
    );
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
        Box::new(HashSet::new),
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
            "codex",
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

#[tokio::test]
async fn claude_accounts_keep_turn_observations_without_idle_reads() {
    crate::test_support::isolated_env_test!();
    let f = Fixture::for_provider("claude", vec![]);
    assert_eq!(
        f.usage().availability,
        ProviderAccountUsageAvailability::Unavailable
    );
    assert!(f.refresher.due_accounts(f.at(0.0)).is_empty());
    assert!(f
        .refresher
        .stale_accounts("local", Some("claude"), f.at(0.0))
        .is_empty());
    f.refresher.refresh_due().await;
    f.refresher
        .refresh_stale_for_request("local", Some("claude"))
        .await;
    assert_eq!(f.reads.load(Ordering::SeqCst), 0);

    let observed_at = f.t0 - 6 * 60 * MINUTE_MS;
    let mut usage = snapshot(63.0, observed_at).with_reading_source("event");
    usage.provider = "claude".to_string();
    f.registry
        .update_usage("local", "claude", &f.key.profile_id, usage)
        .unwrap();

    assert!(f.refresher.due_accounts(f.at(0.0)).is_empty());
    assert!(f
        .refresher
        .stale_accounts("local", None, f.at(0.0))
        .is_empty());
    let usage = f.usage();
    assert_eq!(usage.observed_at_ms, Some(observed_at));
    assert_eq!(usage.source, "event");
    assert_eq!(usage.meters[0].used_percent, None);
    assert_eq!(
        usage.meters[0].state,
        ProviderAccountUsageMeterState::Unknown
    );
    assert_eq!(usage.meters[0].observed_at_ms, observed_at);
    assert_eq!(usage.meters[0].source, "event");
}

#[test]
fn a_window_past_its_reset_reads_as_not_measured_yet_until_a_new_reading() {
    crate::test_support::isolated_env_test!();
    let f = Fixture::new(vec![Ok(63.0), Ok(2.0)]);
    f.refresher.refresh_account(&f.key, f.at(0.0));
    let current = f.registry.get("local", "codex", &f.key.profile_id).unwrap();
    assert_eq!(current.usage.meters[0].used_percent, Some(63.0));

    // The last reading is six hours old: its 5-hour window reset an hour ago.
    let six_hours = 6 * 60 * MINUTE_MS;
    f.registry
        .update_usage(
            "local",
            "codex",
            &f.key.profile_id,
            snapshot(63.0, f.t0 - six_hours),
        )
        .unwrap();

    let expired = f.registry.get("local", "codex", &f.key.profile_id).unwrap();
    let meter = &expired.usage.meters[0];
    assert_eq!(
        meter.used_percent, None,
        "a stale percent must not be shown"
    );
    assert_eq!(meter.state, ProviderAccountUsageMeterState::Unknown);
    assert_eq!(meter.observed_at_ms, f.t0 - six_hours);
    let listed = f.registry.list("local", Some("codex")).unwrap();
    assert_eq!(listed[0].usage.meters[0].used_percent, None);
    let accounts = f.registry.usage_accounts("local", "machine").unwrap();
    assert_eq!(accounts[0].usage.meters[0].used_percent, None);
    assert_eq!(f.refresher.due_accounts(f.at(0.0)), vec![f.key.clone()]);

    f.refresher.refresh_account(&f.key, f.at(0.0));
    let renewed = f.registry.get("local", "codex", &f.key.profile_id).unwrap();
    assert_eq!(renewed.usage.meters[0].used_percent, Some(2.0));
}
