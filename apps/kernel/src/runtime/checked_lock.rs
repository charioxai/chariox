//! Shared state locks identify their caller while waiting and while held.
use std::ops::{Deref, DerefMut};
use std::sync::{
    LockResult, Mutex, MutexGuard, PoisonError, RwLock, RwLockReadGuard, RwLockWriteGuard,
    TryLockError, TryLockResult,
};
#[cfg(test)]
use std::time::Duration;
use std::time::Instant;

use super::stall_watchdog::{self, Watch};

#[cfg(test)]
pub(crate) const LOCK_ORDER: &[(&str, u16)] = &[
    // The kernel's documented order: activity -> session -> prompt owner -> agent.
    ("session_state", 2),
    ("prompt_state_owner", 3),
    ("room_list_launch_target", 5),
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

/// A guard of a plain std lock taken through [`lock_checked`], [`read_checked`] or
/// [`write_checked`]: the stall watchdog sees its hold as it sees a [`CheckedMutex`] hold,
/// and tests check it against [`LOCK_ORDER`].
pub struct CheckedStdGuard<G> {
    guard: G,
    _watch: Watch,
    #[cfg(test)]
    identity: usize,
}

impl<G: Deref> Deref for CheckedStdGuard<G> {
    type Target = G::Target;
    fn deref(&self) -> &G::Target {
        &self.guard
    }
}

impl<G: DerefMut> DerefMut for CheckedStdGuard<G> {
    fn deref_mut(&mut self) -> &mut G::Target {
        &mut self.guard
    }
}

impl<G> Drop for CheckedStdGuard<G> {
    fn drop(&mut self) {
        #[cfg(test)]
        HELD.with(|held| {
            let mut held = held.borrow_mut();
            if let Some(index) = held.iter().rposition(|(id, _)| *id == self.identity) {
                held.remove(index);
            }
        });
    }
}

/// Runs the same order check as [`CheckedMutex::lock`] for a plain std lock at `identity`.
#[track_caller]
fn check_std_order(name: &'static str, identity: usize) {
    #[cfg(test)]
    HELD.with(|held| {
        let rank = LOCK_ORDER
            .iter()
            .find(|(lock, _)| *lock == name)
            .map_or(0, |(_, rank)| *rank);
        assert!(
            held.borrow()
                .iter()
                .all(|(id, previous)| *id != identity && *previous <= rank),
            "lock order inversion for {} at {}",
            name,
            std::panic::Location::caller()
        );
    });
    #[cfg(not(test))]
    let _ = (name, identity);
}

fn checked_std_result<G>(
    name: &'static str,
    identity: usize,
    location: &'static std::panic::Location<'static>,
    result: LockResult<G>,
) -> LockResult<CheckedStdGuard<G>> {
    let wrap = |guard| {
        #[cfg(test)]
        {
            let rank = LOCK_ORDER
                .iter()
                .find(|(lock, _)| *lock == name)
                .map_or(0, |(_, rank)| *rank);
            HELD.with(|held| held.borrow_mut().push((identity, rank)));
        }
        CheckedStdGuard {
            guard,
            _watch: stall_watchdog::watch_at(name, "lock_hold", location),
            #[cfg(test)]
            identity,
        }
    };
    #[cfg(not(test))]
    let _ = identity;
    match result {
        Ok(guard) => Ok(wrap(guard)),
        Err(error) => Err(PoisonError::new(wrap(error.into_inner()))),
    }
}

/// Locks a plain std mutex as a [`CheckedMutex`] would: an uncontended lock costs one
/// `try_lock`; a contended wait and every hold are named, with their call site, at the
/// stall watchdog's threshold.
#[track_caller]
pub(crate) fn lock_checked<'a, T>(
    name: &'static str,
    mutex: &'a Mutex<T>,
) -> LockResult<CheckedStdGuard<MutexGuard<'a, T>>> {
    let identity = mutex as *const Mutex<T> as usize;
    let location = std::panic::Location::caller();
    check_std_order(name, identity);
    let result = match mutex.try_lock() {
        Ok(guard) => Ok(guard),
        Err(TryLockError::Poisoned(error)) => Err(error),
        Err(TryLockError::WouldBlock) => {
            let _waiting = stall_watchdog::watch_at(name, "lock_wait", location);
            mutex.lock()
        }
    };
    checked_std_result(name, identity, location, result)
}

