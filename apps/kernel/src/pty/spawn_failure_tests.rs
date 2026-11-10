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

/// Every open descriptor and what it points at.
#[cfg(target_os = "linux")]
fn descriptors() -> std::collections::BTreeMap<String, String> {
    std::fs::read_dir("/proc/self/fd")
        .unwrap()
        .filter_map(|entry| {
            let path = entry.ok()?.path();
            // A descriptor closed since the listing (the directory's own) has no target.
            let target = std::fs::read_link(&path).ok()?;
            Some((
                path.file_name()?.to_string_lossy().into_owned(),
                target.to_string_lossy().into_owned(),
            ))
        })
        .collect()
}

/// The descriptors `after` holds that `before` did not, except the listing's own.
#[cfg(target_os = "linux")]
fn opened_since(
    before: &std::collections::BTreeMap<String, String>,
    after: &std::collections::BTreeMap<String, String>,
) -> Vec<(String, String)> {
    let listing = format!("/proc/{}/fd", std::process::id());
    after
        .iter()
        .filter(|(fd, target)| before.get(*fd) != Some(*target) && **target != listing)
        .map(|(fd, target)| (fd.clone(), target.clone()))
        .collect()
}

#[cfg(target_os = "linux")]
#[test]
fn failed_attach_releases_descriptors_and_reaps_its_child() {
    crate::test_support::isolated_env_test!();
    let mut manager = PtyManager::new();
    let before = descriptors();
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
    // The process-wide sampler runs `ps` every 200 ms on its own thread
    // (process_resources/sampling.rs `run_ps`), and the pipes that capture its
    // output are open here for as long as one `ps` lives. A descriptor that a
    // failed attach left behind never closes; a `ps` pipe closes within one
    // sampling period, so wait for the extra ones to go.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    let mut transient = std::collections::BTreeSet::new();
    let (after, leaked) = loop {
        let after = descriptors();
        let extra = opened_since(&before, &after);
        if extra.is_empty() || std::time::Instant::now() > deadline {
            break (after, extra);
        }
        transient.extend(extra);
        std::thread::yield_now();
    };
    eprintln!(
        "12 failed attaches: descriptors {} -> {}; held meanwhile by the sampler {transient:?}; unreaped {}",
        before.len(),
        after.len(),
        unreaped.len()
    );
    assert!(
        leaked.is_empty(),
        "failed attaches leaked descriptors: {leaked:?}"
    );
    assert!(
        unreaped.is_empty(),
        "failed attaches left children unreaped: {unreaped:?}"
    );
}
