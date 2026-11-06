//! Kernel subprocess deadlines, output budgets, and checked shutdown.
use std::io;
use std::process::{Child, Command, ExitStatus, Output};
use std::time::{Duration, Instant};

pub(crate) mod capture;

fn capture_error(error: capture::CaptureError) -> io::Error {
    match error {
        capture::CaptureError::Io(error) => error,
        capture::CaptureError::TimedOut => io::Error::new(
            io::ErrorKind::TimedOut,
            "subprocess output deadline exceeded; child shutdown requested",
        ),
        capture::CaptureError::OutputLimit => {
            io::Error::other("subprocess output exceeded the capture budget")
        }
    }
}

const COMMAND_LIMIT: Duration = Duration::from_secs(60);
/// Active lifetime of work that changes state or builds: Git commit and push
/// with their hooks, worktree checkout, pull request creation, cold builds.
const MUTATION_LIMIT: Duration = Duration::from_secs(30 * 60);

/// What a command is for, which decides how long it may stay active.
#[derive(Clone, Copy)]
pub(crate) enum Work {
    /// Probes and reads: status, rev-parse, diff, log, config.
    Query,
    /// Long-running by nature; ends at its own deadline, never the query cap.
    Mutation,
}

#[cfg(test)]
thread_local! {
    static TEST_COMMAND_LIMIT: std::cell::Cell<Option<Duration>> = const { std::cell::Cell::new(None) };
    static TEST_MUTATION_LIMIT: std::cell::Cell<Option<Duration>> = const { std::cell::Cell::new(None) };
}

fn command_limit() -> Duration {
    #[cfg(test)]
    if let Some(limit) = TEST_COMMAND_LIMIT.with(std::cell::Cell::get) {
        return limit;
    }
    COMMAND_LIMIT
}

fn work_limit(work: Work) -> Duration {
    match work {
        Work::Query => command_limit(),
        Work::Mutation => {
            #[cfg(test)]
            if let Some(limit) = TEST_MUTATION_LIMIT.with(std::cell::Cell::get) {
                return limit;
            }
            MUTATION_LIMIT
        }
    }
}

/// Shortens the mutation deadline on this thread so tests can show that a
/// hung mutation ends at its own deadline with its process tree reaped.
#[cfg(test)]
pub(crate) fn with_test_mutation_limit<T>(limit: Duration, run: impl FnOnce() -> T) -> T {
    struct Reset;
    impl Drop for Reset {
        fn drop(&mut self) {
            TEST_MUTATION_LIMIT.with(|cell| cell.set(None));
        }
    }
    TEST_MUTATION_LIMIT.with(|cell| cell.set(Some(limit)));
    let _reset = Reset;
    run()
}

/// Shortens the generic command cap on this thread so tests can show that
/// progress-based work does not inherit it.
#[cfg(test)]
pub(crate) fn with_test_command_limit<T>(limit: Duration, run: impl FnOnce() -> T) -> T {
    struct Reset;
    impl Drop for Reset {
        fn drop(&mut self) {
            TEST_COMMAND_LIMIT.with(|cell| cell.set(None));
        }
    }
    TEST_COMMAND_LIMIT.with(|cell| cell.set(Some(limit)));
    let _reset = Reset;
    run()
}
const REAP_LIMIT: Duration = Duration::from_secs(1);

pub(crate) trait BoundedCommandExt {
    #[track_caller]
    fn bounded_output(&mut self) -> io::Result<Output>;
    /// Output of a command whose lifetime depends on what it does.
    #[track_caller]
    fn output_for(&mut self, work: Work) -> io::Result<Output>;
    fn bounded_status(&mut self) -> io::Result<ExitStatus>;
    #[track_caller]
    fn output_with_deadline(&mut self, timeout: Duration) -> io::Result<Output>;
    /// Bulk work whose lifetime is its progress: the child lives while
    /// `position` keeps changing and is stopped, with its pipe owners, once it
    /// stays unchanged for `stall` or `position` fails.
    #[track_caller]
    fn output_with_progress(
        &mut self,
        stall: Duration,
        position: &mut dyn FnMut() -> io::Result<u64>,
    ) -> io::Result<Output>;
}

impl BoundedCommandExt for Command {
    fn bounded_status(&mut self) -> io::Result<ExitStatus> {
        let _watch = super::stall_watchdog::watch("subprocess", "status");
        let mut child = self.spawn()?;
        drop(child.stdin.take());
        let deadline = Instant::now() + COMMAND_LIMIT;
        loop {
            if let Some(status) = child.try_wait()? {
                return Ok(status);
            }
            if Instant::now() >= deadline {
                super::child_shutdown::stop(&mut child, "status_deadline")?;
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "subprocess status deadline exceeded",
                ));
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    #[track_caller]
    fn bounded_output(&mut self) -> io::Result<Output> {
        self.output_for(Work::Query)
    }
    #[track_caller]
    fn output_for(&mut self, work: Work) -> io::Result<Output> {
        self.output_with_deadline(work_limit(work))
    }
    #[track_caller]
    fn output_with_progress(
        &mut self,
        stall: Duration,
        position: &mut dyn FnMut() -> io::Result<u64>,
    ) -> io::Result<Output> {
        let _watch = super::stall_watchdog::progress_watch("subprocess", "output", stall);
        capture::capture_command_with_progress(self, stall, position).map_err(capture_error)
    }
    #[track_caller]
    fn output_with_deadline(&mut self, timeout: Duration) -> io::Result<Output> {
        let _watch = super::stall_watchdog::watch("subprocess", "output");
        capture::capture_command(self, timeout).map_err(capture_error)
    }
}

pub(crate) trait BoundedChildExt {
    type Status;
    fn bounded_wait(&mut self) -> io::Result<Self::Status>;
    fn output_with_deadline(self, _timeout: Duration) -> io::Result<Output>
    where
        Self: Sized,
    {
        Err(io::Error::other("output capture requires a native child"))
    }
}

impl BoundedChildExt for Child {
    type Status = ExitStatus;
    fn output_with_deadline(self, timeout: Duration) -> io::Result<Output> {
        capture::capture_child(self, timeout).map_err(capture_error)
    }

