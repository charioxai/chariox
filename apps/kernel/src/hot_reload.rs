//! Hot reload: a planned upgrade replaces the kernel program in place on
//! SIGUSR2. The kernel stops as for a planned restart, except that running
//! terminals and running agent turns are handed over instead of ended, and so
//! are the jobs that ended workloads left running (they stay their owners').
//! Then it `exec`s the installed program, keeping its PID, so their processes
//! stay its children and the new program adopts them. Everything else
//! (clients, the relay, idle provider processes) comes back as after any
//! restart.

use crate::config::ListenPort;
use crate::runtime::bounded_process::AsyncBoundedCommandExt as _;
use std::collections::BTreeMap;
use std::ffi::OsString;
use std::os::fd::{AsRawFd as _, FromRawFd as _, OwnedFd};
use std::path::PathBuf;
use std::process::Command;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::room_terminal::{RoomTerminalService, TerminalHandover};
use crate::runtime::state::JobHandover;

mod job_state_file;
mod read;

use read::{end_unreadable, read_handover, salvage_document, ReadHandover};

/// The handover this program writes and reads. A new program must print this
/// number, or the next one, for `--handover-format` before this one hands
/// over to it.
/// This includes the persisted terminal records used to restore the handed-over PTYs;
/// TerminalScope records cannot restore the earlier session-only records.
///
/// A change an older program cannot read bumps this number by one, and the
/// program that bumps it keeps reading the documents of the release before
/// (`PREVIOUS_HANDOVER_FORMAT`). A field added to a handover shape needs no
/// bump when it has a serde default. `handover_compatibility` in the tests
/// reads the previous release's documents and fails on either slip.
pub const HANDOVER_FORMAT: u32 = 18;
/// The format of the documents the previous release writes, which this one still reads.
pub const PREVIOUS_HANDOVER_FORMAT: u32 = 17;
const HANDOVER_ENV: &str = "CHARIOX_HANDOVER";
const VERIFY_TIMEOUT: Duration = Duration::from_secs(10);

// A part a release did not write is the part's default: the handover of the previous release
// is read whole (the lists and the user-domain access are read separately, part by part).
#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(default)]
struct Handover {
    format: u32,
    terminals: Vec<TerminalHandover>,
    providers: Vec<ProviderHandover>,
    attachments: Vec<AttachmentHandover>,
    // Jobs that workloads which ended left running stay their owners' (agent or
    // Room terminal): the next program watches them again.
    released_jobs: Vec<crate::runtime::process_resources::ReleasedJobs>,
    // Processes agents watch keep their wakes: the next program supervises the
    // same processes again, from the pipes handed over with them.
    watched: Vec<crate::runtime::state::WatchedHandover>,
    // Local jobs and remote jobs whose output a reader follows. The key keeps its name so the
    // previous release's documents (local jobs only) read whole.
    local_jobs: Vec<JobHandover>,
    // Commands whose admission stopped them for room stay stopped, listed and
    // startable: the next program's admission is told which they are.
    admission: Vec<crate::runtime::process_resources::HeldCommand>,
    // Focus and grants list agents' user-domain tools: a provider that listed
    // them must not see its tools change.
    user_domain: crate::runtime::kernel_browser_host::LoadedHandover,
    // Provider runs the owner cut off from the gateway: a handed-over run keeps its
    // id and tool token, so the revocation has to survive the swap with it.
    revoked_reach_runs: Vec<String>,
    // A pipe holding the providers' launch credentials: they stay in memory.
    credentials_fd: Option<i32>,
    // A memory file holding the unlocked Vault keys, so a program update does
    // not lock the Vault again. Keys never reach the handover file.
    vault_fd: Option<i32>,
    listeners: BTreeMap<String, i32>,
}

impl Handover {
    fn with_terminals(terminals: Vec<TerminalHandover>) -> Self {
        Self {
            format: HANDOVER_FORMAT,
            terminals,
            providers: Vec::new(),
            attachments: Vec::new(),
            released_jobs: Vec::new(),
            watched: Vec::new(),
            local_jobs: Vec::new(),
            admission: Vec::new(),
            user_domain: Default::default(),
            revoked_reach_runs: Vec::new(),
            credentials_fd: None,
            vault_fd: None,
            listeners: lock(&LISTENERS).clone(),
        }
    }

    /// The descriptors the handover names, which the next program inherits.
    fn inherited_fds(&self) -> Vec<i32> {
        self.terminals
            .iter()
            .map(TerminalHandover::master_fd)
            .chain(self.providers.iter().flat_map(ProviderHandover::fds))
            .chain(
                self.watched
                    .iter()
                    .flat_map(|watched| watched.fds().iter().copied()),
            )
            .chain(self.local_jobs.iter().flat_map(JobHandover::fds))
            .chain(self.credentials_fd)
            .chain(self.vault_fd)
            .chain(self.listeners.values().copied())
            .collect()
    }
}

