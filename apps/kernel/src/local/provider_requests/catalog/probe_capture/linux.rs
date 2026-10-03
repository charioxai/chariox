use std::io;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::path::PathBuf;

pub(super) fn terminate(fds: &[RawFd]) -> io::Result<()> {
    let pipes = fds
        .iter()
        .map(|fd| std::fs::read_link(format!("/proc/self/fd/{fd}")))
        .collect::<io::Result<Vec<_>>>()?;
    // Retry after signalling so a child forked during the first scan is also
    // found. The read ends stay open throughout, preventing pipe inode reuse.
    for _ in 0..3 {
        for entry in std::fs::read_dir("/proc")?.flatten() {
            let Some(pid) = entry
                .file_name()
                .to_str()
                .and_then(|value| value.parse::<u32>().ok())
            else {
                continue;
            };
            if pid == std::process::id() || !owns_writer(pid, &pipes) {
                continue;
            }
            let fd = unsafe { libc::syscall(libc::SYS_pidfd_open, pid, 0) };
            if fd < 0 {
                let error = io::Error::last_os_error();
                if error.raw_os_error() == Some(libc::ESRCH) {
                    continue;
                }
                return Err(error);
            }
            let process = unsafe { OwnedFd::from_raw_fd(fd as RawFd) };
            // Pin the process before the final ownership check. Sending to a
            // pidfd cannot signal a different process if the numeric PID changes.
            if owns_writer(pid, &pipes) {
                let result = unsafe {
                    libc::syscall(
                        libc::SYS_pidfd_send_signal,
                        process.as_raw_fd(),
                        libc::SIGKILL,
                        std::ptr::null::<libc::siginfo_t>(),
                        0,
                    )
                };
                if result < 0 {
                    let error = io::Error::last_os_error();
                    if error.raw_os_error() != Some(libc::ESRCH) {
                        return Err(error);
                    }
                }
            }
        }
    }
    Ok(())
}

fn owns_writer(pid: u32, pipes: &[PathBuf]) -> bool {
    let Ok(fds) = std::fs::read_dir(format!("/proc/{pid}/fd")) else {
        return false;
    };
    fds.flatten().any(|fd| {
        if !std::fs::read_link(fd.path()).is_ok_and(|pipe| pipes.contains(&pipe)) {
            return false;
        }
        // Other concurrent forks can briefly inherit our read ends before
        // close-on-exec. Those handles do not establish probe ownership.
        std::fs::read_to_string(format!(
            "/proc/{pid}/fdinfo/{}",
            fd.file_name().to_string_lossy()
        ))
        .ok()
        .and_then(|info| {
            info.lines().find_map(|line| {
                line.strip_prefix("flags:")
                    .and_then(|value| u32::from_str_radix(value.trim(), 8).ok())
            })
        })
        .is_some_and(|flags| flags as i32 & libc::O_ACCMODE != libc::O_RDONLY)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inherited_read_ends_do_not_establish_probe_ownership() {
        let mut fds = [0; 2];
        assert_eq!(unsafe { libc::pipe2(fds.as_mut_ptr(), libc::O_CLOEXEC) }, 0);
        let reader = unsafe { OwnedFd::from_raw_fd(fds[0]) };
        let writer = unsafe { OwnedFd::from_raw_fd(fds[1]) };
        let pipes = [std::fs::read_link(format!("/proc/self/fd/{}", reader.as_raw_fd())).unwrap()];
        assert!(owns_writer(std::process::id(), &pipes));
        drop(writer);
        assert!(!owns_writer(std::process::id(), &pipes));
    }
}
