//! Event-gated fork/exec fixture for descriptor-lifetime regressions.
use std::{
    io::{Read, Write},
    os::unix::{io::AsRawFd, net::UnixStream, process::CommandExt},
    process::{Command, ExitStatus, Stdio},
    thread::JoinHandle,
    time::Duration,
};

pub(crate) struct PreExecChild {
    parent: UnixStream,
    spawning: Option<JoinHandle<std::io::Result<ExitStatus>>>,
}

impl PreExecChild {
    /// Returns only once the child has inherited the parent's descriptors but
    /// has not exec'd. CLOEXEC cannot release those references yet.
    pub(crate) fn pause() -> Self {
        unsafe { Self::pause_after_fork(|| Ok(())) }
    }

    /// # Safety
    /// The callback runs after fork in a multithreaded process. It must use
    /// only async-signal-safe operations, without allocating or locking.
    pub(crate) unsafe fn pause_after_fork(
        mut after_fork: impl FnMut() -> std::io::Result<()> + Send + Sync + 'static,
    ) -> Self {
        let (parent, child) = UnixStream::pair().unwrap();
        parent
            .set_read_timeout(Some(Duration::from_secs(10)))
            .unwrap();
        child
            .set_read_timeout(Some(Duration::from_secs(10)))
            .unwrap();
        let executable = std::env::current_exe().expect("current test executable");
        let spawning = std::thread::spawn(move || {
            let fd = child.as_raw_fd();
            let mut command = Command::new(executable);
            command
                .arg("--list")
                .stdout(Stdio::null())
                .stderr(Stdio::null());
            // Only async-signal-safe syscalls in the forked child.
            unsafe {
                command.pre_exec(move || {
                    after_fork()?;
                    if libc::write(fd, b"R".as_ptr().cast(), 1) != 1 {
                        return Err(std::io::Error::last_os_error());
                    }
                    let mut release = 0_u8;
                    if libc::read(fd, (&mut release as *mut u8).cast(), 1) != 1 {
                        return Err(std::io::Error::last_os_error());
                    }
                    Ok(())
                });
            }
            let result = command.spawn().and_then(|mut process| process.wait());
            drop(child);
            result
        });
        let mut gate = Self {
            parent,
            spawning: Some(spawning),
        };
        let mut ready = [0];
        gate.parent.read_exact(&mut ready).unwrap();
        assert_eq!(ready, *b"R");
        gate
    }

    pub(crate) fn resume_and_wait(mut self) {
        self.parent.write_all(b"G").unwrap();
        assert!(self
            .spawning
            .take()
            .unwrap()
            .join()
            .unwrap()
            .unwrap()
            .success());
    }
}

impl Drop for PreExecChild {
    fn drop(&mut self) {
        if let Some(spawning) = self.spawning.take() {
            let _ = self.parent.write_all(b"G");
            let _ = spawning.join();
        }
    }
}

#[test]
fn paused_child_executes_the_current_test_binary_after_release() {
    // Exercises the inherited-descriptor barrier and executable lookup together.
    PreExecChild::pause().resume_and_wait();
}
