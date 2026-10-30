//! Long-lived children must belong to the kernel process, not a temporary caller.
use portable_pty::{CommandBuilder, SlavePty};
use std::io;
use std::process::{Child, Command};

#[cfg(all(test, target_os = "linux"))]
#[path = "process_spawn_tests.rs"]
mod tests;

/// Kernel workloads must be killable without lowering a positive service policy.
pub(crate) fn command(program: impl AsRef<std::ffi::OsStr>) -> Command {
    let mut command = Command::new(program);
    make_killable(&mut command);
    command
}

pub(crate) fn async_command(program: impl AsRef<std::ffi::OsStr>) -> tokio::process::Command {
    command(program).into()
}

fn make_killable(command: &mut Command) {
    #[cfg(target_os = "linux")]
    {
        use std::os::unix::process::CommandExt;
        // Only async-signal-safe syscalls run between fork and exec, so a
        // refusal cannot be logged here. Killability is best effort: the
        // kernel is non-dumpable (PR_SET_DUMPABLE 0), which makes its
        // /proc/self files root-owned, so an ordinary user may not open
        // oom_score_adj for writing. Open read-only first and write only to
        // raise an inherited negative adjustment (which needs privilege the
        // caller then has); never fail the spawn.
        unsafe {
            command.pre_exec(|| {
                let fd = libc::open(
                    c"/proc/self/oom_score_adj".as_ptr(),
                    libc::O_RDONLY | libc::O_CLOEXEC,
                );
                if fd < 0 {
                    return Ok(());
                }
                let mut adjustment = [0_u8; 16];
                let count = loop {
                    let result = libc::read(fd, adjustment.as_mut_ptr().cast(), adjustment.len());
                    if result < 0 && io::Error::last_os_error().kind() == io::ErrorKind::Interrupted
                    {
                        continue;
                    }
                    break result;
                };
                libc::close(fd);
                // A positive adjustment is already killable and an
                // unprivileged child may not lower it.
                if count > 0 && adjustment[0] == b'-' {
                    let fd = libc::open(
                        c"/proc/self/oom_score_adj".as_ptr(),
                        libc::O_WRONLY | libc::O_CLOEXEC,
                    );
                    if fd >= 0 {
                        libc::write(fd, b"0\n".as_ptr().cast(), 2);
                        libc::close(fd);
                    }
                }
                Ok(())
            });
        }
    }
    #[cfg(not(target_os = "linux"))]
    let _ = command;
}

pub(crate) fn spawn_command(mut command: Command) -> io::Result<Child> {
    make_killable(&mut command);
    on_owner(
        move || command.spawn(),
        |mut child| {
            let _ = child.kill();
            let _ = child.wait();
        },
    )
}

// Keep shell/native entry policies aligned with the standard pre-exec hook.
#[cfg(target_os = "linux")]
pub(crate) const KILLABLE_SHELL_PREFIX: &str = concat!(
    "IFS= read -r chariox_oom_score_adj < /proc/self/oom_score_adj; ",
    "case \"$chariox_oom_score_adj\" in -*) printf '0\\n' > /proc/self/oom_score_adj;; esac;"
);

pub(crate) fn spawn_pty(
    slave: Box<dyn SlavePty + Send>,
    mut command: CommandBuilder,
) -> io::Result<Box<dyn portable_pty::Child + Send + Sync>> {
    #[cfg(target_os = "linux")]
    {
        let argv = std::mem::take(command.get_argv_mut());
        let entry = format!("{KILLABLE_SHELL_PREFIX} exec \"$@\"");
        *command.get_argv_mut() = ["/bin/sh", "-c", &entry, "chariox-owned-process"]
            .into_iter()
            .map(std::ffi::OsString::from)
            .chain(argv)
            .collect();
    }
    on_owner(
        move || slave.spawn_command(command).map_err(io::Error::other),
        |mut child| {
            let _ = child.kill();
            let _ = child.wait();
        },
    )
}

#[cfg(not(target_os = "linux"))]
fn on_owner<T>(
    spawn: impl FnOnce() -> io::Result<T> + Send + 'static,
    _cleanup: fn(T),
) -> io::Result<T>
where
    T: Send + 'static,
{
    spawn()
}

#[cfg(target_os = "linux")]
fn on_owner<T>(
    spawn: impl FnOnce() -> io::Result<T> + Send + 'static,
    cleanup: fn(T),
) -> io::Result<T>
where
    T: Send + 'static,
{
    use std::sync::{mpsc, OnceLock};
    type Job = Box<dyn FnOnce() + Send>;
    // Linux parent-death signals follow the spawning THREAD. Tokio blocking
    // workers can retire while their children are active. This owner stays
    // alive for the process lifetime, preserving bwrap --die-with-parent.
    static OWNER: OnceLock<Result<mpsc::SyncSender<Job>, String>> = OnceLock::new();
    let owner = OWNER
        .get_or_init(|| {
            let (sender, receiver) = mpsc::sync_channel::<Job>(16);
            std::thread::Builder::new()
                .name("chariox-process-spawn-owner".into())
                .spawn(move || {
                    for job in receiver {
                        // A backend panic must not kill this thread: existing
                        // sandboxes depend on its lifetime, and later callers
                        // must still be able to spawn. Unwinding drops the
                        // failed job's reply sender, returning an error to it.
                        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(job));
                    }
                })
                .map_err(|error| format!("could not start process spawn owner: {error}"))?;
            Ok(sender)
        })
        .as_ref()
        .map_err(|error| io::Error::other(error.clone()))?;
    // Rendezvous transfers ownership to the waiting caller, never to an
    // abandoned reply buffer. An unavailable caller leaves cleanup here.
    let (reply, response) = mpsc::sync_channel(0);
    owner
        .send(Box::new(move || {
            if let Err(mpsc::SendError(Ok(child))) = reply.send(spawn()) {
                cleanup(child);
            }
        }))
        .map_err(|_| io::Error::other("process spawn owner disconnected"))?;
    response
        .recv()
        .map_err(|_| io::Error::other("process spawn owner lost its response"))?
}
