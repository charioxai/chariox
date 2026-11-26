//! Instrumented acquisition of the shared `DaemonApp` mutex.
//!
//! Every production path that still locks the app directly goes through
//! [`lock_app_instrumented`], which records wait and hold durations per call
//! site. The aggregate surfaces in daemon health so lock contention and
//! long holds are observable while the ownership migration removes them.
//! The architecture boundaries test polices call sites for this helper the
//! same way it polices raw `app.lock().await`.

use std::collections::BTreeMap;
use std::ops::{Deref, DerefMut};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex as StdMutex, OnceLock, PoisonError};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use tokio::sync::{Mutex, MutexGuard};

use crate::app::DaemonApp;

const SLOW_HOLD_THRESHOLD_MS: u64 = 100;
const STALL_HOLD_THRESHOLD_MS: u64 = 500;

pub(crate) async fn lock_app_instrumented<'a>(
    app: &'a Mutex<DaemonApp>,
    site: &'static str,
) -> InstrumentedAppGuard<'a> {
    let waiting_since = Instant::now();
    let guard = app.lock().await;
    InstrumentedAppGuard {
        guard,
        site,
        wait: waiting_since.elapsed(),
        acquired_at: Instant::now(),
    }
}

pub(crate) struct InstrumentedAppGuard<'a> {
    guard: MutexGuard<'a, DaemonApp>,
    site: &'static str,
    wait: Duration,
    acquired_at: Instant,
}

impl Deref for InstrumentedAppGuard<'_> {
    type Target = DaemonApp;

    fn deref(&self) -> &Self::Target {
        &self.guard
    }
}

impl DerefMut for InstrumentedAppGuard<'_> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.guard
    }
}

impl Drop for InstrumentedAppGuard<'_> {
    fn drop(&mut self) {
        record(self.site, self.wait, self.acquired_at.elapsed());
    }
}

/// Who holds the app lock now, kept on every build (not only with performance diagnostics): the
/// kernel's own watchdog reads it from its thread when the runtime itself no longer advances.
struct Holder {
    id: u64,
    operation: &'static str,
    thread: String,
    since: Instant,
}

/// One slot per app lock: a new holder replaces the mark, so only one continuous hold has an age.
pub(crate) struct HolderSlot {
    holder: StdMutex<Option<Holder>>,
    next: AtomicU64,
}

static HOLDER: HolderSlot = HolderSlot::new();

/// The app lock's current holder, as seen by a watchdog.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AppLockHolder {
    /// The function that runs under the lock (its closure's type name).
    pub(crate) operation: &'static str,
    pub(crate) thread: String,
    pub(crate) held: Duration,
}

/// Marks the app lock as held by its operation until dropped. Take it right after the lock and
/// drop it before the guard.
pub(crate) struct HolderMark {
    slot: &'static HolderSlot,
    id: u64,
}

impl HolderSlot {
    const fn new() -> Self {
        Self {
            holder: StdMutex::new(None),
            next: AtomicU64::new(1),
        }
    }

    fn mark(&'static self, operation: &'static str) -> HolderMark {
        let id = self.next.fetch_add(1, Ordering::Relaxed);
        *self.holder.lock().unwrap_or_else(PoisonError::into_inner) = Some(Holder {
            id,
            operation,
            thread: std::thread::current()
                .name()
                .unwrap_or("unnamed")
                .to_string(),
            since: Instant::now(),
        });
        HolderMark { slot: self, id }
    }

    fn current(&self) -> Option<AppLockHolder> {
        self.holder
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .as_ref()
            .map(|holder| AppLockHolder {
                operation: holder.operation,
                thread: holder.thread.clone(),
                held: holder.since.elapsed(),
            })
    }
}

impl Drop for HolderMark {
    fn drop(&mut self) {
        let mut holder = self
            .slot
            .holder
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if holder.as_ref().is_some_and(|holder| holder.id == self.id) {
            *holder = None;
        }
    }
}

pub(crate) fn mark_holder(operation: &'static str) -> HolderMark {
    HOLDER.mark(operation)
}

/// The type name of `operation`: for a closure it names the function that wrote it.
pub(crate) fn operation_name<T>(_: &T) -> &'static str {
    std::any::type_name::<T>()
}

pub(crate) fn current_holder() -> Option<AppLockHolder> {
    HOLDER.current()
}

#[derive(Debug, Default, Clone, Copy)]
struct SiteStats {
    acquisitions: u64,
    total_wait_us: u64,
    max_wait_us: u64,
    total_hold_us: u64,
    max_hold_us: u64,
    holds_over_100ms: u64,
    holds_over_500ms: u64,
}

