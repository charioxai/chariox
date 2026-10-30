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
pub const HANDOVER_FORMAT: u32 = 4;
const HANDOVER_ENV: &str = "CHARIOX_HANDOVER";
const VERIFY_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Debug, Default, Serialize, Deserialize)]
struct Handover {
    format: u32,
    terminals: Vec<TerminalHandover>,
    providers: Vec<ProviderHandover>,
    // A pipe holding the providers' launch credentials: they stay in memory.
    credentials_fd: Option<i32>,
    listeners: BTreeMap<String, i32>,
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
    process: HandedOverProcess,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum HandedOverProcess {
    Claude(Box<crate::provider::ClaudeHandover>),
    /// A Codex app-server keeps its PTY and port; the turn it runs continues.
    Codex {
        pty: crate::pty::PtyHandover,
        turn: crate::provider::CodexHandover,
    },
}

impl HandedOverProcess {
    fn adapter_key(&self) -> &'static str {
        match self {
            Self::Claude(_) => "claude",
            Self::Codex { .. } => "codex",
        }
    }
}

impl std::fmt::Debug for ProviderHandover {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProviderHandover")
            .field("session_id", &self.session_id)
            .field("agent_id", &self.agent_id)
            .field("run_id", &self.run_id)
            .field("process", &self.process)
            .finish_non_exhaustive()
    }
}

impl ProviderHandover {
    fn fds(&self) -> Vec<i32> {
        match &self.process {
            HandedOverProcess::Claude(claude) => claude.fds().to_vec(),
            HandedOverProcess::Codex { pty, turn } => vec![pty.master_fd, turn.connection()],
        }
    }

