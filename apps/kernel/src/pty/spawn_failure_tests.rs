use super::*;

struct SpawnRefused(i32, bool);

impl portable_pty::SlavePty for SpawnRefused {
    fn spawn_command(
        &self,
        _command: CommandBuilder,
    ) -> anyhow::Result<Box<dyn Child + Send + Sync>> {
        let error = anyhow::Error::new(std::io::Error::from_raw_os_error(self.0));
        Err(if self.1 {
            error.context("dup of fd 317 failed")
        } else {
            error
        })
    }
}

#[test]
fn resource_refusals_keep_wrapped_errno_out_of_the_client() {
    for context in [false, true] {
        for errno in [libc::EMFILE, libc::ENFILE, libc::EAGAIN, libc::ENOMEM] {
            let error = crate::process_spawn::spawn_pty(
                Box::new(SpawnRefused(errno, context)),
                CommandBuilder::new("/bin/cat"),
            )
            .unwrap_err();
            let refusal = spawn_failed("private-run", &error);
            let public = crate::transport::kernel_protocol::map_kernel_error(&refusal);
            let expected = match errno {
            libc::EMFILE | libc::ENFILE => {
                "too many open files in this kernel; close a terminal and try again"
            }
            _ => "not enough resources in this kernel to open a terminal; close a terminal and try again",
        };
            assert_eq!(public.message, expected, "errno {errno}");
            assert_eq!(
                public.code,
                match errno {
                    libc::EMFILE | libc::ENFILE => "too_many_open_files",
                    _ => "terminal_refused",
                }
            );
            assert_eq!(
                public.retryable,
                matches!(errno, libc::EMFILE | libc::ENFILE)
            );
            assert!(!public.message.contains("private-run"));
            assert!(!public.message.contains("317"));
        }
    }
    let error = std::io::Error::from_raw_os_error(libc::ENOENT);
    assert!(matches!(
        spawn_failed("missing", &error),
        DaemonError::PtySpawn { .. }
    ));
}

struct WriterRefused {
    master: Box<dyn MasterPty + Send>,
    errno: i32,
}

#[test]
fn openpty_resource_refusals_keep_formatted_errno_out_of_the_client() {
    for errno in [libc::EMFILE, libc::ENFILE, libc::EAGAIN, libc::ENOMEM] {
        // portable-pty 0.8.1 formats openpty's IO error rather than retaining it.
        let error = anyhow::anyhow!(
            "failed to openpty: {:?}",
            std::io::Error::from_raw_os_error(errno)
        );
        let refusal = spawn_failed("private-run", &*error);
        let public = crate::transport::kernel_protocol::map_kernel_error(&refusal);
        assert_eq!(
            public.code,
            match errno {
                libc::EMFILE | libc::ENFILE => "too_many_open_files",
                _ => "terminal_refused",
            }
        );
        assert!(public.message.contains("close a terminal and try again"));
        assert!(!public.message.contains("openpty"));
        assert!(!public.message.contains("private-run"));
    }
    let error = anyhow::anyhow!(
        "failed to openpty: {:?}",
        std::io::Error::from_raw_os_error(libc::ENOENT)
    );
    assert!(matches!(
        spawn_failed("missing", &*error),
        DaemonError::PtySpawn { .. }
    ));
}

impl MasterPty for WriterRefused {
    fn resize(&self, size: PtySize) -> anyhow::Result<()> {
        self.master.resize(size)
    }
    fn get_size(&self) -> anyhow::Result<PtySize> {
        self.master.get_size()
    }
    fn try_clone_reader(&self) -> anyhow::Result<Box<dyn Read + Send>> {
        self.master.try_clone_reader()
    }
    fn take_writer(&self) -> anyhow::Result<Box<dyn Write + Send>> {
        Err(
            anyhow::Error::new(std::io::Error::from_raw_os_error(self.errno))
                .context("dup of fd 317 failed"),
        )
    }
    fn process_group_leader(&self) -> Option<libc::pid_t> {
        self.master.process_group_leader()
    }
    fn as_raw_fd(&self) -> Option<std::os::fd::RawFd> {
        self.master.as_raw_fd()
    }
}

#[cfg(target_os = "linux")]
#[test]
fn failed_attach_releases_descriptors_and_reaps_its_child() {
    crate::test_support::isolated_env_test!();
    let mut manager = PtyManager::new();
    let fds = || std::fs::read_dir("/proc/self/fd").unwrap().count();
    let before = fds();
    let mut unreaped = Vec::new();
    for (i, errno) in [libc::EMFILE, libc::ENFILE, libc::EAGAIN, libc::ENOMEM]
        .into_iter()
        .cycle()
        .take(12)
        .enumerate()
    {
        let pair = native_pty_system().openpty(PtySize::default()).unwrap();
        let child = pair
            .slave
            .spawn_command(CommandBuilder::new("/bin/cat"))
            .unwrap();
        drop(pair.slave);
        let pid = child.process_id().unwrap();
        assert!(pid > 1 && pid <= i32::MAX as u32);
        let resources = crate::runtime::process_resources::PreparedDomain::for_tests()
            .start(pid)
            .unwrap();
        let refusal = manager.attach(
            format!("failed-{i}"),
            format!("failed-{i}"),
            None,
            Box::new(WriterRefused {
                master: pair.master,
                errno,
            }),
            child,
            resources,
            None,
        );
        assert!(refusal.is_err());
        assert!(manager.processes.is_empty());
        assert!(manager.process_aliases.is_empty());
        // waitpid must say that attach already reaped the child. Reap it here
        // on the red path too, so the regression never leaves a zombie behind.
        let waited = unsafe { libc::waitpid(pid as i32, std::ptr::null_mut(), 0) };
        if waited != -1 || std::io::Error::last_os_error().raw_os_error() != Some(libc::ECHILD) {
            unreaped.push(pid);
        }
    }
    let after = fds();
    eprintln!(
        "12 failed attaches: descriptors {before} -> {after}; unreaped {}",
        unreaped.len()
    );
    assert_eq!(after, before, "failed attaches leaked descriptors");
    assert!(
        unreaped.is_empty(),
        "failed attaches left children unreaped: {unreaped:?}"
    );
}
