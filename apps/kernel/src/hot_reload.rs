//! Hot reload: a planned upgrade replaces the kernel program in place on
//! SIGUSR2. The kernel stops as for a planned restart, except that running
//! terminals and running agent turns are handed over instead of ended; then
//! it `exec`s the installed program, keeping its PID, so their processes stay
//! its children and the new program adopts them. Everything else (clients,
//! the relay, idle provider processes) comes back as after any restart.

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::path::PathBuf;
use std::process::Command;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::room_terminal::{RoomTerminalService, TerminalHandover};

/// The handover this program writes and reads. A new program must print the
/// same number for `--handover-format` before this one hands over to it.
pub const HANDOVER_FORMAT: u32 = 2;
const HANDOVER_ENV: &str = "CHARIOX_HANDOVER";
const VERIFY_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Debug, Default, Serialize, Deserialize)]
struct Handover {
    format: u32,
    terminals: Vec<TerminalHandover>,
    providers: Vec<ProviderHandover>,
}

/// A provider run whose turn was running: the next program adopts it as the
/// same run of the same agent, and the recovered prompt continues it.
#[derive(Serialize, Deserialize)]
pub(crate) struct ProviderHandover {
    session_id: String,
    agent_id: Option<String>,
    run_id: String,
    // The running process authenticates its kernel tool calls with it.
    runtime_mcp_auth_token: Option<String>,
    claude: crate::provider::ClaudeHandover,
}

impl std::fmt::Debug for ProviderHandover {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProviderHandover")
            .field("session_id", &self.session_id)
            .field("agent_id", &self.agent_id)
            .field("run_id", &self.run_id)
            .field("claude", &self.claude)
            .finish_non_exhaustive()
    }
}

impl ProviderHandover {
    fn fds(&self) -> [i32; 3] {
        self.claude.fds()
    }

    fn pid(&self) -> u32 {
        self.claude.pid()
    }
}

/// How this program was started, to start its successor the same way: the
/// kernel removes secrets from its environment once it has read them.
struct Start {
    program: PathBuf,
    args: Vec<OsString>,
    env: Vec<(OsString, OsString)>,
}

static START: OnceLock<Start> = OnceLock::new();
static RECEIVED_TERMINALS: Mutex<Vec<TerminalHandover>> = Mutex::new(Vec::new());
static RECEIVED_PROVIDERS: Mutex<Vec<ProviderHandover>> = Mutex::new(Vec::new());
// Claimed by a new run of the same agent, until its runtime adopts the process.
static ADOPTING: Mutex<BTreeMap<String, crate::provider::ClaudeHandover>> =
    Mutex::new(BTreeMap::new());

fn lock<T>(slot: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    slot.lock().unwrap_or_else(|e| e.into_inner())
}

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
    for provider in &handover.providers {
        for fd in provider.fds() {
            set_close_on_exec(fd, true);
        }
    }
    crate::logging::info_with_fields(
        "hot_reload.receive",
        "kernel program replaced in place",
        serde_json::json!({
            "terminals": handover.terminals.len(),
            "provider_runs": handover.providers.iter().map(|p| &p.run_id).collect::<Vec<_>>(),
        }),
    );
    *lock(&RECEIVED_TERMINALS) = handover.terminals;
    *lock(&RECEIVED_PROVIDERS) = handover.providers;
}

pub(crate) fn take_terminals() -> Vec<TerminalHandover> {
    std::mem::take(&mut *lock(&RECEIVED_TERMINALS))
}

/// Whether the agent's turn kept running across the hot reload.
pub(crate) fn provider_handed_over(session_id: &str, agent_id: &str) -> bool {
    lock(&RECEIVED_PROVIDERS)
        .iter()
        .any(|p| p.session_id == session_id && p.agent_id.as_deref() == Some(agent_id))
}