fn registry() -> &'static StdMutex<BTreeMap<&'static str, SiteStats>> {
    static REGISTRY: OnceLock<StdMutex<BTreeMap<&'static str, SiteStats>>> = OnceLock::new();
    REGISTRY.get_or_init(|| StdMutex::new(BTreeMap::new()))
}

fn record(site: &'static str, wait: Duration, hold: Duration) {
    if !performance_diagnostics_enabled() {
        return;
    }
    let wait_us = u64::try_from(wait.as_micros()).unwrap_or(u64::MAX);
    let hold_us = u64::try_from(hold.as_micros()).unwrap_or(u64::MAX);
    let mut sites = registry().lock().unwrap_or_else(PoisonError::into_inner);
    let stats = sites.entry(site).or_default();
    stats.acquisitions = stats.acquisitions.saturating_add(1);
    stats.total_wait_us = stats.total_wait_us.saturating_add(wait_us);
    stats.max_wait_us = stats.max_wait_us.max(wait_us);
    stats.total_hold_us = stats.total_hold_us.saturating_add(hold_us);
    stats.max_hold_us = stats.max_hold_us.max(hold_us);
    if hold_us >= SLOW_HOLD_THRESHOLD_MS * 1_000 {
        stats.holds_over_100ms = stats.holds_over_100ms.saturating_add(1);
    }
    if hold_us >= STALL_HOLD_THRESHOLD_MS * 1_000 {
        stats.holds_over_500ms = stats.holds_over_500ms.saturating_add(1);
    }
}

fn performance_diagnostics_enabled() -> bool {
    if cfg!(test) {
        return true;
    }
    static ENABLED: OnceLock<bool> = OnceLock::new();
    *ENABLED.get_or_init(|| {
        std::env::var("CHARIOX_PERF_DIAGNOSTICS")
            .ok()
            .is_some_and(|value| matches!(value.trim(), "1" | "true" | "yes" | "on"))
    })
}

#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppLockSiteHealthSnapshot {
    pub site: String,
    pub acquisitions: u64,
    pub total_wait_us: u64,
    pub max_wait_us: u64,
    pub total_hold_us: u64,
    pub max_hold_us: u64,
    pub holds_over_100ms: u64,
    pub holds_over_500ms: u64,
}

#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppLockHealthSnapshot {
    pub sites: Vec<AppLockSiteHealthSnapshot>,
}

pub(crate) fn app_lock_health_snapshot() -> AppLockHealthSnapshot {
    let sites = registry().lock().unwrap_or_else(PoisonError::into_inner);
    let mut sites = sites
        .iter()
        .map(|(site, stats)| AppLockSiteHealthSnapshot {
            site: (*site).to_string(),
            acquisitions: stats.acquisitions,
            total_wait_us: stats.total_wait_us,
            max_wait_us: stats.max_wait_us,
            total_hold_us: stats.total_hold_us,
            max_hold_us: stats.max_hold_us,
            holds_over_100ms: stats.holds_over_100ms,
            holds_over_500ms: stats.holds_over_500ms,
        })
        .collect::<Vec<_>>();
    sites.sort_by_key(|site| std::cmp::Reverse(site.total_hold_us));
    AppLockHealthSnapshot { sites }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[tokio::test]
    async fn instrumented_lock_records_wait_and_hold_per_site() {
        let app = Arc::new(Mutex::new(
            DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests())
                .expect("test daemon app should bootstrap"),
        ));

        {
            let guard = lock_app_instrumented(&app, "app_lock_test_site").await;
            let _ = &*guard;
            tokio::time::sleep(Duration::from_millis(2)).await;
        }

        let snapshot = app_lock_health_snapshot();
        let site = snapshot
            .sites
            .iter()
            .find(|site| site.site == "app_lock_test_site")
            .expect("instrumented site should be recorded");
        assert!(site.acquisitions >= 1);
        assert!(site.max_hold_us >= 1_000, "hold should include sleep time");
    }

    /// Row 678: a watchdog sees one continuous hold; each new holder starts a new age, so many
    /// short holders never add up to a long one.
    #[test]
    fn the_holder_age_restarts_with_every_new_holder() {
        let slot: &'static HolderSlot = Box::leak(Box::new(HolderSlot::new()));
        let first = slot.mark("first-holder");
        std::thread::sleep(Duration::from_millis(30));
        let held = slot.current().expect("a holder is marked");
        assert_eq!(held.operation, "first-holder");
        assert!(held.held >= Duration::from_millis(30));
        drop(first);
        assert_eq!(slot.current(), None);
        let _second = slot.mark("second-holder");
        let held = slot.current().expect("a holder is marked");
        assert_eq!(held.operation, "second-holder");
        assert!(held.held < Duration::from_millis(30));
    }
}
