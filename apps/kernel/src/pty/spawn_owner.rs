use portable_pty::{Child, CommandBuilder, SlavePty};

type SpawnResult = Result<Box<dyn Child + Send + Sync>, String>;

#[cfg(not(target_os = "linux"))]
pub(super) fn spawn(slave: Box<dyn SlavePty + Send>, command: CommandBuilder) -> SpawnResult {
    slave
        .spawn_command(command)
        .map_err(|error| error.to_string())
}

#[cfg(target_os = "linux")]
pub(super) fn spawn(slave: Box<dyn SlavePty + Send>, command: CommandBuilder) -> SpawnResult {
    use std::sync::{mpsc, OnceLock};

    struct Request {
        slave: Box<dyn SlavePty + Send>,
        command: CommandBuilder,
        reply: mpsc::SyncSender<SpawnResult>,
    }

    // Linux parent-death signals follow the spawning THREAD, not its process.
    // A Tokio blocking worker can retire while the kernel and provider remain
    // active. Keep this owner alive for the kernel's lifetime, retaining bwrap's
    // --die-with-parent protection without coupling it to a temporary caller.
    static OWNER: OnceLock<Result<mpsc::SyncSender<Request>, String>> = OnceLock::new();
    let owner = OWNER
        .get_or_init(|| {
            let (sender, receiver) = mpsc::sync_channel::<Request>(16);
            std::thread::Builder::new()
                .name("chariox-pty-spawn-owner".into())
                .spawn(move || {
                    for request in receiver {
                        let result = request
                            .slave
                            .spawn_command(request.command)
                            .map_err(|error| error.to_string());
                        if let Err(mpsc::SendError(Ok(mut child))) = request.reply.send(result) {
                            // Never orphan a child if its requesting caller disappeared.
                            let _ = child.kill();
                            let _ = child.wait();
                        }
                    }
                })
                .map_err(|error| format!("could not start PTY spawn owner: {error}"))?;
            Ok(sender)
        })
        .as_ref()
        .map_err(Clone::clone)?;

    let (reply, response) = mpsc::sync_channel(1);
    owner
        .send(Request {
            slave,
            command,
            reply,
        })
        .map_err(|_| "PTY spawn owner disconnected".to_string())?;
    response
        .recv()
        .map_err(|_| "PTY spawn owner lost its response".to_string())?
}