    fn bounded_wait(&mut self) -> io::Result<ExitStatus> {
        drop(self.stdin.take());
        let _watch = super::stall_watchdog::watch("subprocess", "reap");
        let deadline = Instant::now() + REAP_LIMIT;
        loop {
            if let Some(status) = self.try_wait()? {
                return Ok(status);
            }
            if Instant::now() >= deadline {
                super::child_shutdown::stop(self, "reap_deadline")?;
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "child did not exit before its reap deadline",
                ));
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}

/// Only a dedicated process owner may follow a deliberately long-lived child.
/// Each poll returns promptly; request threads use bounded_wait instead.
pub(crate) fn supervise(child: &mut Child) -> io::Result<ExitStatus> {
    drop(child.stdin.take());
    loop {
        if let Some(status) = child.try_wait()? {
            return Ok(status);
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

impl BoundedChildExt for dyn portable_pty::Child + Send + Sync {
    type Status = portable_pty::ExitStatus;
    fn bounded_wait(&mut self) -> io::Result<Self::Status> {
        let deadline = Instant::now() + REAP_LIMIT;
        loop {
            if let Some(status) = self.try_wait()? {
                return Ok(status);
            }
            if Instant::now() >= deadline {
                self.kill()?;
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "PTY child exceeded its reap deadline",
                ));
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}

pub(crate) trait AsyncBoundedCommandExt {
    async fn bounded_output(&mut self) -> io::Result<Output>;
    async fn status_for(&mut self, work: Work) -> io::Result<ExitStatus>;
}

impl AsyncBoundedCommandExt for tokio::process::Command {
    async fn status_for(&mut self, work: Work) -> io::Result<ExitStatus> {
        self.kill_on_drop(true);
        tokio::time::timeout(work_limit(work), self.status())
            .await
            .map_err(|_| {
                io::Error::new(
                    io::ErrorKind::TimedOut,
                    "async subprocess status deadline exceeded",
                )
            })?
    }
    async fn bounded_output(&mut self) -> io::Result<Output> {
        self.kill_on_drop(true);
        tokio::time::timeout(COMMAND_LIMIT, self.output())
            .await
            .map_err(|_| {
                io::Error::new(
                    io::ErrorKind::TimedOut,
                    "async subprocess output deadline exceeded",
                )
            })?
    }
}

pub(crate) trait AsyncChildOwner {
    async fn supervised_wait(&mut self) -> io::Result<ExitStatus>;
}

impl AsyncChildOwner for tokio::process::Child {
    async fn supervised_wait(&mut self) -> io::Result<ExitStatus> {
        drop(self.stdin.take());
        loop {
            if let Some(status) = self.try_wait()? {
                return Ok(status);
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }
}

#[cfg(all(test, unix))]
mod progress_tests {
    use super::*;

    #[test]
    fn failed_progress_sample_stops_the_child_promptly() {
        let mut command = Command::new("sh");
        command.args(["-c", "echo $$ >&2; exec sleep 60"]);
        let started = Instant::now();
        let mut calls = 0;
        let mut position = || {
            calls += 1;
            if calls > 3 {
                Err(io::Error::other("cancelled"))
            } else {
                Ok(0)
            }
        };
        let error = command
            .output_with_progress(Duration::from_secs(30), &mut position)
            .unwrap_err();
        assert!(error.to_string().contains("cancelled"));
        assert!(started.elapsed() < Duration::from_secs(5));
    }

    #[test]
    fn mutation_work_outlives_the_query_cap_and_ends_at_its_own_deadline() {
        let slow = || {
            let mut command = Command::new("sh");
            command.args(["-c", "sleep 2"]);
            command
        };
        with_test_command_limit(Duration::from_millis(300), || {
            assert!(slow().bounded_output().is_err());
            assert!(slow().output_for(Work::Mutation).unwrap().status.success());
            with_test_mutation_limit(Duration::from_millis(300), || {
                let error = slow().output_for(Work::Mutation).unwrap_err();
                assert_eq!(error.kind(), io::ErrorKind::TimedOut);
            });
        });
    }

    #[test]
    fn async_mutation_status_outlives_the_query_cap() {
        let slow = || {
            let mut command = tokio::process::Command::new("sh");
            command.args(["-c", "sleep 2"]);
            command
        };
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        // Test limits are per thread, so the runtime runs on this thread.
        with_test_command_limit(Duration::from_millis(300), || {
            runtime.block_on(async {
                assert!(slow().status_for(Work::Query).await.is_err());
                assert!(slow().status_for(Work::Mutation).await.unwrap().success());
            })
        });
    }
}
