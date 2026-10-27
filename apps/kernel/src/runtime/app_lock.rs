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
    sites.sort_by(|a, b| b.total_hold_us.cmp(&a.total_hold_us));
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
}