/// A new run of the agent takes the handed-over run's id and tool token, and
/// its runtime adopts the process instead of starting one.
pub(crate) fn claim_provider_run(
    session_id: &str,
    agent_id: Option<&str>,
    adapter_key: &str,
) -> Option<(String, Option<String>)> {
    if adapter_key != "claude" {
        return None;
    }
    let mut providers = lock(&RECEIVED_PROVIDERS);
    let index = providers
        .iter()
        .position(|p| p.session_id == session_id && p.agent_id.as_deref() == agent_id)?;
    let provider = providers.remove(index);
    lock(&ADOPTING).insert(provider.run_id.clone(), provider.claude);
    Some((provider.run_id, provider.runtime_mcp_auth_token))
}

pub(crate) fn take_claude(run_id: &str) -> Option<crate::provider::ClaudeHandover> {
    lock(&ADOPTING).remove(run_id)
}

/// New runs must not take a handed-over run's id.
pub(crate) fn is_handed_over_run(run_id: &str) -> bool {
    lock(&RECEIVED_PROVIDERS).iter().any(|p| p.run_id == run_id)
        || lock(&ADOPTING).contains_key(run_id)
}

/// Ends handed-over turns no recovered prompt claimed, as a restart would have.
pub(crate) fn end_unclaimed_providers() {
    let unclaimed = std::mem::take(&mut *lock(&RECEIVED_PROVIDERS));
    for provider in unclaimed {
        crate::logging::warn_with_fields(
            "hot_reload",
            "a handed-over agent turn was not resumed; ending it as after a restart",
            serde_json::json!({ "provider_run_id": provider.run_id }),
        );
        end_child(provider.pid());
        for fd in provider.fds() {
            unsafe { libc::close(fd) };
        }
    }
}

