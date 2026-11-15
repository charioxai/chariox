//! The process boundary is the last shutdown deadline: cancelling an async
//! future cannot stop a blocking syscall or a subsystem's synchronous Drop.
use std::collections::BTreeMap;
use std::sync::{Arc, Condvar, Mutex, OnceLock};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

pub(crate) const SHUTDOWN_BUDGET: Duration = Duration::from_secs(10);
static WATCHDOG: OnceLock<Arc<Shared>> = OnceLock::new();

struct State {
    started: Option<Instant>,
    stage: &'static str,
    finished: bool,
    blocking: BTreeMap<&'static str, usize>,
}

struct Shared {
    state: Mutex<State>,
    changed: Condvar,
}

/// Installed by the executable, rather than library users with their own runtime.
pub struct ShutdownWatchdog {
    shared: Arc<Shared>,
    thread: JoinHandle<()>,
}

impl ShutdownWatchdog {
    pub fn install() -> std::io::Result<Self> {
        let watchdog = Self::start(SHUTDOWN_BUDGET)?;
        assert!(WATCHDOG.set(watchdog.shared.clone()).is_ok());
        Ok(watchdog)
    }

    fn start(budget: Duration) -> std::io::Result<Self> {
        let shared = Arc::new(Shared {
            state: Mutex::new(State {
                started: None,
                stage: "outside event waits",
                finished: false,
                blocking: BTreeMap::new(),
            }),
            changed: Condvar::new(),
        });
        let watching = shared.clone();
        let thread = std::thread::Builder::new()
            .name("kernel-shutdown".into())
            .spawn(move || watch(watching, budget))?;
        Ok(Self { shared, thread })
    }

    /// Call only after runtime teardown and the final log flush.
    pub fn finish(self, runtime_abandoned: bool) {
        let message = {
            let state = self.shared.state.lock().expect("shutdown state");
            state.started.map(|started| {
                let abandoned = if runtime_abandoned {
                    blocking_names(&state)
                } else {
                    "none".into()
                };
                format!(
                    "kernel shutdown complete in {} ms; abandoned: {abandoned}",
                    started.elapsed().as_millis()
                )
            })
        };
        if let Some(message) = message {
            eprintln!("{message}");
        }
        let mut state = self.shared.state.lock().expect("shutdown state");
        state.finished = true;
        self.shared.changed.notify_one();
        drop(state);
        let _ = self.thread.join();
    }
}

pub fn begin() {
    if let Some(shared) = WATCHDOG.get() {
        let mut state = shared.state.lock().expect("shutdown state");
        state.started.get_or_insert_with(Instant::now);
        shared.changed.notify_one();
    }
}

/// Fixed subsystem labels only: shutdown diagnostics never contain user data.
pub fn stage(name: &'static str) {
    if let Some(shared) = WATCHDOG.get() {
        shared.state.lock().expect("shutdown state").stage = name;
    }
}

struct BlockingJob {
    shared: Arc<Shared>,
    name: &'static str,
}

impl Drop for BlockingJob {
    fn drop(&mut self) {
        let mut state = self.shared.state.lock().expect("shutdown state");
        if let Some(count) = state.blocking.get_mut(self.name) {
            *count -= 1;
            if *count == 0 {
                state.blocking.remove(self.name);
            }
        }
    }
}

/// Retain the subsystem label until blocking work really stops, including
/// after its async caller has been cancelled during shutdown.
pub(crate) fn spawn_blocking<T: Send + 'static>(
    name: &'static str,
    work: impl FnOnce() -> T + Send + 'static,
) -> tokio::task::JoinHandle<T> {
    let job = WATCHDOG.get().map(|shared| {
        *shared
            .state
            .lock()
            .expect("shutdown state")
            .blocking
            .entry(name)
            .or_default() += 1;
        BlockingJob {
            shared: shared.clone(),
            name,
        }
    });
    tokio::task::spawn_blocking(move || {
        let _job = job;
        work()
    })
}

fn blocking_names(state: &State) -> String {
    if state.blocking.is_empty() {
        "untracked runtime work".into()
    } else {
        state
            .blocking
            .keys()
            .copied()
            .collect::<Vec<_>>()
            .join(", ")
    }
}

fn watch(shared: Arc<Shared>, budget: Duration) {
    let mut state = shared.state.lock().expect("shutdown state");
    loop {
        if state.finished {
            return;
        }
        if let Some(started) = state.started {
            let remaining = budget.saturating_sub(started.elapsed());
            if remaining.is_zero() {
                let stage = state.stage;
                let tasks = blocking_names(&state);
                drop(state);
                // The regular logger may itself be blocked on its sink. Keep
                // the final diagnostic independent of it and of the runtime.
                eprintln!("kernel shutdown budget exhausted after {} ms; abandoning {stage} and remaining shutdown work; outstanding blocking jobs: {tasks}; exiting 0", budget.as_millis());
                std::process::exit(0);
            }
            (state, _) = shared
                .changed
                .wait_timeout(state, remaining)
                .expect("shutdown wait");
        } else {
            state = shared.changed.wait(state).expect("shutdown wait");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shutdown_deadline_exits_even_when_the_main_thread_is_blocked() {
        const CHILD: &str = "CHARIOX_SHUTDOWN_WATCHDOG_TEST";
        if std::env::var_os(CHILD).is_some() {
            let watchdog = ShutdownWatchdog::start(Duration::from_millis(100)).unwrap();
            let mut state = watchdog.shared.state.lock().unwrap();
            state.started = Some(Instant::now());
            state.stage = "Files blocking writer";
            watchdog.shared.changed.notify_one();
            drop(state);
            std::thread::sleep(Duration::from_secs(30));
            panic!("the watchdog did not exit");
        }
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "shutdown::tests::shutdown_deadline_exits_even_when_the_main_thread_is_blocked",
                "--nocapture",
            ])
            .env(CHILD, "1")
            .output()
            .unwrap();
        assert!(output.status.success(), "{output:?}");
        let diagnostic = String::from_utf8(output.stderr).unwrap();
        assert!(
            diagnostic.contains("abandoning Files blocking writer"),
            "{diagnostic}"
        );
        assert!(diagnostic.contains("exiting 0"), "{diagnostic}");
    }

    #[test]
    fn completed_shutdown_disarms_the_deadline() {
        let watchdog = ShutdownWatchdog::start(Duration::from_millis(1)).unwrap();
        watchdog.finish(false);
    }
}
