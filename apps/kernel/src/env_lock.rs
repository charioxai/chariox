#[cfg(test)]
use std::collections::HashMap;
#[cfg(test)]
use std::ffi::OsString;
use std::sync::{Condvar, Mutex, OnceLock};
use std::thread::{self, ThreadId};

#[derive(Debug)]
struct EnvLockState {
    owner: Option<ThreadId>,
    depth: usize,
}

#[derive(Debug)]
struct EnvLockInner {
    state: Mutex<EnvLockState>,
    ready: Condvar,
}

#[derive(Debug)]
pub(crate) struct EnvGuard {
    inner: &'static EnvLockInner,
    #[cfg(test)]
    previous: HashMap<OsString, OsString>,
}

pub(crate) fn lock() -> EnvGuard {
    static LOCK: OnceLock<EnvLockInner> = OnceLock::new();
    let inner = LOCK.get_or_init(|| EnvLockInner {
        state: Mutex::new(EnvLockState {
            owner: None,
            depth: 0,
        }),
        ready: Condvar::new(),
    });
    let thread_id = thread::current().id();
    let mut state = inner
        .state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());

    loop {
        match state.owner {
            Some(owner) if owner == thread_id => {
                state.depth += 1;
                break;
            }
            Some(_) => {
                state = inner
                    .ready
                    .wait(state)
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
            }
            None => {
                state.owner = Some(thread_id);
                state.depth = 1;
                break;
            }
        }
    }

    EnvGuard {
        inner,
        #[cfg(test)]
        previous: std::env::vars_os().collect(),
    }
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        let thread_id = thread::current().id();
        let mut state = self
            .inner
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());

        if state.owner == Some(thread_id) {
            #[cfg(test)]
            {
                for (name, _) in std::env::vars_os() {
                    if !self.previous.contains_key(&name) {
                        std::env::remove_var(name);
                    }
                }
                for (name, value) in &self.previous {
                    std::env::set_var(name, value);
                }
            }
            state.depth = state.depth.saturating_sub(1);
            if state.depth == 0 {
                state.owner = None;
                self.inner.ready.notify_all();
            }
        }
    }
}

#[test]
fn restores_environment_after_nested_scope_and_panic() {
    let _outer = lock();
    let name = "CHARIOX_TEST_ENV_GUARD_RESTORE";
    std::env::remove_var(name);
    {
        let _inner = lock();
        std::env::set_var(name, "inner");
    }
    assert!(std::env::var_os(name).is_none());
    assert!(std::panic::catch_unwind(|| {
        let _inner = lock();
        std::env::set_var(name, "panic");
        panic!("synthetic MP-08 fixture failure");
    })
    .is_err());
    assert!(std::env::var_os(name).is_none());
}
