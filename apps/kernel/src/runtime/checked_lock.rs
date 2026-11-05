//! Shared state locks identify their caller while waiting and while held.
use std::ops::{Deref, DerefMut};
use std::sync::{LockResult, Mutex, MutexGuard, PoisonError, TryLockError, TryLockResult};
#[cfg(test)]
use std::time::Duration;
use std::time::Instant;

use super::stall_watchdog::{self, Watch};

#[cfg(test)]
pub(crate) const LOCK_ORDER: &[(&str, u16)] = &[
    ("waiting_room_launch_target", 5),
    ("provider_managed_admission", 10),
    ("provider_move_admission", 20),
    ("provider_service", 30),
    ("process_rows_snapshot", 40),
    ("provider_operation_lanes", 50),
];

#[cfg(test)]
thread_local! { static HELD: std::cell::RefCell<Vec<(usize, u16)>> = const { std::cell::RefCell::new(Vec::new()) }; }

pub(crate) struct CheckedMutex<T> {
    inner: Mutex<T>,
    name: &'static str,
}

pub struct CheckedMutexGuard<'a, T> {
    guard: MutexGuard<'a, T>,
    _watch: Watch,
    acquired: Instant,
    name: &'static str,
    #[cfg(test)]
    identity: usize,
}

impl<T: std::fmt::Debug> std::fmt::Debug for CheckedMutex<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CheckedMutex")
            .field("name", &self.name)
            .finish_non_exhaustive()
    }
}

impl<T> CheckedMutex<T> {
    pub(crate) const fn new(name: &'static str, value: T) -> Self {
        Self {
            inner: Mutex::new(value),
            name,
        }
    }

    #[track_caller]
    pub(crate) fn lock(&self) -> LockResult<CheckedMutexGuard<'_, T>> {
        self.check_order();
        let location = std::panic::Location::caller();
        let waiting = stall_watchdog::watch_at(self.name, "lock_wait", location);
        #[cfg(not(test))]
        let result = self.inner.lock();
        #[cfg(test)]
        let result = {
            let deadline = Instant::now() + Duration::from_secs(30);
            loop {
                match self.inner.try_lock() {
                    Ok(guard) => break Ok(guard),
                    Err(std::sync::TryLockError::Poisoned(error)) => break Err(error),
                    Err(std::sync::TryLockError::WouldBlock) => {
                        assert!(
                            Instant::now() < deadline,
                            "{} lock wait exceeded 30s at {}",
                            self.name,
                            std::panic::Location::caller()
                        );
                        std::thread::sleep(Duration::from_millis(1));
                    }
                }
            }
        };
        drop(waiting);
        let guard = |guard| CheckedMutexGuard {
            guard,
            _watch: stall_watchdog::watch_at(self.name, "lock_hold", location),
            acquired: Instant::now(),
            name: self.name,
            #[cfg(test)]
            identity: self.acquired(),
        };
        match result {
            Ok(value) => Ok(guard(value)),
            Err(error) => Err(PoisonError::new(guard(error.into_inner()))),
        }
    }

    #[track_caller]
    fn check_order(&self) {
        #[cfg(test)]
        HELD.with(|held| {
            let rank = LOCK_ORDER
                .iter()
                .find(|(name, _)| *name == self.name)
                .map_or(0, |(_, rank)| *rank);
            let identity = self as *const Self as usize;
            assert!(
                held.borrow()
                    .iter()
                    .all(|(id, previous)| *id != identity && *previous <= rank),
                "lock order inversion for {} at {}",
                self.name,
                std::panic::Location::caller()
            );
        });
    }

    #[cfg(test)]
    fn acquired(&self) -> usize {
        let identity = self as *const Self as usize;
        let rank = LOCK_ORDER
            .iter()
            .find(|(name, _)| *name == self.name)
            .map_or(0, |(_, rank)| *rank);
        HELD.with(|held| held.borrow_mut().push((identity, rank)));
        identity
    }

    #[track_caller]
    pub(crate) fn try_lock(&self) -> TryLockResult<CheckedMutexGuard<'_, T>> {
        self.check_order();
        let location = std::panic::Location::caller();
        let guard = |guard| CheckedMutexGuard {
            guard,
            _watch: stall_watchdog::watch_at(self.name, "lock_hold", location),
            acquired: Instant::now(),
            name: self.name,
            #[cfg(test)]
            identity: self.acquired(),
        };
        match self.inner.try_lock() {
            Ok(value) => Ok(guard(value)),
            Err(TryLockError::WouldBlock) => Err(TryLockError::WouldBlock),
            Err(TryLockError::Poisoned(error)) => Err(TryLockError::Poisoned(PoisonError::new(
                guard(error.into_inner()),
            ))),
        }
    }
}

impl<T> Deref for CheckedMutexGuard<'_, T> {
    type Target = T;
    fn deref(&self) -> &T {
        &self.guard
    }
}

impl<T> DerefMut for CheckedMutexGuard<'_, T> {
    fn deref_mut(&mut self) -> &mut T {
        &mut self.guard
    }
}

impl<T> Drop for CheckedMutexGuard<'_, T> {
    fn drop(&mut self) {
        #[cfg(test)]
        HELD.with(|held| held.borrow_mut().retain(|(id, _)| *id != self.identity));
        #[cfg(test)]
        if !std::thread::panicking() {
            assert!(
                self.acquired.elapsed() < Duration::from_secs(30),
                "{} lock held longer than 30s",
                self.name
            );
        }
        #[cfg(not(test))]
        let _ = (&self.acquired, self.name);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inventory_lock_order_rejects_reverse_edges() {
        let store = CheckedMutex::new("provider_service", ());
        let earlier = CheckedMutex::new("provider_move_admission", ());
        let _store = store.lock().unwrap();
        let inversion = std::panic::catch_unwind(|| {
            let _guard = earlier.lock().unwrap();
        });
        assert!(inversion.is_err());
    }

    #[test]
    fn watchdog_logs_the_holder_call_site_before_release() {
        let capture = crate::logging::capture::start();
        let lock = CheckedMutex::new("watchdog_lock_test", ());
        let guard = lock.lock().unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        let notice = loop {
            if let Some(record) = capture.records().into_iter().find(|record| {
                record.contains("watchdog_lock_test") && record.contains("lock_hold")
            }) {
                break record;
            }
            assert!(
                Instant::now() < deadline,
                "watchdog did not log a held lock"
            );
            std::thread::sleep(Duration::from_millis(20));
        };
        assert!(notice.contains("checked_lock.rs"));
        assert!(notice.contains("call_site"));
        assert!(notice.contains("held_locks"));
        drop(guard);
        assert_eq!(
            capture
                .records()
                .iter()
                .filter(
                    |record| record.contains("watchdog_lock_test") && record.contains("lock_hold")
                )
                .count(),
            1
        );
    }

    #[test]
    fn excessive_hold_panics_in_tests() {
        let lock = CheckedMutex::new("checked_lock_test", 0);
        let result = std::panic::catch_unwind(|| {
            let mut guard = lock.lock().unwrap();
            guard.acquired = Instant::now() - Duration::from_secs(31);
        });
        assert!(result.is_err());
    }
}
