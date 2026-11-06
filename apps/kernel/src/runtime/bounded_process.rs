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

#[cfg(test)]
thread_local! {
    static TEST_COMMAND_LIMIT: std::cell::Cell<Option<Duration>> = const { std::cell::Cell::new(None) };
}

fn command_limit() -> Duration {
    #[cfg(test)]
    if let Some(limit) = TEST_COMMAND_LIMIT.with(std::cell::Cell::get) {
        return limit;
    }
    COMMAND_LIMIT
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
        self.output_with_deadline(command_limit())
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
    async fn bounded_status(&mut self) -> io::Result<ExitStatus>;
    async fn bounded_output(&mut self) -> io::Result<Output>;
}

impl AsyncBoundedCommandExt for tokio::process::Command {
    async fn bounded_status(&mut self) -> io::Result<ExitStatus> {
        self.kill_on_drop(true);
        tokio::time::timeout(COMMAND_LIMIT, self.status())
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
    async fn output_with_deadline(self, timeout: Duration) -> io::Result<Output>;
    async fn supervised_wait(&mut self) -> io::Result<ExitStatus>;
}

impl AsyncChildOwner for tokio::process::Child {
    async fn output_with_deadline(self, timeout: Duration) -> io::Result<Output> {
        let mut owned = AsyncChildGuard(self);
        drop(owned.0.stdin.take());
        let stdout = owned.0.stdout.take();
        let stderr = owned.0.stderr.take();
        let budget = std::sync::atomic::AtomicUsize::new(0);
        let result = tokio::time::timeout(timeout, async {
            tokio::try_join!(
                owned.0.supervised_wait(),
                read_async_pipe(stdout, &budget),
                read_async_pipe(stderr, &budget)
            )
        })
        .await;
        match result {
            Ok(Ok((status, stdout, stderr))) => Ok(Output {
                status,
                stdout,
                stderr,
            }),
            failed => {
                let error = match failed {
                    Ok(Err(error)) => error,
                    _ => io::Error::new(
                        io::ErrorKind::TimedOut,
                        "async child output deadline exceeded",
                    ),
                };
                if owned.0.try_wait()?.is_none() {
                    owned.0.start_kill()?;
                }
                tokio::time::timeout(REAP_LIMIT, owned.0.supervised_wait())
                    .await
                    .map_err(|_| {
                        io::Error::new(
                            io::ErrorKind::TimedOut,
                            "async child reap deadline exceeded",
                        )
                    })??;
                Err(error)
            }
        }
    }
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

struct AsyncChildGuard(tokio::process::Child);
impl Drop for AsyncChildGuard {
    fn drop(&mut self) {
        if self.0.id().is_some_and(|pid| pid > 1) {
            if let Err(error) = self.0.start_kill() {
                crate::logging::error_with_fields(
                    "subprocess",
                    "async child cancellation failed",
                    serde_json::json!({"operation": "async_child_drop", "error": error.to_string()}),
                );
            }
        }
    }
}

async fn read_async_pipe<R: tokio::io::AsyncRead + Unpin>(
    pipe: Option<R>,
    budget: &std::sync::atomic::AtomicUsize,
) -> io::Result<Vec<u8>> {
    use std::sync::atomic::Ordering;
    use tokio::io::AsyncReadExt;
    let Some(mut pipe) = pipe else {
        return Ok(Vec::new());
    };
    let mut bytes = Vec::new();
    let mut buffer = [0; 8192];
    loop {
        let read = pipe.read(&mut buffer).await?;
        if read == 0 {
            return Ok(bytes);
        }
        if budget.fetch_add(read, Ordering::Relaxed) > (64 * 1024 * 1024usize).saturating_sub(read)
        {
            return Err(io::Error::other(
                "async child output exceeded the capture budget",
            ));
        }
        bytes.extend_from_slice(&buffer[..read]);
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
}
