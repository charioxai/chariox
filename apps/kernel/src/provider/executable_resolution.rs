use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{Duration, Instant};

const TRANSIENT_RETRY_WINDOW: Duration = Duration::from_secs(5);
const TRANSIENT_RETRY_INTERVAL: Duration = Duration::from_millis(100);

pub(super) struct ExecutableResolutionState {
    adapter_key: &'static str,
    resolved_once: AtomicBool,
}

impl ExecutableResolutionState {
    pub(super) const fn new(adapter_key: &'static str) -> Self {
        Self {
            adapter_key,
            resolved_once: AtomicBool::new(false),
        }
    }

    pub(super) fn resolve(&self, mut resolver: impl FnMut() -> Option<PathBuf>) -> Option<PathBuf> {
        if let Some(path) = resolver() {
            self.resolved_once.store(true, Ordering::Relaxed);
            return Some(path);
        }
        if !self.resolved_once.load(Ordering::Relaxed) {
            return None;
        }

        let started = Instant::now();
        crate::logging::info_with_fields(
            "daemon.provider",
            "provider executable temporarily unavailable; retrying",
            serde_json::json!({ "adapter_key": self.adapter_key }),
        );
        while started.elapsed() < TRANSIENT_RETRY_WINDOW {
            thread::sleep(TRANSIENT_RETRY_INTERVAL);
            if let Some(path) = resolver() {
                self.resolved_once.store(true, Ordering::Relaxed);
                crate::logging::info_with_fields(
                    "daemon.provider",
                    "provider executable recovered after transient replacement",
                    serde_json::json!({
                        "adapter_key": self.adapter_key,
                        "elapsed_ms": started.elapsed().as_millis(),
                    }),
                );
                return Some(path);
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::path::PathBuf;

    use super::ExecutableResolutionState;

    #[test]
    fn missing_executable_fails_immediately_before_any_successful_resolution() {
        let state = ExecutableResolutionState::new("test");
        let calls = Cell::new(0);

        let resolved = state.resolve(|| {
            calls.set(calls.get() + 1);
            None
        });

        assert_eq!(resolved, None);
        assert_eq!(calls.get(), 1);
    }

    #[test]
    fn retries_a_transient_gap_after_a_successful_resolution() {
        let state = ExecutableResolutionState::new("test");
        let expected = PathBuf::from("/tmp/provider");
        assert_eq!(
            state.resolve(|| Some(expected.clone())),
            Some(expected.clone())
        );

        let calls = Cell::new(0);
        let resolved = state.resolve(|| {
            calls.set(calls.get() + 1);
            (calls.get() >= 3).then(|| expected.clone())
        });

        assert_eq!(resolved, Some(expected));
        assert_eq!(calls.get(), 3);
    }
}
