//! Long-lived children must belong to the kernel process, not a temporary caller.
use portable_pty::{CommandBuilder, SlavePty};
use std::io;
use std::process::{Child, Command};

#[cfg(all(test, target_os = "linux"))]
#[path = "process_spawn_tests.rs"]
mod tests;

/// Every kernel-owned command starts with its own OOM policy. In particular,
/// a workload must not inherit the service's -1000 protection.
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
        // Only async-signal-safe syscalls run between fork and exec. Raising an
        // inherited negative adjustment to zero requires no added privilege.
        unsafe {
            command.pre_exec(|| {
                let fd = libc::open(
                    c"/proc/self/oom_score_adj".as_ptr(),
                    libc::O_WRONLY | libc::O_CLOEXEC,
                );
                if fd < 0 {
                    return Err(io::Error::last_os_error());
                }
                let written = loop {
                    let result = libc::write(fd, b"0\n".as_ptr().cast(), 2);
                    if result < 0 && io::Error::last_os_error().kind() == io::ErrorKind::Interrupted
                    {
                        continue;
                    }
                    break result;
                };
                let error = io::Error::last_os_error();
                let closed = libc::close(fd);
                if written != 2 {
                    return Err(error);
                }
                if closed != 0 {
                    return Err(io::Error::last_os_error());
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

pub(crate) fn spawn_pty(
    slave: Box<dyn SlavePty + Send>,
    mut command: CommandBuilder,
) -> io::Result<Box<dyn portable_pty::Child + Send + Sync>> {
    #[cfg(target_os = "linux")]
    {
        let argv = std::mem::take(command.get_argv_mut());
        *command.get_argv_mut() = [
            "/bin/sh",
            "-c",
            "printf '0\\n' > /proc/self/oom_score_adj || exit 125; exec \"$@\"",
            "chariox-owned-process",
        ]
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
