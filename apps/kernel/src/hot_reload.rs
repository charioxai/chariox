//! Hot reload: a planned upgrade replaces the kernel program in place on
//! SIGUSR2. The process keeps its PID, so every child stays its own; running
//! terminals are handed to the new program, which adopts them. Everything
//! else (client connections, the relay, provider links) reconnects as after
//! any restart.

use std::ffi::OsString;
use std::path::PathBuf;
use std::process::Command;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::room_terminal::{RoomTerminalService, TerminalHandover};

/// The handover this program writes and reads. A new program must print the
/// same number for `--handover-format` before this one hands over to it.
pub const HANDOVER_FORMAT: u32 = 1;
const HANDOVER_ENV: &str = "CHARIOX_HANDOVER";
const VERIFY_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Debug, Default, Serialize, Deserialize)]
struct Handover {
    format: u32,
    terminals: Vec<TerminalHandover>,
}

/// How this program was started, to start its successor the same way: the
/// kernel removes secrets from its environment once it has read them.
struct Start {
    program: PathBuf,
    args: Vec<OsString>,
    env: Vec<(OsString, OsString)>,
}

static START: OnceLock<Start> = OnceLock::new();
static RECEIVED: Mutex<Option<Handover>> = Mutex::new(None);

/// Call before anything reads or removes the environment.
pub fn remember_start() {
    let mut args = std::env::args_os();
    let first = args.next().map(PathBuf::from);
    // The installed path, not the resolved file: an upgrade replaces what it names.
    let Some(program) = first
        .filter(|path| path.is_absolute())
        .or_else(|| std::env::current_exe().ok())
    else {
        return;
    };
    let _ = START.set(Start {
        program,
        args: args.collect(),
        env: std::env::vars_os()
            .filter(|(name, _)| name != HANDOVER_ENV)
            .collect(),
    });
}

/// Reads the handover the previous program left when it started this one.
pub fn receive() {
    let Some(path) = std::env::var_os(HANDOVER_ENV) else {
        return;
    };
    std::env::remove_var(HANDOVER_ENV);
    let body = std::fs::read(&path);
    let _ = std::fs::remove_file(&path);
    let handover = body
        .map_err(|error| error.to_string())
        .and_then(|body| serde_json::from_slice::<Handover>(&body).map_err(|e| e.to_string()))
        .and_then(|handover| {
            (handover.format == HANDOVER_FORMAT)
                .then_some(handover)
                .ok_or_else(|| "a handover of another format".to_string())
        });
    let handover = match handover {
        Ok(handover) => handover,
        Err(error) => {
            crate::logging::error_with_fields(
                "hot_reload.receive",
                "the handover could not be read; running terminals end as after a restart",
                serde_json::json!({ "error": error }),
            );
            return;
        }
    };
    // Children started from here on must not inherit them.
    for terminal in &handover.terminals {
        set_close_on_exec(terminal.master_fd(), true);
    }
    crate::logging::info_with_fields(
        "hot_reload.receive",
        "kernel program replaced in place",
        serde_json::json!({ "terminals": handover.terminals.len() }),
    );
    *RECEIVED.lock().unwrap_or_else(|e| e.into_inner()) = Some(handover);
}

pub(crate) fn take_terminals() -> Vec<TerminalHandover> {
    RECEIVED
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .take()
        .map(|handover| handover.terminals)
        .unwrap_or_default()
}

/// Hot-reloads on every SIGUSR2. A refused reload changes nothing: this
/// program keeps running with everything it had.
pub async fn serve(terminals: RoomTerminalService, handover_path: PathBuf) {
    use tokio::signal::unix::{signal, SignalKind};
    let mut requests = match signal(SignalKind::user_defined2()) {
        Ok(requests) => requests,
        Err(error) => {
            crate::logging::warn_with_fields(
                "hot_reload",
                "failed to register the SIGUSR2 handler; hot reload is unavailable",
                serde_json::json!({ "error": error.to_string() }),
            );
            return;
        }
    };
    while requests.recv().await.is_some() {
        let error = reload(&terminals, &handover_path).await;
        crate::logging::error_with_fields(
            "hot_reload",
            "hot reload refused; the kernel keeps running",
            serde_json::json!({ "reason": error }),
        );
    }
}

/// Returns only when the swap did not happen, with the reason.
async fn reload(terminals: &RoomTerminalService, handover_path: &std::path::Path) -> String {
    let Some(start) = START.get() else {
        return "the kernel program path is unknown".to_string();
    };
    if let Err(error) = verify(start).await {
        return error;
    }
    let handing = terminals.clone();
    let terminals_handed = match tokio::task::spawn_blocking(move || handing.hand_over()).await {
        Ok(Ok(handed)) => handed,
        Ok(Err(error)) => {
            terminals.release_handover();
            return error.to_string();
        }
        Err(error) => {
            terminals.release_handover();
            return error.to_string();
        }
    };
    let inherited: Vec<i32> = terminals_handed.iter().map(|t| t.master_fd()).collect();
    let handover = Handover {
        format: HANDOVER_FORMAT,
        terminals: terminals_handed,
    };
    if let Err(error) = write_private(handover_path, &handover) {
        terminals.release_handover();
        return format!("the handover could not be written: {error}");
    }
    crate::logging::info_with_fields(
        "hot_reload",
        "replacing the kernel program in place",
        serde_json::json!({
            "program": start.program.display().to_string(),
            "terminals": inherited.len(),
        }),
    );
    let mut command = Command::new(&start.program);
    command
        .args(&start.args)
        .env_clear()
        .envs(start.env.iter().map(|(name, value)| (name, value)))
        .env(HANDOVER_ENV, handover_path);
    let error = tokio::task::spawn_blocking(move || {
        crate::process_spawn::exec_in_place(command, inherited)
    })
    .await
    .map_or_else(|error| error.to_string(), |error| error.to_string());
    let _ = std::fs::remove_file(handover_path);
    terminals.release_handover();
    format!("the new kernel program could not start: {error}")
}

/// The new program must run here and take this handover format. Its release
/// signature and protocol were checked by the upgrade before it was installed.
async fn verify(start: &Start) -> Result<(), String> {
    let output = tokio::time::timeout(
        VERIFY_TIMEOUT,
        crate::process_spawn::async_command(&start.program)
            .arg("--handover-format")
            .env_clear()
            .kill_on_drop(true)
            .output(),
    )
    .await
    .map_err(|_| "the new kernel program did not answer in time".to_string())?
    .map_err(|error| format!("the new kernel program does not run: {error}"))?;
    let format = String::from_utf8_lossy(&output.stdout);
    if !output.status.success() || format.trim() != HANDOVER_FORMAT.to_string() {
        return Err(format!(
            "the new kernel program takes no handover of format {HANDOVER_FORMAT}"
        ));
    }
    Ok(())
}

fn write_private(path: &std::path::Path, handover: &Handover) -> std::io::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let partial = path.with_extension("partial");
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(&partial)?;
    file.write_all(&serde_json::to_vec(handover)?)?;
    file.sync_all()?;
    std::fs::rename(partial, path)
}

pub(crate) fn set_close_on_exec(fd: i32, close: bool) {
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFD) };
    if flags < 0 {
        return;
    }
    let flags = if close {
        flags | libc::FD_CLOEXEC
    } else {
        flags & !libc::FD_CLOEXEC
    };
    unsafe { libc::fcntl(fd, libc::F_SETFD, flags) };
}
