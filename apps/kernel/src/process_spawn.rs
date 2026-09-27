//! Long-lived children must belong to the kernel process, not a temporary caller.
use portable_pty::{CommandBuilder, SlavePty};
use std::io;
use std::process::{Child, Command};

#[cfg(all(test, target_os = "linux"))]
#[path = "process_spawn_tests.rs"]
mod tests;

pub(crate) fn spawn_command(mut command: Command) -> io::Result<Child> {
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
    command: CommandBuilder,
) -> io::Result<Box<dyn portable_pty::Child + Send + Sync>> {
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