/// [`lock_checked`] for the read side of a std `RwLock`. A second read on the same thread
/// is an inversion too: a waiting writer blocks it.
#[track_caller]
pub(crate) fn read_checked<'a, T>(
    name: &'static str,
    lock: &'a RwLock<T>,
) -> LockResult<CheckedStdGuard<RwLockReadGuard<'a, T>>> {
    let identity = lock as *const RwLock<T> as usize;
    let location = std::panic::Location::caller();
    check_std_order(name, identity);
    let result = match lock.try_read() {
        Ok(guard) => Ok(guard),
        Err(TryLockError::Poisoned(error)) => Err(error),
        Err(TryLockError::WouldBlock) => {
            let _waiting = stall_watchdog::watch_at(name, "lock_wait", location);
            lock.read()
        }
    };
    checked_std_result(name, identity, location, result)
}

/// [`lock_checked`] for the write side of a std `RwLock`.
#[track_caller]
pub(crate) fn write_checked<'a, T>(
    name: &'static str,
    lock: &'a RwLock<T>,
) -> LockResult<CheckedStdGuard<RwLockWriteGuard<'a, T>>> {
    let identity = lock as *const RwLock<T> as usize;
    let location = std::panic::Location::caller();
    check_std_order(name, identity);
    let result = match lock.try_write() {
        Ok(guard) => Ok(guard),
        Err(TryLockError::Poisoned(error)) => Err(error),
        Err(TryLockError::WouldBlock) => {
            let _waiting = stall_watchdog::watch_at(name, "lock_wait", location);
            lock.write()
        }
    };
    checked_std_result(name, identity, location, result)
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
    fn std_locks_follow_sessions_then_prompts() {
        let sessions = RwLock::new(());
        let prompts = Mutex::new(());
        let owner = lock_checked("prompt_state_owner", &prompts).unwrap();
        let inversion = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _sessions = write_checked("session_state", &sessions).unwrap();
        }));
        assert!(
            inversion.is_err(),
            "the prompt owner must not wait for the sessions"
        );
        drop(owner);
        let read = read_checked("session_state", &sessions).unwrap();
        let reentry = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _again = read_checked("session_state", &sessions).unwrap();
        }));
        assert!(
            reentry.is_err(),
            "a waiting writer blocks a second read on this thread"
        );
        let _owner = lock_checked("prompt_state_owner", &prompts).unwrap();
        drop(read);
    }

    #[test]
    fn a_contended_std_lock_wait_names_its_call_site() {
        let capture = crate::logging::capture::start();
        let prompts = std::sync::Arc::new(Mutex::new(()));
        let holder = lock_checked("std_wait_lock_test", &prompts).unwrap();
        let waiter = {
            let prompts = std::sync::Arc::clone(&prompts);
            std::thread::spawn(move || {
                let _guard = lock_checked("std_wait_lock_test", &prompts).unwrap();
            })
        };
        let deadline = Instant::now() + Duration::from_secs(10);
        let notice = loop {
            if let Some(record) = capture.records().into_iter().find(|record| {
                record.contains("std_wait_lock_test") && record.contains("lock_wait")
            }) {
                break record;
            }
            assert!(Instant::now() < deadline, "watchdog did not log the wait");
            std::thread::sleep(Duration::from_millis(20));
        };
        assert!(notice.contains("checked_lock.rs"));
        assert!(notice.contains("std_wait_lock_test"), "{notice}");
        drop(holder);
        waiter.join().unwrap();
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