/// Ends and reaps one child tree of this process.
fn end_child(pid: u32) {
    if pid <= 1 {
        return;
    }
    crate::runtime::process_health::terminate_process_tree(pid);
    for _ in 0..50 {
        let mut status = 0;
        if unsafe { libc::waitpid(pid as libc::pid_t, &mut status, libc::WNOHANG) } != 0 {
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// While the kernel stops for a prepared hot reload.
pub(crate) fn handing_over() -> bool {
    lock(&PREPARED).is_some()
}

/// Keeps a running provider turn for the next program.
pub(crate) fn hand_over_provider(
    run: &crate::provider::RuntimeProviderRun,
    claude: crate::provider::ClaudeHandover,
) {
    let provider = ProviderHandover {
        session_id: run.session_id().to_string(),
        agent_id: run.agent_instance_id().map(str::to_string),
        run_id: run.id().to_string(),
        runtime_mcp_auth_token: run.runtime_mcp_auth_token().map(str::to_string),
        claude,
    };
    if let Some(prepared) = lock(&PREPARED).as_mut() {
        prepared.handover.providers.push(provider);
    }
}

/// A prepared swap: the terminals are held and saved; the rest of the kernel
/// stops as for a planned restart before `finish` replaces the program.
struct Prepared {
    handover: Handover,
    path: PathBuf,
}

static PREPARED: Mutex<Option<Prepared>> = Mutex::new(None);

/// Completes when a SIGUSR2 hot reload is prepared. A refused request
/// changes nothing: the kernel keeps running and waits for the next one.
pub async fn requested(terminals: RoomTerminalService, handover_path: PathBuf) {
    use tokio::signal::unix::{signal, SignalKind};
    let mut requests = match signal(SignalKind::user_defined2()) {
        Ok(requests) => requests,
        Err(error) => {
            crate::logging::warn_with_fields(
                "hot_reload",
                "failed to register the SIGUSR2 handler; hot reload is unavailable",
                serde_json::json!({ "error": error.to_string() }),
            );
            return std::future::pending().await;
        }
    };
    while requests.recv().await.is_some() {
        match prepare(&terminals).await {
            Ok(handover) => {
                *PREPARED.lock().unwrap_or_else(|e| e.into_inner()) = Some(Prepared {
                    handover,
                    path: handover_path,
                });
                return;
            }
            Err(reason) => crate::logging::error_with_fields(
                "hot_reload",
                "hot reload refused; the kernel keeps running",
                serde_json::json!({ "reason": reason }),
            ),
        }
    }
    std::future::pending().await
}

async fn prepare(terminals: &RoomTerminalService) -> Result<Handover, String> {
    let start = START.get().ok_or("the kernel program path is unknown")?;
    verify(start).await?;
    let handing = terminals.clone();
    match tokio::task::spawn_blocking(move || handing.hand_over()).await {
        Ok(Ok(terminals)) => Ok(Handover {
            format: HANDOVER_FORMAT,
            terminals,
            providers: Vec::new(),
        }),
        Ok(Err(error)) => {
            terminals.release_handover();
            Err(error.to_string())
        }
        Err(error) => {
            terminals.release_handover();
            Err(error.to_string())
        }
    }
}

/// After the kernel stopped for a prepared hot reload: replaces this program
/// with the installed one, handing over the terminals. Returns `None` when no
/// hot reload was prepared, or why the new program could not start.
pub fn finish() -> Option<String> {
    let prepared = PREPARED.lock().unwrap_or_else(|e| e.into_inner()).take()?;
    let start = START.get()?;
    if let Err(error) = write_private(&prepared.path, &prepared.handover) {
        return Some(format!("the handover could not be written: {error}"));
    }
    let inherited: Vec<i32> = prepared
        .handover
        .terminals
        .iter()
        .map(TerminalHandover::master_fd)
        .chain(
            prepared
                .handover
                .providers
                .iter()
                .flat_map(ProviderHandover::fds),
        )
        .collect();
    crate::logging::info_with_fields(
        "hot_reload",
        "replacing the kernel program in place",
        serde_json::json!({
            "program": start.program.display().to_string(),
            "terminals": prepared.handover.terminals.len(),
            "provider_runs": prepared.handover.providers.len(),
        }),
    );
    let kept: Vec<u32> = prepared
        .handover
        .terminals
        .iter()
        .map(TerminalHandover::pid)
        .chain(
            prepared
                .handover
                .providers
                .iter()
                .map(ProviderHandover::pid),
        )
        .collect();
    end_children_except(&kept);
    let mut command = Command::new(&start.program);
    command
        .args(&start.args)
        .env_clear()
        .envs(start.env.iter().map(|(name, value)| (name, value)))
        .env(HANDOVER_ENV, &prepared.path);
    let error = crate::process_spawn::exec_in_place(command, inherited);
    let _ = std::fs::remove_file(&prepared.path);
    Some(error.to_string())
}

/// A planned restart leaves what the kernel did not stop to its service
/// manager (`KillMode=mixed` ends the rest of the unit). The exec keeps the
/// PID, so the kernel ends and reaps those children itself.
fn end_children_except(kept: &[u32]) {
    let own = std::process::id();
    let lister = Command::new("ps")
        .args(["-A", "-o", "pid=", "-o", "ppid="])
        .stdout(std::process::Stdio::piped())
        .spawn();
    // `ps` lists itself as a child too.
    let lister_pid = lister.as_ref().map_or(0, std::process::Child::id);
    let listed = lister.and_then(std::process::Child::wait_with_output);
    let Ok(listed) = listed else {
        crate::logging::warn_with_fields(
            "hot_reload",
            "child processes could not be listed; leftovers keep running",
            serde_json::json!({}),
        );
        return;
    };
    let children: Vec<u32> = String::from_utf8_lossy(&listed.stdout)
        .lines()
        .filter_map(|line| {
            let mut fields = line.split_whitespace();
            let pid = fields.next()?.parse::<u32>().ok()?;
            let parent = fields.next()?.parse::<u32>().ok()?;
            (parent == own && pid > 1 && pid != lister_pid && !kept.contains(&pid)).then_some(pid)
        })
        .collect();
    for pid in &children {
        end_child(*pid);
    }
    if !children.is_empty() {
        crate::logging::info_with_fields(
            "hot_reload",
            "ended the children a planned restart leaves to the service manager",
            serde_json::json!({ "processes": children.len() }),
        );
    }
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
