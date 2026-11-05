//! Child handles pin the process identity until reaped, even when process
//! inspection is unavailable. Never depend on a process-table scan to kill one.
use std::io;
use std::process::Child;
use std::time::{Duration, Instant};

const TERM_GRACE: Duration = Duration::from_millis(250);
const REAP_LIMIT: Duration = Duration::from_secs(1);

#[track_caller]
pub(crate) fn stop(child: &mut Child, operation: &'static str) -> io::Result<()> {
    let _watch = super::stall_watchdog::watch("child_shutdown", operation);
    let result = stop_inner(child);
    if let Err(error) = &result {
        crate::logging::error_with_fields(
            "child_shutdown",
            "child shutdown failed",
            serde_json::json!({"operation": operation, "pid": child.id(), "error": error.to_string()}),
        );
    }
    result
}

fn stop_inner(child: &mut Child) -> io::Result<()> {
    if child.try_wait()?.is_some() {
        return Ok(());
    }
    let pid = i32::try_from(child.id())
        .ok()
        .filter(|pid| *pid > 1)
        .ok_or_else(|| io::Error::other("refusing to signal a reserved child PID"))?;
    #[cfg(unix)]
    if unsafe { libc::kill(pid, libc::SIGTERM) } != 0 {
        let error = io::Error::last_os_error();
        if error.raw_os_error() != Some(libc::ESRCH) {
            return Err(error);
        }
    }
    #[cfg(not(unix))]
    let _ = pid;
    let exited = wait_until(child, Instant::now() + TERM_GRACE)?;
    // Keep the child handle unreaped until the escalation decision; Child::kill
    // targets only this child and works even if inspecting /proc failed.
    if !exited {
        child.kill()?;
    }
    if !exited && !wait_until(child, Instant::now() + REAP_LIMIT)? {
        return Err(io::Error::new(
            io::ErrorKind::TimedOut,
            format!(
                "child {} did not exit within the TERM/KILL reap deadline",
                child.id()
            ),
        ));
    }
    Ok(())
}

fn wait_until(child: &mut Child, deadline: Instant) -> io::Result<bool> {
    loop {
        if child.try_wait()?.is_some() {
            return Ok(true);
        }
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Ok(false);
        }
        std::thread::sleep(remaining.min(Duration::from_millis(10)));
    }
}