/// A client's attachment to a session: the client resumes its subscription on
/// it after the swap. (`RuntimeAttachment` leaves its owner out of what it
/// serializes for clients.)
#[derive(Debug, Serialize, Deserialize)]
struct AttachmentHandover {
    id: String,
    session_id: String,
    client_id: String,
    capability_level: crate::attachment::ClientCapabilityLevel,
    owner_user_id: String,
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
    // The run as far as tool calls need it, until a recovered prompt claims the process: the
    // surviving process keeps calling its tools with the token it has (Vault, timers, events).
    // A program older than this field hands none over, and its runs authenticate at the claim.
    #[serde(default)]
    identity: Option<RunIdentity>,
    process: HandedOverProcess,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum HandedOverProcess {
    Claude(Box<crate::provider::ClaudeHandover>),
    /// A Codex app-server keeps its PTY and port; the turn it runs continues.
    Codex {
        pty: crate::pty::PtyHandover,
        turn: Box<crate::provider::CodexHandover>,
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

/// A handed-over run as its tool calls read it. The run's own serialization leaves out what is
/// kept in memory only; the runtime state tool calls read is carried beside it: whether workflow
/// actions are offered (`workflow_tools_enabled`) and the workflow node whose fresh context the run
/// holds. The other runtime-only fields (resolved account, discovery and substitute markers,
/// settings staleness, sync roots, project environment revision, native checkpoint, last output
/// time) are read at launch, prompt dispatch, settlement and reload, which the claiming run does.
#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct RunIdentity {
    run: crate::provider::RuntimeProviderRun,
    #[serde(default)]
    workflow_tools_enabled: bool,
    #[serde(default)]
    workflow_fresh_context_node_run_id: Option<String>,
}

impl RunIdentity {
    pub(crate) fn of(run: &crate::provider::RuntimeProviderRun) -> Self {
        Self {
            run: run.handover_identity(),
            workflow_tools_enabled: run.workflow_tools_enabled(),
            workflow_fresh_context_node_run_id: run
                .workflow_fresh_context_node_run_id()
                .map(str::to_string),
        }
    }

    pub(crate) fn into_run(self) -> crate::provider::RuntimeProviderRun {
        let mut run = self.run;
        if self.workflow_tools_enabled {
            run.enable_workflow_tools();
        }
        if let Some(node) = self.workflow_fresh_context_node_run_id {
            run.mark_workflow_fresh_context(node);
        }
        run
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
static RECEIVED_ADMISSION: Mutex<Vec<crate::runtime::process_resources::HeldCommand>> =
    Mutex::new(Vec::new());
static RECEIVED_JOBS: Mutex<Vec<crate::runtime::process_resources::ReleasedJobs>> =
    Mutex::new(Vec::new());
static RECEIVED_WATCHED: Mutex<Vec<crate::runtime::state::WatchedHandover>> =
    Mutex::new(Vec::new());
static RECEIVED_JOB_READERS: Mutex<Vec<JobHandover>> = Mutex::new(Vec::new());
static RECEIVED_TERMINALS: Mutex<Vec<TerminalHandover>> = Mutex::new(Vec::new());
static RECEIVED_PROVIDERS: Mutex<Vec<ProviderHandover>> = Mutex::new(Vec::new());
// A retained goal approval remains authoritative while its process and reply
// channel are being adopted, even after its run has been claimed.
static RETAINED_GOAL_APPROVALS: Mutex<BTreeMap<String, crate::provider::NativeApprovalGoal>> =
    Mutex::new(BTreeMap::new());
static RECEIVED_ATTACHMENTS: Mutex<Vec<AttachmentHandover>> = Mutex::new(Vec::new());
static RECEIVED_USER_DOMAIN: Mutex<Option<crate::runtime::kernel_browser_host::LoadedHandover>> =
    Mutex::new(None);
static RECEIVED_REVOKED_REACH_RUNS: Mutex<Vec<String>> = Mutex::new(Vec::new());
// The handed-over runs' tool identities, from the handover until a recovered prompt claims the run:
// their processes are alive and keep calling the kernel's tools with the token they have.
static UNCLAIMED_RUNS: Mutex<BTreeMap<String, crate::provider::RuntimeProviderRun>> =
    Mutex::new(BTreeMap::new());
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
    let body = body.map_err(|error| error.to_string());
    let read = body
        .as_deref()
        .map_err(Clone::clone)
        .and_then(read_handover);
    let ReadHandover {
        handover,
        unreadable,
    } = match read {
        Ok(read) => read,
        Err(error) => {
            // The old program's listening sockets are open in this one whatever the
            // document says: keep serving on them, or the ports stay taken.
            if let Ok(body) = &body {
                keep_inherited_listeners(body);
                // Exec kept the previous program's children and descriptors whatever the
                // document says: what it names ends as after a restart, and only the
                // listeners stay open.
                let listeners: Vec<i32> = lock(&INHERITED_LISTENERS).values().copied().collect();
                end_unreadable(salvage_document(body), &listeners);
            }
            note_for_log(
                true,
                "the handover could not be read; running terminals end as after a restart",
                serde_json::json!({ "error": error }),
            );
            return;
        }
    };
    end_unreadable(unreadable, &handover.inherited_fds());
    // Children started from here on must not inherit them.
    for terminal in &handover.terminals {
        set_close_on_exec(terminal.master_fd(), true);
    }
    for provider in &handover.providers {
        for fd in provider.fds() {
            set_close_on_exec(fd, true);
        }
    }
    for watched in &handover.watched {
        for fd in watched.fds() {
            set_close_on_exec(*fd, true);
        }
    }
    for fd in handover.local_jobs.iter().flat_map(JobHandover::fds) {
        set_close_on_exec(fd, true);
    }
    let mut handover = handover;
    if let Some(fd) = handover.credentials_fd {
        receive_credentials(fd, &mut handover.providers);
    }
    if let Some(fd) = handover.vault_fd {
        receive_vault_unlocks(fd);
    }
    for fd in handover.listeners.values() {
        set_close_on_exec(*fd, true);
    }
    *lock(&INHERITED_LISTENERS) = std::mem::take(&mut handover.listeners);
    // Reading bootstrap environment must not start sampler/watchdog workers.
    // Activate these only after startup finishes consuming the environment.
    *lock(&RECEIVED_ADMISSION) = std::mem::take(&mut handover.admission);
    *lock(&RECEIVED_JOBS) = std::mem::take(&mut handover.released_jobs);
    *lock(&RECEIVED_WATCHED) = std::mem::take(&mut handover.watched);
    *lock(&RECEIVED_JOB_READERS) = std::mem::take(&mut handover.local_jobs);
    note_for_log(
        false,
        "kernel program replaced in place",
        serde_json::json!({
            "terminals": handover.terminals.len(),
            "provider_runs": handover.providers.iter().map(|p| &p.run_id).collect::<Vec<_>>(),
            "attachments": handover.attachments.len(),
        }),
    );
    *lock(&UNCLAIMED_RUNS) = handover
        .providers
        .iter_mut()
        .filter_map(|p| {
            p.identity
                .take()
                .map(|identity| (p.run_id.clone(), identity.into_run()))
        })
        .collect();
    *lock(&RECEIVED_TERMINALS) = handover.terminals;
    *lock(&RETAINED_GOAL_APPROVALS) = handover
        .providers
        .iter()
        .filter_map(|provider| {
            let HandedOverProcess::Codex { turn, .. } = &provider.process else {
                return None;
            };
            Some((
                provider.run_id.clone(),
                turn.pending_goal_approval()?.clone(),
            ))
        })
        .collect();
    *lock(&RECEIVED_PROVIDERS) = handover.providers;
    *lock(&RECEIVED_ATTACHMENTS) = handover.attachments;
    *lock(&RECEIVED_USER_DOMAIN) = Some(handover.user_domain);
    *lock(&RECEIVED_REVOKED_REACH_RUNS) = handover.revoked_reach_runs;
}

// What `receive` has to say: the logger starts after it, so it is written once the logger is up.
static RECEIVE_NOTES: Mutex<Vec<(bool, &'static str, serde_json::Value)>> = Mutex::new(Vec::new());

pub(crate) fn note_for_log(error: bool, message: &'static str, fields: serde_json::Value) {
    lock(&RECEIVE_NOTES).push((error, message, fields));
}

fn write_received_notes() {
    for (error, message, fields) in std::mem::take(&mut *lock(&RECEIVE_NOTES)) {
        if error {
            crate::logging::error_with_fields("hot_reload.receive", message, fields);
        } else {
            crate::logging::info_with_fields("hot_reload.receive", message, fields);
        }
    }
}

/// The listening sockets a document that could not be read still names, as far as it can be
/// parsed at all.
fn keep_inherited_listeners(body: &[u8]) {
    let listeners = serde_json::from_slice::<serde_json::Value>(body)
        .ok()
        .and_then(|document| document.get("listeners").cloned())
        .and_then(|listeners| serde_json::from_value::<BTreeMap<String, i32>>(listeners).ok());
    let Some(listeners) = listeners else {
        return;
    };
    for fd in listeners.values() {
        set_close_on_exec(*fd, true);
    }
    *lock(&INHERITED_LISTENERS) = listeners;
}

/// Start inherited watches after environment bootstrap, inside the runtime.
pub fn adopt_received_processes() {
    write_received_notes();
    let held = std::mem::take(&mut *lock(&RECEIVED_ADMISSION));
    let jobs = std::mem::take(&mut *lock(&RECEIVED_JOBS));
    // Admission must precede the first watch sample.
    crate::runtime::process_resources::adopt_admission(held);
    crate::runtime::process_resources::adopt_released(jobs);
}

/// The watched processes the previous program handed over, once.
pub(crate) fn take_watched() -> Vec<crate::runtime::state::WatchedHandover> {
    std::mem::take(&mut *lock(&RECEIVED_WATCHED))
}

pub(crate) fn take_terminals() -> Vec<TerminalHandover> {
    std::mem::take(&mut *lock(&RECEIVED_TERMINALS))
}

pub(crate) fn take_attachments() -> Vec<crate::attachment::RuntimeAttachment> {
    std::mem::take(&mut *lock(&RECEIVED_ATTACHMENTS))
        .into_iter()
        .map(|a| {
            crate::attachment::RuntimeAttachment::new(
                a.id,
                a.session_id,
                a.client_id,
                a.capability_level,
                a.owner_user_id,
            )
        })
        .collect()
}

pub(crate) fn has_received_handover() -> bool {
    lock(&RECEIVED_USER_DOMAIN).is_some()
}

pub(crate) fn take_user_domain() -> Option<crate::runtime::kernel_browser_host::LoadedHandover> {
    lock(&RECEIVED_USER_DOMAIN).take()
}

pub(crate) fn take_revoked_reach_runs() -> std::collections::HashSet<String> {
    std::mem::take(&mut *lock(&RECEIVED_REVOKED_REACH_RUNS))
        .into_iter()
        .collect()
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
    port: ListenPort,
) -> std::io::Result<std::net::TcpListener> {
    use std::os::fd::{AsRawFd, FromRawFd};
    let same_host = |listener: &std::net::TcpListener| {
        listener.local_addr().is_ok_and(|address| {
            host.parse::<std::net::IpAddr>()
                .map_or(true, |ip| ip == address.ip())
        })
    };
    let same = |listener: &std::net::TcpListener| match port {
        ListenPort::Exact(port) => {
            same_host(listener) && listener.local_addr().is_ok_and(|a| a.port() == port)
        }
        // Providers of the previous program hold the port it got, whichever it is.
        ListenPort::PreferredElseAny(_) => same_host(listener),
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
    // A kept listener answers only for the port it was bound to: with a
    // preferred port, a second bind in this process is another endpoint.
    let kept = kept.filter(|listener| matches!(port, ListenPort::Exact(_)) && same(listener));
    let listener = match kept.or_else(|| inherited().filter(same)) {
        Some(listener) => listener,
        None => match port {
            ListenPort::Exact(port) => std::net::TcpListener::bind((host, port))?,
            ListenPort::PreferredElseAny(port) => match std::net::TcpListener::bind((host, port)) {
                Err(error) if error.kind() == std::io::ErrorKind::AddrInUse => {
                    std::net::TcpListener::bind((host, 0))?
                }
                bound => bound?,
            },
        },
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
    // SAFETY: the previous program named this file for this handover alone.
    let mut file = unsafe { std::fs::File::from_raw_fd(fd) };
    let mut body = zeroize::Zeroizing::new(Vec::new());
    if file.read_to_end(&mut body).is_err() {
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

/// Reads the Vault unlocks the previous program left and closes the
/// descriptor: it is read once. A Vault that cannot be restored stays locked.
fn receive_vault_unlocks(fd: i32) {
    use std::io::Read;
    use std::os::fd::FromRawFd;
    // SAFETY: the previous program named this file for this handover alone.
    let mut file = unsafe { std::fs::File::from_raw_fd(fd) };
    let mut body = zeroize::Zeroizing::new(Vec::new());
    let restored = match file.read_to_end(&mut body) {
        Ok(_) => crate::secret::adopt_vault_unlocks(&body)
            .map(drop)
            .map_err(|error| error.to_string()),
        Err(error) => Err(error.to_string()),
    };
    if let Err(error) = restored {
        crate::logging::error_with_fields(
            "hot_reload.receive",
            "the Vault unlocks could not be restored; the Vault stays locked",
            serde_json::json!({ "error": error }),
        );
    }
}

/// Hands the Vault's unlocked keys to the next program in a descriptor it
/// inherits, or none when the Vault is locked.
fn send_vault_unlocks() -> std::io::Result<Option<i32>> {
    let body = crate::secret::hand_over_vault_unlocks().map_err(std::io::Error::other)?;
    if body.is_empty() {
        return Ok(None);
    }
    memory_file_for_next_program(c"chariox-handover-vault", &body).map(Some)
}

/// Hands the providers' credentials to the next program in a descriptor it
/// inherits. Nothing reads it before `exec`, so writing must never wait for a
/// reader.
fn send_credentials(providers: &[ProviderHandover]) -> std::io::Result<Option<i32>> {
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
    memory_file_for_next_program(c"chariox-handover-credentials", &body).map(Some)
}

/// A file only in memory, of any size, read from its start. It carries keys
/// and credentials: it is never a path on disk.
#[cfg(target_os = "linux")]
fn memory_file_for_next_program(name: &std::ffi::CStr, body: &[u8]) -> std::io::Result<i32> {
    use std::io::{Seek, Write};
    use std::os::fd::{FromRawFd, IntoRawFd};
    let fd = unsafe { libc::memfd_create(name.as_ptr(), libc::MFD_CLOEXEC) };
    if fd < 0 {
        return Err(std::io::Error::last_os_error());
    }
    // SAFETY: the descriptor was just created here.
    let mut file = unsafe { std::fs::File::from_raw_fd(fd) };
    file.write_all(body)?;
    file.rewind()?;
    Ok(file.into_raw_fd())
}

/// Without memory-only files: a pipe, which must hold the whole body since
/// nothing reads it before `exec`. A larger body fails the hot reload instead
/// of waiting for ever.
#[cfg(not(target_os = "linux"))]
fn memory_file_for_next_program(_name: &std::ffi::CStr, body: &[u8]) -> std::io::Result<i32> {
    use std::io::Write;
    use std::os::fd::{FromRawFd, IntoRawFd};
    const PIPE_CAPACITY: usize = 16 << 10;
    if body.len() > PIPE_CAPACITY {
        return Err(std::io::Error::other(format!(
            "{} bytes for the next program do not fit a pipe",
            body.len()
        )));
    }
    let mut fds = [0; 2];
    if unsafe { libc::pipe(fds.as_mut_ptr()) } != 0 {
        return Err(std::io::Error::last_os_error());
    }
    for fd in fds {
        set_close_on_exec(fd, true);
    }
    // SAFETY: both ends were just created here.
    let (read, mut write) = unsafe {
        (
            std::fs::File::from_raw_fd(fds[0]),
            std::fs::File::from_raw_fd(fds[1]),
        )
    };
    write.write_all(body)?;
    drop(write);
    Ok(read.into_raw_fd())
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

pub(crate) fn handed_over_goal_approval(
    session_id: &str,
    agent_id: &str,
) -> Option<(String, crate::provider::NativeApprovalGoal)> {
    let providers = lock(&RECEIVED_PROVIDERS);
    let provider = providers.iter().find(|provider| {
        provider.session_id == session_id && provider.agent_id.as_deref() == Some(agent_id)
    })?;
    let HandedOverProcess::Codex { turn, .. } = &provider.process else {
        return None;
    };
    Some((
        provider.run_id.clone(),
        turn.pending_goal_approval()?.clone(),
    ))
}

pub(crate) fn retained_goal_approval(run_id: &str) -> Option<crate::provider::NativeApprovalGoal> {
    lock(&RETAINED_GOAL_APPROVALS).get(run_id).cloned()
}

pub(crate) fn release_retained_goal_approval(run_id: &str) {
    lock(&RETAINED_GOAL_APPROVALS).remove(run_id);
}

pub(crate) fn handed_over_run_id(
    session_id: &str,
    agent_id: Option<&str>,
    adapter_key: &str,
) -> Option<String> {
    lock(&RECEIVED_PROVIDERS)
        .iter()
        .find(|p| {
            p.session_id == session_id
                && p.agent_id.as_deref() == agent_id
                && p.process.adapter_key() == adapter_key
        })
        .map(|p| p.run_id.clone())
}

/// The handed-over runs no recovered prompt has claimed yet that hold this tool token: their
/// processes are alive and keep calling the kernel's tools, so the token must keep working.
pub(crate) fn unclaimed_runs_for_token(token: &str) -> Vec<crate::provider::RuntimeProviderRun> {
    lock(&UNCLAIMED_RUNS)
        .values()
        .filter(|run| run.runtime_mcp_auth_token() == Some(token))
        .cloned()
        .collect()
}

/// The handed-over run of an agent that no recovered prompt has claimed yet: until then it is the
/// agent's current run, the one its event tools (timers, waits, inbox) name.
pub(crate) fn unclaimed_run_for_agent(
    session_id: &str,
    agent_id: &str,
) -> Option<crate::provider::RuntimeProviderRun> {
    lock(&UNCLAIMED_RUNS)
        .values()
        .find(|run| run.session_id() == session_id && run.agent_instance_id() == Some(agent_id))
        .cloned()
}

/// The run as the next program reads it from the handover: through the serialized form.
#[cfg(test)]
pub(crate) fn run_after_handover_for_tests(
    run: &crate::provider::RuntimeProviderRun,
) -> crate::provider::RuntimeProviderRun {
    let body = serde_json::to_vec(&RunIdentity::of(run)).unwrap();
    serde_json::from_slice::<RunIdentity>(&body)
        .unwrap()
        .into_run()
}

#[cfg(test)]
pub(crate) fn hold_unclaimed_run_for_tests(run: crate::provider::RuntimeProviderRun) {
    lock(&UNCLAIMED_RUNS).insert(run.id().to_string(), run);
}

#[cfg(test)]
pub(crate) fn forget_unclaimed_run_for_test(run_id: &str) {
    lock(&UNCLAIMED_RUNS).remove(run_id);
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
    lock(&UNCLAIMED_RUNS).remove(&provider.run_id);
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

/// A handed-over Claude turn this program adopts for `run_id`, as after a claim.
#[cfg(test)]
pub(crate) fn adopt_claude_for_tests(run_id: &str, claude: crate::provider::ClaudeHandover) {
    lock(&ADOPTING).insert(
        run_id.to_string(),
        HandedOverProcess::Claude(Box::new(claude)),
    );
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
        HandedOverProcess::Codex { turn, .. } => Some(*turn),
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
    lock(&UNCLAIMED_RUNS).clear();
    for provider in unclaimed {
        release_retained_goal_approval(&provider.run_id);
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

/// Ends and reaps one child tree of this process, if `pid` is one of its
/// children: a number from a document that could not be read names nothing else.
fn end_own_child(pid: u32) {
    // SAFETY: a non-blocking wait on a pid only reports on, or reaps, a child.
    let mut status = 0;
    let waited = unsafe { libc::waitpid(pid as libc::pid_t, &mut status, libc::WNOHANG) };
    if waited == 0 {
        end_child(pid);
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

/// A prepared hot reload (with nothing to hand over) until dropped.
#[cfg(test)]
pub(crate) struct PreparedForTests;

#[cfg(test)]
impl PreparedForTests {
    pub(crate) fn new() -> Self {
        *lock(&PREPARED) = Some(Prepared {
            handover: Handover::default(),
            masters: Vec::new(),
            watched_resources: Vec::new(),
            kernel: None,
            path: PathBuf::new(),
            offered_ptys: BTreeMap::new(),
            local_job_guards: Vec::new(),
        });
        Self
    }

    /// A swap prepared as on a SIGUSR2, with the running terminals of `terminals`.
    pub(crate) fn with_terminals(terminals: &RoomTerminalService) -> Self {
        let terminals = terminals
            .hand_over()
            .expect("the terminals are handed over");
        *lock(&PREPARED) = Some(Prepared::new(terminals, PathBuf::new()));
        Self
    }

    /// The descriptors the swap names for the next program to inherit.
    pub(crate) fn inherited_fds(&self) -> Vec<i32> {
        lock(&PREPARED)
            .as_ref()
            .expect("a prepared swap")
            .handover
            .inherited_fds()
    }
}

#[cfg(test)]
impl Drop for PreparedForTests {
    fn drop(&mut self) {
        *lock(&PREPARED) = None;
    }
}

/// Takes the stopped kernel's state when its run returns, before `finish`.
/// A prepared swap keeps it until `exec`: the PTY processes it hands over
/// would end with it.
pub(crate) fn keep_until_exec(kernel: Box<dyn std::any::Any + Send>) {
    if let Some(prepared) = lock(&PREPARED).as_mut() {
        prepared.kernel = Some(kernel);
    }
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

/// Keeps the processes agents watch and their output pipes until exec.
pub(crate) fn hand_over_watched(watched: Vec<crate::runtime::state::WatchedProcessTransfer>) {
    if let Some(prepared) = lock(&PREPARED).as_mut() {
        for transfer in watched {
            prepared.handover.watched.push(transfer.handover);
            prepared.masters.extend(transfer.pipes);
            prepared.watched_resources.extend(transfer.resources);
        }
    }
}

pub(crate) fn hand_over_job(
    job_id: String,
    wake_id: String,
    exec: Option<crate::room_exec::JobExecHandover>,
    pipes: Vec<OwnedFd>,
    guard: Option<crate::room_exec::JobExecGuard>,
    log: OwnedFd,
    body: &[u8],
) -> std::io::Result<()> {
    let state = job_state_file::write(body)?;
    let mut prepared = lock(&PREPARED);
    let prepared = prepared
        .as_mut()
        .ok_or_else(|| std::io::Error::other("the kernel reload is no longer prepared"))?;
    prepared.handover.local_jobs.push(JobHandover::new(
        job_id,
        wake_id,
        exec,
        state.as_raw_fd(),
        log.as_raw_fd(),
    ));
    prepared.masters.extend(pipes);
    prepared.masters.push(state);
    prepared.masters.push(log);
    prepared.local_job_guards.extend(guard);
    Ok(())
}

pub(crate) fn job_wakes() -> Vec<String> {
    lock(&RECEIVED_JOB_READERS)
        .iter()
        .map(|job| job.wake_id().to_string())
        .collect()
}

pub(crate) fn take_job(id: &str) -> Option<JobHandover> {
    let mut jobs = lock(&RECEIVED_JOB_READERS);
    let index = jobs.iter().position(|job| job.id() == id)?;
    Some(jobs.swap_remove(index))
}

pub(crate) fn discard_unclaimed_jobs() {
    let jobs = std::mem::take(&mut *lock(&RECEIVED_JOB_READERS));
    for job in jobs {
        job.discard();
    }
}

pub(crate) fn read_job_state(fd: i32) -> std::io::Result<zeroize::Zeroizing<Vec<u8>>> {
    // The previous program named this descriptor for this job alone.
    job_state_file::read(unsafe { OwnedFd::from_raw_fd(fd) })
}

/// Keeps the users' focus and grants for the next program.
pub(crate) fn hand_over_user_domain(access: crate::runtime::kernel_browser_host::LoadedHandover) {
    if let Some(prepared) = lock(&PREPARED).as_mut() {
        prepared.handover.user_domain = access;
    }
}

/// Keeps the owner's gateway revocations of live provider runs for the next program.
pub(crate) fn hand_over_revoked_reach_runs(runs: impl IntoIterator<Item = String>) {
    if let Some(prepared) = lock(&PREPARED).as_mut() {
        prepared.handover.revoked_reach_runs = runs.into_iter().collect();
    }
}

/// Keeps the attachments of a stopping session for the next program.
pub(crate) fn hand_over_attachments(attachments: &[crate::attachment::RuntimeAttachment]) {
    if let Some(prepared) = lock(&PREPARED).as_mut() {
        prepared
            .handover
            .attachments
            .extend(attachments.iter().map(|a| AttachmentHandover {
                id: a.id().to_string(),
                session_id: a.session_id().to_string(),
                client_id: a.client_id().to_string(),
                capability_level: a.capability_level(),
                owner_user_id: a.owner_user_id().to_string(),
            }));
    }
}

/// Offers a provider server's PTY to a running turn that may be handed over.
/// The PTY of a run whose turn is not handed over ends before the swap.
pub(crate) fn offer_provider_pty(run_id: &str, pty: crate::pty::OutgoingPty) {
    if let Some(prepared) = lock(&PREPARED).as_mut() {
        prepared.masters.push(pty.master);
        prepared
            .offered_ptys
            .insert(run_id.to_string(), pty.handover);
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
        HandedOverProcess::Codex {
            pty,
            turn: Box::new(turn),
        },
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
        identity: Some(RunIdentity::of(run)),
        process,
    }
}

/// A prepared swap: the terminals are held and saved; the rest of the kernel
/// stops as for a planned restart before `finish` replaces the program.
struct Prepared {
    handover: Handover,
    // The PTY masters the handover names, owned by the swap alone until
    // `exec`: the terminals and runs that handed them over close their own
    // descriptors whenever the kernel's stop drops them.
    masters: Vec<OwnedFd>,
    watched_resources: Vec<crate::runtime::process_resources::ProcessDomain>,
    // The stopped kernel's state, which owns the handed-over PTY processes.
    kernel: Option<Box<dyn std::any::Any + Send>>,
    path: PathBuf,
    offered_ptys: BTreeMap<String, crate::pty::PtyHandover>,
    local_job_guards: Vec<crate::room_exec::JobExecGuard>,
}

impl Prepared {
    fn new(terminals: Vec<(TerminalHandover, OwnedFd)>, path: PathBuf) -> Self {
        let (terminals, masters) = terminals.into_iter().unzip();
        Self {
            handover: Handover::with_terminals(terminals),
            masters,
            watched_resources: Vec::new(),
            kernel: None,
            path,
            offered_ptys: BTreeMap::new(),
            local_job_guards: Vec::new(),
        }
    }
}

static PREPARED: Mutex<Option<Prepared>> = Mutex::new(None);

type RestartAnswer = tokio::sync::oneshot::Sender<Result<(), String>>;
/// A requested restart with the deadline its requester's wait gives it, fixed when it was enqueued:
/// time spent behind another reload counts against it.
struct RestartRequest {
    answer: RestartAnswer,
    deadline: Instant,
}
static RESTART_REQUESTS: OnceLock<tokio::sync::mpsc::UnboundedSender<RestartRequest>> =
    OnceLock::new();
/// How long the kernel keeps serving after it prepared a requested restart: the answer to the request leaves first.
const RESTART_REPLY_GRACE: Duration = Duration::from_millis(400);

/// Running agent turns whose runtime a command holds right now cannot be
/// handed over: they are not at a safe point. Set once by the provider runtime.
type BusyTurns = Box<dyn Fn() -> Vec<String> + Send + Sync>;
static BUSY_TURNS: OnceLock<BusyTurns> = OnceLock::new();

pub(crate) fn watch_busy_turns(probe: impl Fn() -> Vec<String> + Send + Sync + 'static) {
    let _ = BUSY_TURNS.set(Box::new(probe));
}

const SAFE_POINT_POLL: Duration = Duration::from_millis(100);

/// Where the caller of a reload leaves its deadline, next to the handover.
/// A request, not a handover: nothing reads it after the signal.
fn reload_request_path(handover_path: &std::path::Path) -> PathBuf {
    handover_path.with_file_name("hot-reload-request.json")
}

/// The caller's deadline for this reload, once: the file is removed as it is
/// read, so one attempt's deadline never decides the next. No readable file
/// means no deadline: the reload waits for its safe point as long as it takes.
fn take_reload_deadline(handover_path: &std::path::Path) -> Option<Instant> {
    let path = reload_request_path(handover_path);
    let body = std::fs::read(&path).ok()?;
    let _ = std::fs::remove_file(&path);
    let at_ms = serde_json::from_slice::<serde_json::Value>(&body)
        .ok()?
        .get("deadline_unix_ms")?
        .as_u64()?;
    let left_ms = at_ms.saturating_sub(crate::session::unix_epoch_ms());
    Some(Instant::now() + Duration::from_millis(left_ms))
}

/// New provider work (a prompt, a steer) is admitted while this is open. The reload closes
/// it once, right before accepting a safe point, so no command can take a turn's runtime
/// between the probe and the handover; a refused reload opens it again.
static ADMISSION_CLOSED: std::sync::RwLock<bool> = std::sync::RwLock::new(false);

const ADMISSION_CLOSED_REASON: &str =
    "the kernel is preparing a planned reload; try again in a moment";

/// Held by a command while it registers its work, so a closing reload sees it.
pub(crate) fn admit_provider_work(
) -> Result<std::sync::RwLockReadGuard<'static, bool>, &'static str> {
    admit_into(&ADMISSION_CLOSED)
}

fn admit_into(
    gate: &std::sync::RwLock<bool>,
) -> Result<std::sync::RwLockReadGuard<'_, bool>, &'static str> {
    let guard = gate
        .read()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if *guard {
        Err(ADMISSION_CLOSED_REASON)
    } else {
        Ok(guard)
    }
}

fn set_admission_closed(closed: bool) {
    *ADMISSION_CLOSED
        .write()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = closed;
    // A new attempt starts clean; an opened gate is no longer committed.
    LATE_ABORTS.store(0, std::sync::atomic::Ordering::SeqCst);
    COMMITTED.store(false, std::sync::atomic::Ordering::SeqCst);
}

/// Aborts admitted while the gate was closed and the reload not yet committed. An abort is never
/// refused (the cancellation is committed before it is queued), so it spoils the safe point
/// instead: the commit check sees it and the reload waits for the next one.
static LATE_ABORTS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
static COMMITTED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Held by an abort while it registers its work: never refused, but counted when it lands
/// between the safe point and the commit.
pub(crate) fn admit_abort() -> std::sync::RwLockReadGuard<'static, bool> {
    admit_abort_into(&ADMISSION_CLOSED, &LATE_ABORTS, &COMMITTED)
}

fn admit_abort_into<'a>(
    gate: &'a std::sync::RwLock<bool>,
    late: &std::sync::atomic::AtomicUsize,
    committed: &std::sync::atomic::AtomicBool,
) -> std::sync::RwLockReadGuard<'a, bool> {
    let guard = gate
        .read()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if *guard && !committed.load(std::sync::atomic::Ordering::SeqCst) {
        late.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    }
    guard
}

/// The last step before the reload commits, with admission held shut by the write lock: no
/// abort arrived since the safe point and nothing is busy. Otherwise the safe point was
/// spoiled: admission opens again and the caller waits for the next one.
fn commit_safe_point_into(
    gate: &std::sync::RwLock<bool>,
    late: &std::sync::atomic::AtomicUsize,
    committed: &std::sync::atomic::AtomicBool,
    busy: &(dyn Fn() -> Vec<String> + Sync),
    deadline: Option<Instant>,
) -> bool {
    let mut closed = gate
        .write()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    // The caller's deadline decides here too: terminal preparation and this lock can have
    // used it up, and a reload that commits after it would start after the caller's rollback.
    let spoiled = late.swap(0, std::sync::atomic::Ordering::SeqCst) > 0
        || !busy().is_empty()
        || deadline.is_some_and(|deadline| Instant::now() >= deadline);
    if spoiled {
        *closed = false;
        false
    } else {
        committed.store(true, std::sync::atomic::Ordering::SeqCst);
        true
    }
}

/// Waits until no running turn is held by a command, or the caller's deadline
/// passes: then the reload is refused with the busy turns named and nothing
/// has changed. The kernel keeps serving meanwhile. A safe point is accepted
/// only with admission closed and a second probe empty, so work admitted
/// between the first probe and the close is seen; an expired deadline is
/// never accepted, even by an empty probe.
async fn wait_for_safe_point(
    busy: &(dyn Fn() -> Vec<String> + Sync),
    deadline: Option<Instant>,
    poll: Duration,
    close_admission: &(dyn Fn() + Sync),
    open_admission: &(dyn Fn() + Sync),
) -> Result<(), String> {
    loop {
        let mut expired = deadline.is_some_and(|deadline| Instant::now() >= deadline);
        let mut turns = busy();
        if turns.is_empty() && !expired {
            close_admission();
            turns = busy();
            if turns.is_empty() && !deadline.is_some_and(|deadline| Instant::now() >= deadline) {
                return Ok(());
            }
            // Spoiled by work admitted before the close, or the deadline passed while
            // the kernel was acquiring the safe point: admission opens again.
            open_admission();
            expired = deadline.is_some_and(|deadline| Instant::now() >= deadline);
        }
        if expired {
            turns.sort();
            return Err(if turns.is_empty() {
                "the reload's deadline passed before the kernel reached a safe point; nothing was changed and every agent keeps running".to_string()
            } else {
                format!(
                    "{} running agent turns are busy and cannot be handed over yet ({}); the reload's deadline passed, so nothing was changed and every agent keeps running",
                    turns.len(),
                    turns.join(", ")
                )
            });
        }
        tokio::time::sleep(poll).await;
    }
}

/// Completes when a hot reload is prepared, on a SIGUSR2 or on a request (`request_restart`). A refused request
/// changes nothing: the kernel keeps running and waits for the next one; a requester is told why.
pub async fn requested(terminals: RoomTerminalService, handover_path: PathBuf) {
    use tokio::signal::unix::{signal, SignalKind};
    let (restart_tx, mut restarts) = tokio::sync::mpsc::unbounded_channel::<RestartRequest>();
    let _ = RESTART_REQUESTS.set(restart_tx);
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
    loop {
        let answer = tokio::select! {
            signalled = requests.recv() => { if signalled.is_none() { break; } None }
            asked = restarts.recv() => match asked { Some(request) => Some(request), None => break },
        };
        // A request answers within the requester's own wait: its deadline was fixed when it was
        // enqueued, so a reload it was told "did not prepare in time" never commits behind its back,
        // and a request that waited behind another reload past its deadline is refused unprepared.
        let (answer, requested_deadline) = match answer {
            Some(request) => {
                if Instant::now() >= request.deadline || request.answer.is_closed() {
                    let _ = request.answer.send(Err(
                        "the hot reload did not prepare in time; nothing was changed and every agent keeps running".to_string(),
                    ));
                    continue;
                }
                (Some(request.answer), Some(request.deadline))
            }
            None => (None, None),
        };
        match prepare(&terminals, &handover_path, requested_deadline).await {
            Ok(terminals) => {
                *PREPARED.lock().unwrap_or_else(|e| e.into_inner()) =
                    Some(Prepared::new(terminals, handover_path));
                if let Some(answer) = answer {
                    let _ = answer.send(Ok(()));
                    tokio::time::sleep(RESTART_REPLY_GRACE).await;
                }
                return;
            }
            Err(reason) => {
                crate::logging::error_with_fields(
                    "hot_reload",
                    "hot reload refused; the kernel keeps running",
                    serde_json::json!({ "reason": reason }),
                );
                if let Some(answer) = answer {
                    let _ = answer.send(Err(reason));
                }
            }
        }
    }
    std::future::pending().await
}

/// A person's or agent's request to restart this kernel (row 551): the same preparation as a SIGUSR2, answered. `Ok` means the
/// hot reload is prepared and the kernel is stopping; `Err` is why it was refused and the kernel keeps running.
pub async fn request_restart() -> Result<(), String> {
    let Some(requests) = RESTART_REQUESTS.get() else {
        return Err("hot reload is unavailable on this kernel".into());
    };
    let reply = enqueue_restart(requests)?;
    match tokio::time::timeout(request_wait(), reply).await {
        Ok(Ok(result)) => result,
        Ok(Err(_)) => Err("the kernel is already stopping".into()),
        Err(_) => Err("the hot reload did not prepare in time".into()),
    }
}

/// Queues a restart request with its deadline, fixed now.
fn enqueue_restart(
    requests: &tokio::sync::mpsc::UnboundedSender<RestartRequest>,
) -> Result<tokio::sync::oneshot::Receiver<Result<(), String>>, String> {
    let (answer, reply) = tokio::sync::oneshot::channel();
    requests
        .send(RestartRequest {
            answer,
            deadline: Instant::now() + requested_reload_patience(),
        })
        .map_err(|_| "hot reload is unavailable on this kernel".to_string())?;
    Ok(reply)
}

/// How long a requester of a restart waits for the answer.
fn request_wait() -> Duration {
    VERIFY_TIMEOUT * 3
}

/// A requester's own wait decides its reload's deadline (no number of its own): the refusal reaches
/// it, with the busy turns named, before it gives up, with the verification's time kept back.
fn requested_reload_patience() -> Duration {
    request_wait().saturating_sub(VERIFY_TIMEOUT)
}

async fn prepare(
    terminals: &RoomTerminalService,
    handover_path: &std::path::Path,
    requested_deadline: Option<Instant>,
) -> Result<Vec<(TerminalHandover, OwnedFd)>, String> {
    let start = START.get().ok_or("the kernel program path is unknown")?;
    let file_deadline = take_reload_deadline(handover_path);
    let deadline = match (file_deadline, requested_deadline) {
        (Some(file), Some(requested)) => Some(file.min(requested)),
        (file, requested) => file.or(requested),
    };
    verify(start).await?;
    loop {
        if let Some(busy) = BUSY_TURNS.get() {
            wait_for_safe_point(
                busy.as_ref(),
                deadline,
                SAFE_POINT_POLL,
                &|| set_admission_closed(true),
                &|| set_admission_closed(false),
            )
            .await?;
        }
        let handing = terminals.clone();
        let handed = match tokio::task::spawn_blocking(move || handing.hand_over()).await {
            Ok(Ok(handed)) => handed,
            Ok(Err(error)) => {
                terminals.release_handover();
                set_admission_closed(false);
                return Err(error.to_string());
            }
            Err(error) => {
                terminals.release_handover();
                set_admission_closed(false);
                return Err(error.to_string());
            }
        };
        let committed = match BUSY_TURNS.get() {
            Some(busy) => commit_safe_point_into(
                &ADMISSION_CLOSED,
                &LATE_ABORTS,
                &COMMITTED,
                busy.as_ref(),
                deadline,
            ),
            None => true,
        };
        if committed {
            return Ok(handed);
        }
        // An abort (or other work) landed during terminal preparation: that safe point is
        // spoiled. Nothing was committed; prepare again within the caller's deadline.
        terminals.release_handover();
        drop(handed);
        if deadline.is_some_and(|deadline| Instant::now() >= deadline) {
            return Err("the reload's deadline passed before the kernel reached a safe point; nothing was changed and every agent keeps running".to_string());
        }
    }
}

/// After the kernel stopped for a prepared hot reload: replaces this program
/// with the installed one, handing over the terminals. Returns `None` when no
/// hot reload was prepared, or why the new program could not start.
pub fn finish() -> Option<String> {
    let error = replace_program()?;
    // Nothing took over the jobs workloads left running: it was a restart.
    crate::runtime::process_resources::end_owner(None, None);
    Some(error)
}

fn replace_program() -> Option<String> {
    let mut prepared = PREPARED.lock().unwrap_or_else(|e| e.into_inner()).take()?;
    let start = START.get()?;
    // Read last: everything has stopped, so no workload ends after this.
    prepared.handover.released_jobs = crate::runtime::process_resources::hand_over_released();
    prepared.handover.admission = crate::runtime::process_resources::hand_over_admission();
    match send_credentials(&prepared.handover.providers) {
        Ok(fd) => prepared.handover.credentials_fd = fd,
        Err(error) => return Some(format!("the credentials could not be handed over: {error}")),
    }
    match send_vault_unlocks() {
        Ok(fd) => prepared.handover.vault_fd = fd,
        Err(error) => {
            return Some(format!(
                "the Vault unlocks could not be handed over: {error}"
            ))
        }
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
        .inherited_fds()
        .into_iter()
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
        .chain(
            prepared
                .handover
                .released_jobs
                .iter()
                .flat_map(crate::runtime::process_resources::ReleasedJobs::pids),
        )
        .chain(
            prepared
                .handover
                .watched
                .iter()
                .map(crate::runtime::state::WatchedHandover::pid),
        )
        .chain(
            prepared
                .handover
                .local_jobs
                .iter()
                .filter_map(JobHandover::pid),
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
            .bounded_output(),
    )
    .await
    .map_err(|_| "the new kernel program did not answer in time".to_string())?
    .map_err(|error| format!("the new kernel program does not run: {error}"))?;
    let format = String::from_utf8_lossy(&output.stdout);
    let takes = format
        .trim()
        .parse::<u32>()
        .is_ok_and(|format| format == HANDOVER_FORMAT || format == HANDOVER_FORMAT + 1);
    if !output.status.success() || !takes {
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

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;
    use crate::secret::{
        chariox_encrypted_vault_status, clear_all_chariox_encrypted_vault_unlocks,
        create_chariox_encrypted_vault_for_test, unlock_existing_chariox_encrypted_vault,
        VaultUnlockLease,
    };

    async fn safe_point(
        busy: &(dyn Fn() -> Vec<String> + Sync),
        deadline: Option<Instant>,
        poll: Duration,
    ) -> Result<(), String> {
        wait_for_safe_point(busy, deadline, poll, &|| {}, &|| {}).await
    }

    #[tokio::test]
    async fn a_reload_accepts_no_safe_point_that_work_admitted_before_the_close_has_spoiled() {
        // The first probe is empty; work registered before admission closed shows in the second.
        let probes = std::sync::atomic::AtomicUsize::new(0);
        let closed = std::sync::atomic::AtomicBool::new(false);
        let reopened = std::sync::atomic::AtomicUsize::new(0);
        let probe = || {
            let probe = probes.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            if probe == 1 {
                vec!["run-a".to_string()]
            } else {
                Vec::new()
            }
        };
        wait_for_safe_point(
            &probe,
            Some(Instant::now() + Duration::from_secs(5)),
            Duration::from_millis(10),
            &|| closed.store(true, std::sync::atomic::Ordering::SeqCst),
            &|| {
                closed.store(false, std::sync::atomic::Ordering::SeqCst);
                reopened.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            },
        )
        .await
        .expect("the work finished, the next probe pair is empty");
        assert_eq!(
            reopened.load(std::sync::atomic::Ordering::SeqCst),
            1,
            "a spoiled safe point opens admission again before it waits"
        );
        assert!(
            closed.load(std::sync::atomic::Ordering::SeqCst),
            "the accepted safe point keeps admission closed"
        );
    }

    #[tokio::test]
    async fn a_reload_whose_deadline_passed_is_refused_even_when_nothing_is_busy() {
        let nothing_busy = || Vec::<String>::new();
        let closed = std::sync::atomic::AtomicBool::new(false);
        let reason = wait_for_safe_point(
            &nothing_busy,
            Some(Instant::now() - Duration::from_millis(1)),
            Duration::from_millis(10),
            &|| closed.store(true, std::sync::atomic::Ordering::SeqCst),
            &|| {},
        )
        .await
        .expect_err("the caller has begun its own rollback: do not commit");
        assert!(reason.contains("deadline passed"), "{reason}");
        assert!(reason.contains("every agent keeps running"), "{reason}");
        assert!(!closed.load(std::sync::atomic::Ordering::SeqCst));
    }

    #[tokio::test]
    async fn a_deadline_that_passes_while_the_safe_point_is_acquired_refuses_and_reopens_admission()
    {
        let nothing_busy = || Vec::<String>::new();
        let closed = std::sync::atomic::AtomicBool::new(false);
        let reason = wait_for_safe_point(
            &nothing_busy,
            Some(Instant::now() + Duration::from_millis(40)),
            Duration::from_millis(10),
            &|| {
                // Acquiring the admission lock took longer than the caller had left.
                std::thread::sleep(Duration::from_millis(80));
                closed.store(true, std::sync::atomic::Ordering::SeqCst);
            },
            &|| closed.store(false, std::sync::atomic::Ordering::SeqCst),
        )
        .await
        .expect_err("the deadline passed before the safe point was accepted");
        assert!(reason.contains("deadline passed"), "{reason}");
        assert!(
            !closed.load(std::sync::atomic::Ordering::SeqCst),
            "a refused reload leaves admission open"
        );
    }

    #[test]
    fn the_commit_check_refuses_when_an_abort_landed_after_the_safe_point_and_reopens_admission() {
        use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
        let gate = std::sync::RwLock::new(true);
        let late = AtomicUsize::new(0);
        let committed = AtomicBool::new(false);
        let nothing_busy = || Vec::<String>::new();
        // An abort is never refused while the gate is closed, but it is counted.
        drop(admit_abort_into(&gate, &late, &committed));
        assert_eq!(late.load(Ordering::SeqCst), 1);
        assert!(!commit_safe_point_into(
            &gate,
            &late,
            &committed,
            &nothing_busy,
            None
        ));
        assert!(
            !*gate.read().unwrap(),
            "the spoiled safe point opens admission again"
        );
        assert!(!committed.load(Ordering::SeqCst));
        // The next attempt, with no late abort and nothing busy, commits and stays closed.
        *gate.write().unwrap() = true;
        assert!(commit_safe_point_into(
            &gate,
            &late,
            &committed,
            &nothing_busy,
            None
        ));
        assert!(*gate.read().unwrap());
        assert!(committed.load(Ordering::SeqCst));
        // After the commit an abort is the shutdown's own: not counted.
        drop(admit_abort_into(&gate, &late, &committed));
        assert_eq!(late.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn the_commit_check_refuses_an_expired_deadline_even_when_nothing_is_busy() {
        use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
        let gate = std::sync::RwLock::new(true);
        let late = AtomicUsize::new(0);
        let committed = AtomicBool::new(false);
        let nothing_busy = || Vec::<String>::new();
        // Terminal preparation used the caller's time up: the last probe is empty, too late.
        let expired = Instant::now() - Duration::from_millis(1);
        assert!(!commit_safe_point_into(
            &gate,
            &late,
            &committed,
            &nothing_busy,
            Some(expired)
        ));
        assert!(!*gate.read().unwrap(), "admission is open again");
        assert!(!committed.load(Ordering::SeqCst));
        *gate.write().unwrap() = true;
        assert!(commit_safe_point_into(
            &gate,
            &late,
            &committed,
            &nothing_busy,
            Some(Instant::now() + Duration::from_secs(30))
        ));
    }

    #[test]
    fn the_commit_check_refuses_when_something_is_busy_at_the_last_moment() {
        use std::sync::atomic::{AtomicBool, AtomicUsize};
        let gate = std::sync::RwLock::new(true);
        let late = AtomicUsize::new(0);
        let committed = AtomicBool::new(false);
        let busy = || vec!["run-a".to_string()];
        assert!(!commit_safe_point_into(
            &gate, &late, &committed, &busy, None
        ));
        assert!(!*gate.read().unwrap());
    }

    #[tokio::test]
    async fn a_requests_deadline_is_fixed_when_it_is_enqueued_not_when_it_is_dequeued() {
        let (requests, mut queue) = tokio::sync::mpsc::unbounded_channel::<RestartRequest>();
        let before = Instant::now();
        let _first = enqueue_restart(&requests).unwrap();
        let _second = enqueue_restart(&requests).unwrap();
        // Another reload holds the queue; the second request is dequeued much later.
        tokio::time::sleep(Duration::from_millis(60)).await;
        let first = queue.recv().await.unwrap();
        let second = queue.recv().await.unwrap();
        let patience = requested_reload_patience();
        for request in [&first, &second] {
            assert!(request.deadline >= before + patience);
            assert!(
                request.deadline <= Instant::now() + patience - Duration::from_millis(40),
                "the deadline did not restart when the request was dequeued"
            );
        }
    }

    #[test]
    fn a_requested_restart_is_refused_before_its_requester_stops_waiting() {
        // The refusal, with the busy turns named, reaches the requester inside its own wait,
        // and the verification keeps its time: a reload it was told did not prepare never commits.
        assert!(requested_reload_patience() + VERIFY_TIMEOUT <= request_wait());
        assert!(requested_reload_patience() > Duration::ZERO);
    }

    #[test]
    fn admission_is_refused_while_a_reload_holds_it_closed_and_opens_again() {
        let gate = std::sync::RwLock::new(false);
        assert!(admit_into(&gate).is_ok());
        *gate.write().unwrap() = true;
        let refused = admit_into(&gate).unwrap_err();
        assert!(refused.contains("try again"), "{refused}");
        *gate.write().unwrap() = false;
        assert!(admit_into(&gate).is_ok());
    }

    #[tokio::test]
    async fn a_reload_goes_ahead_when_every_running_turn_can_be_handed_over() {
        let nothing_busy = || Vec::<String>::new();
        safe_point(
            &nothing_busy,
            Some(Instant::now() + Duration::from_millis(200)),
            Duration::from_millis(10),
        )
        .await
        .expect("no busy turn: nothing to wait for");
    }

    #[tokio::test]
    async fn a_reload_waits_for_a_busy_turn_to_reach_a_safe_point() {
        let polls = std::sync::atomic::AtomicUsize::new(0);
        let busy_twice = || {
            if polls.fetch_add(1, std::sync::atomic::Ordering::SeqCst) < 2 {
                vec!["run-a".to_string()]
            } else {
                Vec::new()
            }
        };
        safe_point(
            &busy_twice,
            Some(Instant::now() + Duration::from_secs(5)),
            Duration::from_millis(10),
        )
        .await
        .expect("the turn became safe within the wait");
        assert!(polls.load(std::sync::atomic::Ordering::SeqCst) >= 3);
    }

    #[tokio::test]
    async fn a_reload_is_refused_with_the_busy_turns_named_when_none_reaches_a_safe_point() {
        let started = Instant::now();
        let busy = || vec!["run-b".to_string(), "run-a".to_string()];
        let reason = safe_point(
            &busy,
            Some(Instant::now() + Duration::from_millis(150)),
            Duration::from_millis(10),
        )
        .await
        .expect_err("a turn that never frees up refuses the reload");
        assert!(started.elapsed() >= Duration::from_millis(150));
        assert!(
            reason.contains("run-a") && reason.contains("run-b"),
            "{reason}"
        );
        assert!(reason.contains("2 running agent turns"), "{reason}");
        assert!(
            !reason.contains("Error") && !reason.contains("panicked"),
            "{reason}"
        );
    }

    #[tokio::test]
    async fn a_refused_reload_changes_nothing_and_a_second_attempt_can_go_ahead() {
        let busy = std::sync::atomic::AtomicBool::new(true);
        let probe = || {
            if busy.load(std::sync::atomic::Ordering::SeqCst) {
                vec!["run-a".to_string()]
            } else {
                Vec::new()
            }
        };
        safe_point(
            &probe,
            Some(Instant::now() + Duration::from_millis(50)),
            Duration::from_millis(10),
        )
        .await
        .unwrap_err();
        assert!(!handing_over(), "a refusal prepares no handover");
        busy.store(false, std::sync::atomic::Ordering::SeqCst);
        safe_point(
            &probe,
            Some(Instant::now() + Duration::from_millis(50)),
            Duration::from_millis(10),
        )
        .await
        .expect("the second attempt goes ahead");
    }

    #[tokio::test]
    async fn a_reload_without_a_deadline_waits_for_the_safe_point_however_long_it_takes() {
        let polls = std::sync::atomic::AtomicUsize::new(0);
        let busy_for_a_while = || {
            if polls.fetch_add(1, std::sync::atomic::Ordering::SeqCst) < 30 {
                vec!["run-a".to_string()]
            } else {
                Vec::new()
            }
        };
        safe_point(&busy_for_a_while, None, Duration::from_millis(5))
            .await
            .expect("no deadline: the kernel never gives up the reload on its own");
    }

    #[test]
    fn the_callers_deadline_is_read_once_from_the_request_file() {
        let dir = std::env::temp_dir().join(format!("hr-request-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let handover = dir.join("handover.json");
        assert_eq!(
            take_reload_deadline(&handover),
            None,
            "no file: no deadline"
        );
        let at = crate::session::unix_epoch_ms() + 5_000;
        std::fs::write(
            reload_request_path(&handover),
            serde_json::json!({ "deadline_unix_ms": at }).to_string(),
        )
        .unwrap();
        let deadline = take_reload_deadline(&handover).expect("the file names a deadline");
        let left = deadline.saturating_duration_since(Instant::now());
        assert!(
            left > Duration::from_secs(3) && left <= Duration::from_secs(5),
            "{left:?}"
        );
        assert!(
            !reload_request_path(&handover).exists(),
            "a deadline belongs to one attempt"
        );
        assert_eq!(take_reload_deadline(&handover), None);
        std::fs::write(reload_request_path(&handover), "not json").unwrap();
        assert_eq!(
            take_reload_deadline(&handover),
            None,
            "an unreadable request is no deadline"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_deadline_already_past_refuses_a_busy_reload_at_once() {
        let dir = std::env::temp_dir().join(format!("hr-past-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let handover = dir.join("handover.json");
        std::fs::write(
            reload_request_path(&handover),
            serde_json::json!({ "deadline_unix_ms": 1 }).to_string(),
        )
        .unwrap();
        let deadline = take_reload_deadline(&handover).unwrap();
        assert!(deadline <= Instant::now());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_taken_preferred_port_falls_back_but_an_exact_port_fails() {
        let holder = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let taken = holder.local_addr().unwrap().port();

        let fallback = tcp_listener(
            "test_preferred",
            "127.0.0.1",
            ListenPort::PreferredElseAny(taken),
        )
        .expect("a taken preferred port falls back to one the OS assigns");
        let port = fallback.local_addr().unwrap().port();
        assert_ne!(port, taken);
        // The table is the process's: a later test that lists the descriptors
        // a hot reload hands over must not find this listener in it.
        let kept = lock(&LISTENERS).remove("test_preferred").unwrap();
        // SAFETY: the kept copy of the listener, owned by the table until removed.
        unsafe { libc::close(kept) };

        let error = tcp_listener("test_exact", "127.0.0.1", ListenPort::Exact(taken))
            .expect_err("a taken exact port is an error");
        assert_eq!(error.kind(), std::io::ErrorKind::AddrInUse);
        assert!(lock(&LISTENERS).get("test_exact").is_none());
    }

    fn unlocked_vault() -> (crate::test_support::TestTempDir, PathBuf) {
        let owner = crate::test_support::TestTempDir::new("hot-reload-vault");
        let path = owner.join("vault");
        create_chariox_encrypted_vault_for_test(&path, "correct horse battery").unwrap();
        unlock_existing_chariox_encrypted_vault(
            &path,
            "correct horse battery",
            VaultUnlockLease::TtlMinutes(30),
        )
        .unwrap()
        .expect("the passphrase opens the test Vault");
        (owner, path)
    }

    fn is_open(fd: i32) -> bool {
        unsafe { libc::fcntl(fd, libc::F_GETFD) >= 0 }
    }

    #[test]
    fn unlocked_vault_survives_the_swap_and_its_descriptor_is_read_once() {
        clear_all_chariox_encrypted_vault_unlocks().unwrap();
        let (_owner, path) = unlocked_vault();
        let before = chariox_encrypted_vault_status(&path).unwrap();
        let fd = send_vault_unlocks()
            .unwrap()
            .expect("an unlocked Vault is handed over");
        // The new program starts with nothing unlocked.
        clear_all_chariox_encrypted_vault_unlocks().unwrap();
        assert!(!chariox_encrypted_vault_status(&path).unwrap().unlocked);

        receive_vault_unlocks(fd);
        assert_eq!(chariox_encrypted_vault_status(&path).unwrap(), before);
        assert!(!is_open(fd), "the descriptor is consumed and closed");
        clear_all_chariox_encrypted_vault_unlocks().unwrap();
    }

    #[test]
    fn locked_vault_hands_nothing_over() {
        clear_all_chariox_encrypted_vault_unlocks().unwrap();
        assert!(send_vault_unlocks().unwrap().is_none());
    }

    #[test]
    fn restart_without_a_handover_starts_locked() {
        let (_owner, path) = unlocked_vault();
        clear_all_chariox_encrypted_vault_unlocks().unwrap();
        std::env::remove_var(HANDOVER_ENV);
        receive();
        assert!(!chariox_encrypted_vault_status(&path).unwrap().unlocked);
    }

    #[test]
    fn handover_file_names_the_descriptor_not_the_keys() {
        let handover = Handover {
            vault_fd: Some(9),
            ..Handover::default()
        };
        let json = serde_json::to_string(&handover).unwrap();
        assert!(json.contains("\"vault_fd\":9"));
        assert!(!json.contains("key"));
    }
}