    fn pid(&self) -> u32 {
        match &self.process {
            HandedOverProcess::Claude(claude) => claude.pid(),
            HandedOverProcess::Codex { pty, .. } => pty.pid,
        }
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
static ADOPTING: Mutex<BTreeMap<String, HandedOverProcess>> = Mutex::new(BTreeMap::new());

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
    let mut handover = handover;
    if let Some(fd) = handover.credentials_fd {
        receive_credentials(fd, &mut handover.providers);
    }
    for fd in handover.listeners.values() {
        set_close_on_exec(*fd, true);
    }
    *lock(&INHERITED_LISTENERS) = std::mem::take(&mut handover.listeners);
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

// Listening sockets stay open across the swap: a connection that arrives
// meanwhile waits in the backlog for the new program instead of being refused.
static LISTENERS: Mutex<BTreeMap<String, i32>> = Mutex::new(BTreeMap::new());
static INHERITED_LISTENERS: Mutex<BTreeMap<String, i32>> = Mutex::new(BTreeMap::new());

/// The listener named `name`: the one the previous program listened on when
/// it has the same address, else a new one.
pub(crate) fn tcp_listener(
    name: &str,
    host: &str,
    port: u16,
) -> std::io::Result<std::net::TcpListener> {
    use std::os::fd::{AsRawFd, FromRawFd};
    let same = |listener: &std::net::TcpListener| {
        listener.local_addr().is_ok_and(|address| {
            address.port() == port
                && host
                    .parse::<std::net::IpAddr>()
                    .map_or(true, |ip| ip == address.ip())
        })
    };
    // The kept copy holds the port, so binding it again here would fail.
    let kept = lock(&LISTENERS).get(name).and_then(|fd| {
        let copy = unsafe { libc::fcntl(*fd, libc::F_DUPFD_CLOEXEC, 3) };
        // SAFETY: a new descriptor of the kept listener, owned here.
        (copy >= 0).then(|| unsafe { std::net::TcpListener::from_raw_fd(copy) })
    });
    let inherited = || {
        lock(&INHERITED_LISTENERS).remove(name).map(|fd| {
            // SAFETY: the previous program named this fd for this listener alone.
            unsafe { std::net::TcpListener::from_raw_fd(fd) }
        })
    };
    let listener = match kept.filter(same).or_else(|| inherited().filter(same)) {
        Some(listener) => listener,
        None => std::net::TcpListener::bind((host, port))?,
    };
    let kept = unsafe { libc::fcntl(listener.as_raw_fd(), libc::F_DUPFD_CLOEXEC, 3) };
    if kept >= 0 {
        if let Some(previous) = lock(&LISTENERS).insert(name.to_string(), kept) {
            unsafe { libc::close(previous) };
        }
    }
    Ok(listener)
}

fn receive_credentials(fd: i32, providers: &mut [ProviderHandover]) {
    use std::io::Read;
    use std::os::fd::FromRawFd;
    // SAFETY: the previous program named this pipe for this handover alone.
    let mut pipe = unsafe { std::fs::File::from_raw_fd(fd) };
    let mut body = zeroize::Zeroizing::new(Vec::new());
    if pipe.read_to_end(&mut body).is_err() {
        return;
    }
    let Ok(mut runs) = serde_json::from_slice::<BTreeMap<String, BTreeMap<String, String>>>(&body)
    else {
        return;
    };
    for provider in providers {
        let mut credentials = crate::provider::ProviderCredentialEnvironment::default();
        for (name, value) in runs.remove(&provider.run_id).unwrap_or_default() {
            credentials.insert(name, zeroize::Zeroizing::new(value));
        }
        if let HandedOverProcess::Claude(claude) = &mut provider.process {
            claude.set_credentials(credentials);
        }
    }
}

/// Writes the providers' credentials into a pipe the next program inherits.
fn send_credentials(providers: &[ProviderHandover]) -> std::io::Result<Option<i32>> {
    use std::io::Write;
    use std::os::fd::{FromRawFd, IntoRawFd};
    if providers.is_empty() {
        return Ok(None);
    }
    let runs: BTreeMap<&str, BTreeMap<&str, &str>> = providers
        .iter()
        .filter_map(|p| match &p.process {
            HandedOverProcess::Claude(claude) => {
                Some((p.run_id.as_str(), claude.credentials().iter().collect()))
            }
            HandedOverProcess::Codex { .. } => None,
        })
        .collect();
    let body = zeroize::Zeroizing::new(serde_json::to_vec(&runs)?);
    let mut fds = [0; 2];
    if unsafe { libc::pipe(fds.as_mut_ptr()) } != 0 {
        return Err(std::io::Error::last_os_error());
    }
    for fd in fds {
        set_close_on_exec(fd, true);
    }
    // SAFETY: both ends were just created here. The body fits the pipe buffer.
    let (read, mut write) = unsafe {
        (
            std::fs::File::from_raw_fd(fds[0]),
            std::fs::File::from_raw_fd(fds[1]),
        )
    };
    write.write_all(&body)?;
    drop(write);
    Ok(Some(read.into_raw_fd()))
}

/// The launch credentials of the agent's handed-over turn: its launch needs no
/// Vault, which may be locked in this program.
pub(crate) fn handed_over_credentials(
    session_id: &str,
    agent_id: Option<&str>,
) -> Option<crate::provider::ProviderCredentialEnvironment> {
    lock(&RECEIVED_PROVIDERS)
        .iter()
        .find(|p| p.session_id == session_id && p.agent_id.as_deref() == agent_id)
        .and_then(|p| match &p.process {
            HandedOverProcess::Claude(claude) => Some(claude.credentials().clone()),
            // The app-server keeps the login it runs with; the launch only
            // reconnects to it.
            HandedOverProcess::Codex { .. } => None,
        })
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
    let mut providers = lock(&RECEIVED_PROVIDERS);
    let index = providers.iter().position(|p| {
        p.session_id == session_id
            && p.agent_id.as_deref() == agent_id
            && p.process.adapter_key() == adapter_key
    })?;
    let provider = providers.remove(index);
    lock(&ADOPTING).insert(provider.run_id.clone(), provider.process);
    Some((provider.run_id, provider.runtime_mcp_auth_token))
}

pub(crate) fn take_claude(run_id: &str) -> Option<crate::provider::ClaudeHandover> {
    let mut adopting = lock(&ADOPTING);
    match adopting.remove(run_id)? {
        HandedOverProcess::Claude(claude) => Some(*claude),
        other => {
            adopting.insert(run_id.to_string(), other);
            None
        }
    }
}

/// The PTY of a claimed run's handed-over provider server, to adopt instead
/// of starting a new one.
pub(crate) fn adopting_pty(run_id: &str) -> Option<crate::pty::PtyHandover> {
    match lock(&ADOPTING).get(run_id)? {
        HandedOverProcess::Codex { pty, .. } => Some(pty.clone()),
        HandedOverProcess::Claude(_) => None,
    }
}

/// The address the claimed run's handed-over Codex app-server listens on.
pub(crate) fn adopting_codex_endpoint(run_id: &str) -> Option<String> {
    match lock(&ADOPTING).get(run_id)? {
        HandedOverProcess::Codex { turn, .. } => Some(turn.endpoint().to_string()),
        HandedOverProcess::Claude(_) => None,
    }
}

pub(crate) fn take_codex(run_id: &str) -> Option<crate::provider::CodexHandover> {
    let mut adopting = lock(&ADOPTING);
    match adopting.remove(run_id)? {
        HandedOverProcess::Codex { turn, .. } => Some(turn),
        other => {
            adopting.insert(run_id.to_string(), other);
            None
        }
    }
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

/// Keeps a running Claude turn for the next program.
pub(crate) fn hand_over_claude(
    run: &crate::provider::RuntimeProviderRun,
    claude: crate::provider::ClaudeHandover,
) {
    if let Some(prepared) = lock(&PREPARED).as_mut() {
        prepared.handover.providers.push(provider_handover(
            run,
            HandedOverProcess::Claude(Box::new(claude)),
        ));
    }
}

/// Offers a provider server's PTY to a running turn that may be handed over.
/// The PTY of a run whose turn is not handed over ends before the swap.
pub(crate) fn offer_provider_pty(run_id: &str, pty: crate::pty::PtyHandover) {
    if let Some(prepared) = lock(&PREPARED).as_mut() {
        prepared.offered_ptys.insert(run_id.to_string(), pty);
    }
}

pub(crate) fn provider_pty_offered(run_id: &str) -> bool {
    lock(&PREPARED)
        .as_ref()
        .is_some_and(|prepared| prepared.offered_ptys.contains_key(run_id))
}

/// Keeps a running Codex turn, with the app-server PTY offered for its run
/// (`provider_pty_offered`), for the next program.
pub(crate) fn hand_over_codex(
    run: &crate::provider::RuntimeProviderRun,
    turn: crate::provider::CodexHandover,
) {
    let mut prepared = lock(&PREPARED);
    let Some(prepared) = prepared.as_mut() else {
        return;
    };
    let Some(pty) = prepared.offered_ptys.remove(run.id()) else {
        return;
    };
    prepared.handover.providers.push(provider_handover(
        run,
        HandedOverProcess::Codex { pty, turn },
    ));
}

fn provider_handover(
    run: &crate::provider::RuntimeProviderRun,
    process: HandedOverProcess,
) -> ProviderHandover {
    ProviderHandover {
        session_id: run.session_id().to_string(),
        agent_id: run.agent_instance_id().map(str::to_string),
        run_id: run.id().to_string(),
        runtime_mcp_auth_token: run.runtime_mcp_auth_token().map(str::to_string),
        process,
    }
}

/// A prepared swap: the terminals are held and saved; the rest of the kernel
/// stops as for a planned restart before `finish` replaces the program.
struct Prepared {
    handover: Handover,
    path: PathBuf,
    offered_ptys: BTreeMap<String, crate::pty::PtyHandover>,
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
                    offered_ptys: BTreeMap::new(),
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
            credentials_fd: None,
            listeners: lock(&LISTENERS).clone(),
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
    let mut prepared = PREPARED.lock().unwrap_or_else(|e| e.into_inner()).take()?;
    let start = START.get()?;
    match send_credentials(&prepared.handover.providers) {
        Ok(fd) => prepared.handover.credentials_fd = fd,
        Err(error) => return Some(format!("the credentials could not be handed over: {error}")),
    }
    if let Err(error) = write_private(&prepared.path, &prepared.handover) {
        return Some(format!("the handover could not be written: {error}"));
    }
    // The kernel deleted the token file its supervisor wrote once it read it:
    // write it again for the new program to read the same way.
    let token_file = start
        .env
        .iter()
        .find(|(name, _)| name == crate::runtime_transport::KERNEL_LOCAL_AUTH_TOKEN_FILE_ENV)
        .map(|(_, path)| PathBuf::from(path));
    if let (Some(path), Some(token)) = (
        &token_file,
        crate::runtime_transport::configured_kernel_local_auth_token(),
    ) {
        if let Err(error) = crate::config::write_private_file(path, token.as_bytes()) {
            return Some(format!(
                "the local auth token could not be handed over: {error}"
            ));
        }
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
        .chain(prepared.handover.credentials_fd)
        .chain(prepared.handover.listeners.values().copied())
        .chain(crate::slice::hand_over_managed_docker_broker())
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
    if let Some(path) = &token_file {
        let _ = std::fs::remove_file(path);
    }
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
