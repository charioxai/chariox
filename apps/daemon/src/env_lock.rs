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

    EnvGuard { inner }
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
            state.depth = state.depth.saturating_sub(1);
            if state.depth == 0 {
                state.owner = None;
                self.inner.ready.notify_all();
            }
        }
    }
}
