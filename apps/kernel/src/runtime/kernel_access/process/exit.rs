use super::ProcessIdentity;
use std::time::Duration;

impl ProcessIdentity {
    /// Pin the Linux process before checking its identity, so PID reuse cannot
    /// leave a grant waiting for the replacement process to exit. A check that could
    /// not answer is not an exit: the wait goes on.
    pub(crate) async fn wait_for_exit(&self) {
        #[cfg(target_os = "linux")]
        {
            let watched = exit_descriptor(self.pid).and_then(tokio::io::unix::AsyncFd::new);
            if self.gone() {
                return;
            }
            match watched {
                Ok(watched) => match watched.readable().await {
                    Ok(_) => return,
                    Err(error) => warn_watch_unavailable(error),
                },
                Err(error) => warn_watch_unavailable(error),
            }
        }
        // Older Linux kernels and other platforms still bound cleanup without
        // shortening the unrelated transport maintenance interval.
        while !self.gone() {
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
    }
}

#[cfg(target_os = "linux")]
fn exit_descriptor(pid: u32) -> std::io::Result<std::os::fd::OwnedFd> {
    use std::os::fd::FromRawFd;
    let descriptor = unsafe { libc::syscall(libc::SYS_pidfd_open, pid, 0) };
    if descriptor < 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(unsafe { std::os::fd::OwnedFd::from_raw_fd(descriptor as i32) })
}

#[cfg(target_os = "linux")]
fn warn_watch_unavailable(error: std::io::Error) {
    crate::logging::warn_with_fields(
        "kernel.access",
        "process exit watch unavailable; checking holder identity every 250 ms",
        serde_json::json!({"error": error.to_string()}),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn an_exit_watch_ends_for_the_original_process_and_a_changed_identity() {
        let mut child = std::process::Command::new("sleep")
            .arg("30")
            .spawn()
            .unwrap();
        let identity = super::super::inspect(child.id()).unwrap().0;
        let mut changed = identity.clone();
        changed.start += 1;
        tokio::time::timeout(Duration::from_secs(1), changed.wait_for_exit())
            .await
            .unwrap();
        let exited = identity.wait_for_exit();
        tokio::pin!(exited);
        assert!(tokio::time::timeout(Duration::from_millis(50), &mut exited)
            .await
            .is_err());
        assert!(child.id() > 1);
        child.kill().unwrap();
        child.wait().unwrap();
        tokio::time::timeout(Duration::from_secs(1), exited)
            .await
            .unwrap();
    }
}
