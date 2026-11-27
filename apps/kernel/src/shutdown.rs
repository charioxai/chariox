//! The process boundary is the last shutdown deadline: cancelling an async
//! future cannot stop a blocking syscall or a subsystem's synchronous Drop.
use std::cell::Cell;
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, OnceLock};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

/// A stop (or restart) abandons what has not finished after this long.
pub(crate) const SHUTDOWN_BUDGET: Duration = Duration::from_secs(10);
static WATCHDOG: OnceLock<Arc<Shared>> = OnceLock::new();
static STOPPING: AtomicBool = AtomicBool::new(false);

thread_local! {
    static STOP_CANCELS: Cell<bool> = const { Cell::new(false) };
    #[cfg(test)]
    static STOPPING_HERE: Cell<bool> = const { Cell::new(false) };
}

/// Held by a blocking job that must end when the kernel stops, not outlive it.
pub(crate) struct StopCancels(());

/// Work on this thread that polls [`cancelled`] ends once shutdown begins.
/// Background discovery opts in; a state-changing job such as a Git push does not.
pub(crate) fn stop_cancels_this_thread() -> StopCancels {
    STOP_CANCELS.with(|flag| flag.set(true));
    StopCancels(())
}

impl Drop for StopCancels {
    fn drop(&mut self) {
        STOP_CANCELS.with(|flag| flag.set(false));
    }
}

/// True for a job holding [`StopCancels`] once the kernel has begun to stop.
pub(crate) fn cancelled() -> bool {
    STOP_CANCELS.with(Cell::get) && (STOPPING.load(Ordering::Acquire) || stopping_here())
}

#[cfg(test)]
fn stopping_here() -> bool {
    STOPPING_HERE.with(Cell::get)
}

#[cfg(not(test))]
fn stopping_here() -> bool {
    false
}

/// Shutdown begins on this thread only, so tests of other jobs are unaffected.
#[cfg(test)]
pub(crate) fn stop_begins_on_this_thread() {
    STOPPING_HERE.with(|flag| flag.set(true));
}

struct State {
    started: Option<Instant>,
    reload: bool,
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
                reload: false,
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
        STOPPING.store(true, Ordering::Release);
        let mut state = shared.state.lock().expect("shutdown state");
        state.started.get_or_insert_with(Instant::now);
        shared.changed.notify_one();
    }
}

/// The kernel stops for a planned hot reload, which has already passed its
/// safe point: running agent turns are being handed over, and abandoning them
/// would interrupt every agent. The stop budget does not apply; the reload's
/// caller owns its deadline. A stage that outlasts the stop budget is logged.
pub fn begin_hot_reload() {
    if let Some(shared) = WATCHDOG.get() {
        STOPPING.store(true, Ordering::Release);
        let mut state = shared.state.lock().expect("shutdown state");
        state.reload = true;
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
    let mut reload_slow_reported = false;
    loop {
        if state.finished {
            return;
        }
        if let Some(started) = state.started {
            let remaining = budget.saturating_sub(started.elapsed());
            if remaining.is_zero() && state.reload {
                // Never abandon the agents of a planned reload: say what it waits for.
                if !reload_slow_reported {
                    reload_slow_reported = true;
                    let (stage, tasks) = (state.stage, blocking_names(&state));
                    eprintln!("kernel hot reload is still handing over after {} ms at {stage}; outstanding blocking jobs: {tasks}; it keeps waiting", budget.as_millis());
                }
                state = shared.changed.wait(state).expect("shutdown wait");
                continue;
            }
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
    fn a_hot_reload_is_never_abandoned_by_the_stop_budget() {
        const CHILD: &str = "CHARIOX_RELOAD_WATCHDOG_TEST";
        if std::env::var_os(CHILD).is_some() {
            let watchdog = ShutdownWatchdog::start(Duration::from_millis(100)).unwrap();
            let mut state = watchdog.shared.state.lock().unwrap();
            state.started = Some(Instant::now());
            state.reload = true;
            state.stage = "provider sessions";
            watchdog.shared.changed.notify_one();
            drop(state);
            // Well past the stop budget: the process must still be here.
            std::thread::sleep(Duration::from_millis(800));
            eprintln!("still-running-after-stop-budget");
            watchdog.finish(false);
            return;
        }
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "shutdown::tests::a_hot_reload_is_never_abandoned_by_the_stop_budget",
                "--nocapture",
            ])
            .env(CHILD, "1")
            .output()
            .unwrap();
        let diagnostic = String::from_utf8(output.stderr).unwrap();
        assert!(output.status.success(), "{diagnostic}");
        assert!(
            diagnostic.contains("still-running-after-stop-budget"),
            "{diagnostic}"
        );
        assert!(
            diagnostic.contains("hot reload is still handing over"),
            "{diagnostic}"
        );
        assert!(diagnostic.contains("at provider sessions"), "{diagnostic}");
        assert!(!diagnostic.contains("abandoning"), "{diagnostic}");
        assert!(!diagnostic.contains("exiting 0"), "{diagnostic}");
    }

    #[test]
    fn completed_shutdown_disarms_the_deadline() {
        let watchdog = ShutdownWatchdog::start(Duration::from_millis(1)).unwrap();
        watchdog.finish(false);
    }
}
