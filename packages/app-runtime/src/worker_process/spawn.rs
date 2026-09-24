use std::{
    ffi::CString,
    fs::File,
    io,
    mem::MaybeUninit,
    os::fd::{AsRawFd, FromRawFd, OwnedFd},
};

fn checked(code: libc::c_int) -> io::Result<()> {
    if code == 0 {
        Ok(())
    } else {
        Err(io::Error::from_raw_os_error(code))
    }
}

struct Actions(libc::posix_spawn_file_actions_t);
impl Drop for Actions {
    fn drop(&mut self) {
        unsafe {
            libc::posix_spawn_file_actions_destroy(&mut self.0);
        }
    }
}
struct Attributes(libc::posix_spawnattr_t);
impl Drop for Attributes {
    fn drop(&mut self) {
        unsafe {
            libc::posix_spawnattr_destroy(&mut self.0);
        }
    }
}

/// File actions execute inside libc's spawn implementation, preserving its
/// private exec-error reporting channel. No Rust code runs after fork.
pub(super) fn launch(
    program: &CString,
    arguments: &[CString],
    files: &[&File],
) -> io::Result<libc::pid_t> {
    if !(5..=7).contains(&files.len()) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "worker descriptor count",
        ));
    }
    #[cfg(target_os = "macos")]
    if files.len() != 5 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "worker descriptor count",
        ));
    }
    let boundary = files.len() as i32;
    let mut raw_actions = MaybeUninit::uninit();
    let mut raw_attributes = MaybeUninit::uninit();
    unsafe {
        // This owner requires waitpid authority. Automatic SIGCHLD reaping
        // could free the PID before we terminate its process group.
        let mut child_signal = MaybeUninit::<libc::sigaction>::zeroed();
        if libc::sigaction(libc::SIGCHLD, std::ptr::null(), child_signal.as_mut_ptr()) != 0 {
            return Err(io::Error::last_os_error());
        }
        let child_signal = child_signal.assume_init();
        if child_signal.sa_sigaction == libc::SIG_IGN
            || child_signal.sa_flags & libc::SA_NOCLDWAIT != 0
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "worker wait ownership unavailable",
            ));
        }
        checked(libc::posix_spawn_file_actions_init(
            raw_actions.as_mut_ptr(),
        ))?;
        let mut actions = Actions(raw_actions.assume_init());
        checked(libc::posix_spawnattr_init(raw_attributes.as_mut_ptr()))?;
        let mut attributes = Attributes(raw_attributes.assume_init());
        // Relocate all sources first: a low parent fd must not be overwritten by
        // an earlier dup2 action and accidentally alias SDK/control/log streams.
        let mut sources = Vec::<OwnedFd>::new();
        for (target, file) in files.iter().enumerate() {
            let source = libc::fcntl(file.as_raw_fd(), libc::F_DUPFD_CLOEXEC, boundary);
            if source < 0 {
                return Err(io::Error::last_os_error());
            }
            sources.push(OwnedFd::from_raw_fd(source));
            checked(libc::posix_spawn_file_actions_adddup2(
                &mut actions.0,
                source,
                target as i32,
            ))?;
        }
        #[cfg(target_os = "linux")]
        checked(libc::posix_spawn_file_actions_addclosefrom_np(
            &mut actions.0,
            boundary,
        ))?;

        let mut empty = MaybeUninit::<libc::sigset_t>::uninit();
        libc::sigemptyset(empty.as_mut_ptr());
        let empty = empty.assume_init();
        checked(libc::posix_spawnattr_setsigmask(&mut attributes.0, &empty))?;
        let mut defaults = MaybeUninit::<libc::sigset_t>::uninit();
        libc::sigfillset(defaults.as_mut_ptr());
        let mut defaults = defaults.assume_init();
        libc::sigdelset(&mut defaults, libc::SIGKILL);
        libc::sigdelset(&mut defaults, libc::SIGSTOP);
        checked(libc::posix_spawnattr_setsigdefault(
            &mut attributes.0,
            &defaults,
        ))?;
        // Darwin's SDK exports SETSID (0x0400); libc has no Darwin binding yet.
        #[cfg(target_os = "macos")]
        let platform_flags = 0x0400 | libc::POSIX_SPAWN_CLOEXEC_DEFAULT;
        #[cfg(target_os = "linux")]
        let platform_flags = libc::POSIX_SPAWN_SETSID;
        let flags = platform_flags as libc::c_short
            | libc::POSIX_SPAWN_SETSIGMASK as libc::c_short
            | libc::POSIX_SPAWN_SETSIGDEF as libc::c_short;
        checked(libc::posix_spawnattr_setflags(&mut attributes.0, flags))?;
        let mut argv: Vec<_> = std::iter::once(program)
            .chain(arguments)
            .map(|s| s.as_ptr().cast_mut())
            .collect();
        argv.push(std::ptr::null_mut());
        // No inherited loader, Node, provider, account, PATH or HOME state can
        // execute before native main. The loader creates its own private HOME.
        let environment = [
            CString::new("LANG=C.UTF-8").unwrap(),
            CString::new("TZ=UTC").unwrap(),
        ];
        let mut envp: Vec<_> = environment.iter().map(|s| s.as_ptr().cast_mut()).collect();
        envp.push(std::ptr::null_mut());
        let mut pid = 0;
        checked(libc::posix_spawn(
            &mut pid,
            program.as_ptr(),
            &actions.0,
            &attributes.0,
            argv.as_ptr(),
            envp.as_ptr(),
        ))?;
        Ok(pid)
    }
}
