//! MP-08 / MP-09 / MP-10 / MP-11 A03: kernel-owned watched processes.
//! The kernel starts argv (no shell) in the agent's workspace, owns the
//! isolated process session, drains bounded sanitized output and reports exit or one
//! output match. It never attaches to an arbitrary PID.
use super::agent_process_group::{signal_session, Leader};
use super::agent_process_output::{drain, room_protector, Output, OutputSnapshot, Signal};
use super::*;
use crate::durable_state::agent_lifecycle::{AgentWake, Operation};
use crate::runtime::bounded_process::BoundedChildExt as _;
use std::collections::HashMap;
use std::os::fd::OwnedFd;
use std::os::unix::process::ExitStatusExt;
use std::process::{Child, Command, Stdio};

mod handover;
mod run;
use handover::Origin;
pub(crate) use handover::{WatchedHandover, WatchedProcessTransfer};
pub(super) use run::{KernelCommand, KernelCommandExit, KernelCommandOutput};

const ENV_ALLOW: &[&str] = &[
    "PATH",
    "HOME",
    "USER",
    "LOGNAME",
    "LANG",
    "LC_ALL",
    "LC_CTYPE",
    "TZ",
    "TMPDIR",
    "TERM",
    "XDG_CONFIG_HOME",
    "XDG_CACHE_HOME",
    "XDG_DATA_HOME",
    "XDG_RUNTIME_DIR",
    "SSH_AUTH_SOCK",
    "GH_CONFIG_DIR",
];

struct OwnedChild {
    child: Leader,
    birth: u64,
    wake: String,
    /// Set once the leader is reaped after every session member settled.
    status: Option<std::process::ExitStatus>,
    leader_status: Option<std::process::ExitStatus>,
    settle_failures: u8,
    lost: bool,
    /// Counted under the agent's share of the memory ceiling and listed in Health.
    resources: Option<crate::runtime::process_resources::ProcessDomain>,
    /// What the next program needs to supervise it again after a hot reload.
    origin: Origin,
    /// Copies of the output pipes' read ends: they cross a hot reload.
    pipes: Vec<OwnedFd>,
    /// Set when a hot reload asks supervision to stop without an outcome.
    handing_over: bool,
    /// What the output showed when supervision stopped for a hot reload.
    handed_over: Option<OutputSnapshot>,
}

impl OwnedChild {
    fn new(mut child: Child, wake: &str) -> std::io::Result<Self> {
        match super::agent_process_group::birth(child.id()) {
            Ok(birth) => Ok(Self::of(
                Leader::Spawned(child),
                birth,
                wake,
                Origin::default(),
                Vec::new(),
            )),
            Err(error) => {
                // The freshly spawned positive child handle is owned even
                // when group inspection failed. Do not signal a group.
                if child.id() > 1 {
                    let _ = child.kill();
                    let _ = child.bounded_wait();
                }
                Err(error)
            }
        }
    }
    /// The child of the previous program, which a hot reload handed over.
    fn adopted(pid: u32, birth: u64, wake: &str, origin: Origin, pipes: Vec<OwnedFd>) -> Self {
        Self::of(Leader::Adopted(pid), birth, wake, origin, pipes)
    }
    fn of(child: Leader, birth: u64, wake: &str, origin: Origin, pipes: Vec<OwnedFd>) -> Self {
        Self {
            child,
            birth,
            wake: wake.into(),
            status: None,
            leader_status: None,
            settle_failures: 0,
            lost: false,
            resources: None,
            origin,
            pipes,
            handing_over: false,
            handed_over: None,
        }
    }
    fn fail_settlement(&mut self) {
        self.settle_failures = self.settle_failures.saturating_add(1);
        self.lost = self.settle_failures >= 50;
    }
    fn poll(&mut self) -> std::io::Result<Option<std::process::ExitStatus>> {
        if self.lost {
            return Err(std::io::Error::other(
                "process_lost: physical settlement remains unconfirmed after bounded retries",
            ));
        }
        if self.status.is_none() {
            if self.leader_status.is_none() {
                self.leader_status =
                    match super::agent_process_group::poll_exit(&mut self.child, self.birth) {
                        Ok(status) => status,
                        Err(error) => {
                            self.fail_settlement();
                            return Err(error);
                        }
                    };
            }
            if self.leader_status.is_some() {
                // Reaping the leader makes escaped descendants orphans, but a
                // signal request is not proof that their entire subtree exited.
                let cleanup = super::agent_process_group::start_time_ms(self.birth)
                    .ok_or_else(|| std::io::Error::other("process launch clock unavailable"))
                    .and_then(|launch| {
                        super::agent_process_group::reap_orphans(
                            &[(self.wake.clone(), launch)].into(),
                        )
                    });
                if let Err(error) = cleanup {
                    self.fail_settlement();
                    return Err(error);
                }
                self.status = self.leader_status;
            }
        }
        Ok(self.status)
    }
}

type Owned = Arc<std::sync::Mutex<OwnedChild>>;

#[derive(Default)]
pub(crate) struct WatchedProcesses {
    live: std::sync::Mutex<HashMap<String, Owned>>,
    /// Kernel shutdown: outcomes are left to restart recovery so no provider
    /// delivery starts while the kernel is going away.
    stopping: std::sync::atomic::AtomicBool,
}

impl WatchedProcesses {
    pub(crate) fn count(&self) -> usize {
        self.live.lock().map(|l| l.len()).unwrap_or(0)
    }
    pub(super) fn contains(&self, id: &str) -> bool {
        self.live.lock().is_ok_and(|l| l.contains_key(id))
    }
    /// Signals the owned session only while its leader is unreaped, so the
    /// group id cannot have been reused; SIGKILL follows after a grace period.
    /// A transient ownership-check failure is retried rather than dropped.
    pub(super) fn terminate(&self, id: &str) -> bool {
        let Some(child) = self.live.lock().ok().and_then(|l| l.get(id).cloned()) else {
            return false;
        };
        if settled(&child) {
            return false;
        }
        std::thread::spawn(move || {
            let grace = Instant::now() + Duration::from_secs(3);
            let mut asked = false;
            while Instant::now() < grace && !settled(&child) {
                asked = asked || signal_owned_session(&child, libc::SIGTERM);
                std::thread::sleep(Duration::from_millis(200));
            }
            for _ in 0..50 {
                if settled(&child) || signal_owned_session(&child, libc::SIGKILL) {
                    return;
                }
                std::thread::sleep(Duration::from_millis(200));
            }
        });
        true
    }
    /// Kernel shutdown: no watched session outlives its supervisor.
    pub(super) fn stopping(&self) -> bool {
        self.stopping.load(Ordering::Acquire)
    }
    pub(super) fn shutdown(&self) {
        self.stopping.store(true, Ordering::Release);
        let owned: Vec<_> = match self.live.lock() {
            Ok(live) => live.keys().cloned().collect(),
            Err(_) => return,
        };
        for id in &owned {
            self.terminate(id);
        }
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline
            && self
                .live
                .lock()
                .is_ok_and(|live| live.values().any(|child| !settled_now(child)))
        {
            std::thread::sleep(Duration::from_millis(100));
        }
    }
}

fn settled(child: &Owned) -> bool {
    child.lock().map_or(true, |c| c.status.is_some() || c.lost)
}

/// Reaps a settled session so shutdown does not wait on the supervisor thread.
fn settled_now(child: &Owned) -> bool {
    child
        .lock()
        .map_or(true, |mut c| matches!(c.poll(), Ok(Some(_))))
}

fn signal_owned_session(child: &Owned, signal: libc::c_int) -> bool {
    let Ok(mut child) = child.lock() else {
        return false;
    };
    if child.status.is_some() || child.leader_status.is_some() || child.lost {
        return false;
    }
    let signalled = signal_session(child.child.id(), child.birth, signal);
    if !signalled {
        child.fail_settlement();
    }
    signalled
}

/// Builds the launch through the same managed isolation every other
/// agent-launched command uses, then adds kernel session ownership and the
/// agent's resource domain.
fn command(
    wake: &str,
    argv: &[String],
    cwd: &Path,
    env: &BTreeMap<String, String>,
    removed: &[String],
    resources: &crate::runtime::process_resources::PreparedDomain,
) -> std::io::Result<Command> {
    let mut environment = BTreeMap::new();
    for key in ENV_ALLOW {
        if removed.iter().any(|r| r == key) {
            continue;
        }
        if let Some(value) = env.get(*key).cloned().or_else(|| std::env::var(key).ok()) {
            environment.insert(key.to_string(), value);
        }
    }
    environment.insert(
        super::agent_process_group::WAKE_MARKER_ENV.into(),
        wake.into(),
    );
    let launch = crate::provider::managed_isolated_utility_launch(
        argv[0].clone(),
        argv[1..].to_vec(),
        environment,
        Some(cwd.to_path_buf()),
        "agent-watched-process",
    )
    .map_err(std::io::Error::other)?;
    let program = launch
        .pty_program
        .ok_or_else(|| std::io::Error::other("watched process launch has no executable"))?;
    let (program, args) = resources.command(program, launch.pty_args);
    let mut command = crate::process_spawn::command(program);
    command
        .args(args)
        .env_clear()
        // Isolation already scrubbed pty_env; its remove list names only
        // inherited variables, which env_clear drops.
        .envs(launch.pty_env)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(directory) = launch.working_directory {
        command.current_dir(directory);
    }
    Ok(command)
}

fn spawn(
    wake: &str,
    argv: &[String],
    cwd: &Path,
    env: &BTreeMap<String, String>,
    removed: &[String],
    resources: &crate::runtime::process_resources::PreparedDomain,
) -> std::io::Result<Child> {
    spawn_owned_session(command(wake, argv, cwd, env, removed, resources)?)
}

/// The child leads its own session, so the kernel owns its whole group.
fn spawn_owned_session(mut command: Command) -> std::io::Result<Child> {
    use std::os::unix::process::CommandExt;
    #[cfg(target_os = "linux")]
    // SAFETY: prctl is async-signal-safe; the child dies with the kernel.
    unsafe {
        command.pre_exec(|| {
            if libc::setsid() < 0 || libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL) != 0 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    crate::process_spawn::spawn_command(command)
}

/// Counts the process under its agent's share of the memory ceiling and lists
/// it in Health. Without monitoring it still runs: the loss is logged.
fn watch_resources(
    resources: crate::runtime::process_resources::PreparedDomain,
    pid: u32,
    wake: &str,
    origin: &Origin,
) -> Option<crate::runtime::process_resources::ProcessDomain> {
    match resources.start(pid) {
        Ok(domain) => {
            domain.bind_key(wake);
            if let Some(agent) = &origin.agent {
                domain.list_as(
                    &origin.session_id,
                    crate::runtime::process_resources::SessionProcessOwner::Agent(agent.clone()),
                );
            }
            Some(domain)
        }
        Err(error) => {
            crate::logging::error_with_fields(
                "agent_process_watch.resources",
                "watched process resource monitoring could not start",
                serde_json::json!({"error": error.to_string()}),
            );
            None
        }
    }
}

/// Supervises one owned session on its own thread until it settles: reaps
/// the leader, drains the output and sends the outcome. A hot reload stops
/// it early, without an outcome, for the next program to take over.
fn supervise(
    owned: Owned,
    readers: Vec<std::thread::JoinHandle<()>>,
    output: Arc<Output>,
    sender: tokio::sync::mpsc::UnboundedSender<Signal>,
) {
    std::thread::spawn(move || {
        let mut alerted = false;
        let mut signalled = None;
        let code = loop {
            if owned.lock().is_ok_and(|c| c.handing_over) {
                // The pipes stay open in the copies the next program reads.
                output.stop();
                for reader in readers {
                    let _ = reader.join();
                }
                if let Ok(mut child) = owned.lock() {
                    child.handed_over = Some(output.snapshot());
                }
                return;
            }
            match owned.lock().map(|mut c| c.poll()) {
                Ok(Ok(Some(status))) => {
                    // Every watched process is launched through `command`, so
                    // its leader is the sandbox that confines it.
                    signalled =
                        crate::provider::state_confinement::confined_program_signal(&status);
                    break Some(
                        status
                            .code()
                            .or_else(|| status.signal().map(|s| 128 + s))
                            .unwrap_or(-1),
                    );
                }
                Ok(Ok(None)) => std::thread::sleep(Duration::from_millis(200)),
                // A failed ownership check cannot establish physical
                // settlement. Keep supervision and the obligation live.
                _ => {
                    if owned.lock().map_or(true, |mut c| {
                        if c.lost {
                            let _ = c.child.try_wait();
                            true
                        } else {
                            false
                        }
                    }) {
                        break None;
                    }
                    if !alerted {
                        let _ = sender.send(Signal::SupervisionFailed);
                        alerted = true;
                    }
                    std::thread::sleep(Duration::from_millis(200));
                }
            }
        };
        // A detached grandchild may keep a pipe open; bound the drain.
        let deadline = Instant::now() + Duration::from_secs(1);
        for reader in &readers {
            while !reader.is_finished() && Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(20));
            }
        }
        output.stop();
        for reader in readers {
            let _ = reader.join();
        }
        // The stopped command may have been the root itself, which the
        // controller cannot tell through its own stream.
        let resources = owned.lock().ok();
        let domain = resources
            .as_ref()
            .and_then(|child| child.resources.as_ref());
        if let Some(notice) = domain.and_then(|domain| domain.memory_stop_notice()) {
            output.push(notice.trim_end().to_owned());
        } else if let Some(kill) = domain.and_then(|domain| domain.oom_kill()) {
            output.push(kill.notice());
        } else if let Some(signal) = signalled {
            output.push(crate::runtime::process_resources::signal_notice(signal));
        }
        drop(resources);
        let _ = sender.send(match code {
                Some(code) => Signal::Exited(code, output.text()),
                None => Signal::Lost("process_lost: physical settlement could not be confirmed after bounded retries; survivors may still be running and require operator cleanup".into()),
            });
    });
}

/// argv of a kernel-launched command: no shell, bounded, no control characters.
pub(super) fn command_argv(value: &serde_json::Value) -> Result<Vec<String>, DaemonError> {
    serde_json::from_value(value.clone())
        .ok()
        .filter(|a: &Vec<String>| {
            !a.is_empty()
                && a.len() <= 64
                && !a[0].is_empty()
                && a.iter()
                    .all(|s| s.len() <= 4_096 && !s.chars().any(char::is_control))
        })
        .ok_or_else(|| {
            crate::durable_state::agent_lifecycle::error(
                "argv of 1-64 arguments required (no shell; control characters are refused)",
            )
        })
}

/// The run's workspace, or a directory inside it named relative to it.
pub(super) fn workspace_cwd(
    run: &crate::provider::RuntimeProviderRun,
    relative: Option<&str>,
) -> Result<PathBuf, DaemonError> {
    let error = crate::durable_state::agent_lifecycle::error;
    let root = run
        .working_directory()
        .and_then(|p| p.canonicalize().ok())
        .ok_or_else(|| error("agent workspace unavailable"))?;
    match relative.filter(|c| !c.is_empty()) {
        None => Ok(root),
        Some(relative) => root
            .join(relative)
            .canonicalize()
            .ok()
            .filter(|p| p.starts_with(&root) && p.is_dir())
            .ok_or_else(|| error("cwd must be a directory inside the agent workspace")),
    }
}

impl KernelRuntimeState {
    /// Sudo replaces command permission, never execution or filesystem mode.
    /// Fetch again after consent: an owner can change modes while it is pending.
    pub(super) fn watched_process_run(
        &self,
        run_id: &str,
    ) -> Result<crate::provider::RuntimeProviderRun, DaemonError> {
        let run = self.owned.provider_store.get_run(run_id)?;
        if run.state() != crate::provider::ProviderRunState::Running
            || run.execution_mode() != crate::provider::AgentExecutionMode::Build
            || run.write_access_mode() != crate::provider::ProviderWriteAccessMode::Unrestricted
        {
            return Err(crate::durable_state::agent_lifecycle::error(
                "watched processes require a current unrestricted build run",
            ));
        }
        Ok(run)
    }

    /// Launch intent is durable before the process starts; a failed or
    /// uncertain launch is reported, never retried automatically.
    pub(super) async fn start_agent_process(
        &self,
        wake: AgentWake,
        argv: Vec<String>,
        cwd: PathBuf,
        task: &str,
        prompt: &str,
        run: &crate::provider::RuntimeProviderRun,
    ) -> Result<AgentWake, DaemonError> {
        // One immutable argv is approved and launched. A caller changing it
        // must return through admission and obtain a new approval.
        if argv != wake.command {
            return Err(crate::durable_state::agent_lifecycle::error(
                "watched process argv differs from its approved command",
            ));
        }
        let _admission = self.owned.begin_managed_activity_admission()?;
        let store = &self.owned.durable_state_store;
        if store.leased_process_receipt(&wake.id)?.is_none() {
            store.agent_lifecycle(Operation::CreateWake {
                task: task.into(),
                prompt: prompt.into(),
                wake: wake.clone(),
            })?;
        }
        if let Err(error) = self.authorize_current_external_command() {
            self.settle_failed_process_launch(
                &wake,
                "process launch authority ended before execution".into(),
            );
            return Err(error);
        }
        let resources = match crate::runtime::process_resources::PreparedDomain::new(
            run.agent_instance_id().unwrap_or(run.id()),
        ) {
            Ok(resources) => resources,
            Err(e) => {
                self.settle_failed_process_launch(&wake, format!("launch failed: {e}"));
                return Err(crate::durable_state::agent_lifecycle::error(format!(
                    "process launch failed: {e}"
                )));
            }
        };
        let spawned = spawn(
            &wake.id,
            &argv,
            &cwd,
            run.pty_env(),
            run.pty_env_remove(),
            &resources,
        );
        let mut child = match spawned {
            Ok(child) => child,
            Err(e) => {
                self.settle_failed_process_launch(&wake, format!("launch failed: {e}"));
                return Err(crate::durable_state::agent_lifecycle::error(format!(
                    "process launch failed: {e}"
                )));
            }
        };
        // Nonblocking pipes let bounded supervision loss stop its readers too.
        use std::os::fd::AsRawFd;
        for fd in [
            child.stdout.as_ref().map(AsRawFd::as_raw_fd),
            child.stderr.as_ref().map(AsRawFd::as_raw_fd),
        ]
        .into_iter()
        .flatten()
        {
            let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
            if flags < 0 || unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0
            {
                // Only the direct child just spawned by this call is eligible.
                if child.id() > 1 {
                    let _ = child.kill();
                    let _ = child.bounded_wait();
                }
                self.settle_failed_process_launch(
                    &wake,
                    "nonblocking output setup failed; launch stopped".into(),
                );
                return Err(crate::durable_state::agent_lifecycle::error(
                    "watched process output setup failed",
                ));
            }
        }
        let (sender, receiver) = tokio::sync::mpsc::unbounded_channel();
        let output = Arc::new(Output::default());
        let origin = Origin {
            unit: run.agent_instance_id().unwrap_or(run.id()).to_string(),
            session_id: run.session_id().to_string(),
            agent: run.agent_instance_id().map(str::to_string),
            protect_room: if store.leased_process_receipt(&wake.id)?.is_some() {
                run.session_id().to_string()
            } else {
                wake.room_id.clone()
            },
        };
        let streams: Vec<OwnedFd> = [
            child.stdout.take().map(OwnedFd::from),
            child.stderr.take().map(OwnedFd::from),
        ]
        .into_iter()
        .flatten()
        .collect();
        // A hot reload hands these over: the readers' own descriptors close
        // with them.
        let Ok(kept) = streams
            .iter()
            .map(OwnedFd::try_clone)
            .collect::<std::io::Result<Vec<_>>>()
        else {
            if child.id() > 1 {
                let _ = child.kill();
                let _ = child.bounded_wait();
            }
            self.settle_failed_process_launch(&wake, "output setup failed; launch stopped".into());
            return Err(crate::durable_state::agent_lifecycle::error(
                "watched process output setup failed",
            ));
        };
        let readers =
            self.drain_streams(streams, &origin, wake.match_text.clone(), &output, &sender);
        let pid = child.id();
        let owned: Owned = Arc::new(std::sync::Mutex::new(
            match OwnedChild::new(child, &wake.id) {
                Ok(mut child) => {
                    child.origin = origin.clone();
                    child.pipes = kept;
                    child.resources = watch_resources(resources, pid, &wake.id, &origin);
                    child
                }
                Err(_) => {
                    self.settle_failed_process_launch(
                        &wake,
                        "process ownership could not be verified; the new child was stopped".into(),
                    );
                    return Err(crate::durable_state::agent_lifecycle::error(
                        "new process ownership could not be verified",
                    ));
                }
            },
        ));
        let started = self.apply_agent_process_receipt(Operation::ProcessStarted {
            id: wake.id.clone(),
            pid,
            now: crate::session::unix_epoch_ms(),
        });
        let activity = self.owned.begin_managed_activity_mutation();
        if let Ok(mut live) = self.owned.agent_wakes.processes.live.lock() {
            live.insert(wake.id.clone(), owned.clone());
        }
        activity.record();
        if started.is_err() {
            self.owned.agent_wakes.processes.terminate(&wake.id);
        }
        supervise(owned.clone(), readers, output, sender);
        self.deliver_process_signals(
            wake.id.clone(),
            wake.room_id.clone(),
            wake.agent_id.clone(),
            receiver,
        );
        started?;
        let started = self
            .owned
            .durable_state_store
            .agent_wakes(Some(&wake.room_id), Some(&wake.agent_id))?
            .into_iter()
            .find(|w| w.id == wake.id)
            .unwrap_or(wake);
        Ok(started)
    }
    /// One reader thread per output pipe, scrubbed by the room's secrets.
    fn drain_streams(
        &self,
        streams: Vec<OwnedFd>,
        origin: &Origin,
        pattern: Option<String>,
        output: &Arc<Output>,
        sender: &tokio::sync::mpsc::UnboundedSender<Signal>,
    ) -> Vec<std::thread::JoinHandle<()>> {
        let protect = room_protector(
            self.owned.room_secret_observations.clone(),
            origin.protect_room.clone(),
        );
        streams
            .into_iter()
            .map(|stream| {
                let (output, pattern, sender) = (output.clone(), pattern.clone(), sender.clone());
                let protect = protect.clone();
                std::thread::spawn(move || {
                    drain(
                        std::fs::File::from(stream),
                        output,
                        pattern,
                        sender,
                        protect,
                    )
                })
            })
            .collect()
    }

    /// Turns one watched process's signals into durable outcomes and wakes,
    /// until it exits.
    fn deliver_process_signals(
        &self,
        id: String,
        notice_room: String,
        notice_agent: String,
        mut receiver: tokio::sync::mpsc::UnboundedReceiver<Signal>,
    ) {
        let state = self.clone();
        tokio::spawn(async move {
            while let Some(signal) = receiver.recv().await {
                if state.owned.agent_wakes.processes.stopping() {
                    break;
                }
                if matches!(signal, Signal::SupervisionFailed) {
                    state.owned.record_notice_for_agent(&notice_room,None,Some(&notice_agent),
                        state.owned.attachment_store.list_session_attachment_ids(&notice_room),
                        "Wake alert: process ownership could not be verified; physical settlement is unconfirmed, supervision is retrying");
                    continue;
                }
                if matches!(signal, Signal::Lost(_)) {
                    // Physical loss must be visible even if durable delivery
                    // is backpressured by the recipient (#914 admission seam).
                    state.owned.record_notice_for_agent(&notice_room, None, Some(&notice_agent),
                        state.owned.attachment_store.list_session_attachment_ids(&notice_room),
                        "Wake alert: watched process is lost/unsettled after bounded retries; survivors may still be running and require operator cleanup");
                }
                let now = crate::session::unix_epoch_ms();
                let (op, exited) = match signal {
                    Signal::Matched(line) => (
                        Operation::ProcessMatched {
                            id: id.clone(),
                            line,
                            now,
                        },
                        false,
                    ),
                    Signal::Exited(code, tail) => (
                        Operation::ProcessExited {
                            id: id.clone(),
                            exit_code: Some(code),
                            tail,
                            now,
                        },
                        true,
                    ),
                    Signal::Lost(tail) => (
                        Operation::ProcessExited {
                            id: id.clone(),
                            exit_code: None,
                            tail,
                            now,
                        },
                        true,
                    ),
                    Signal::SupervisionFailed => unreachable!(),
                };
                if exited {
                    let activity = state.owned.begin_managed_activity_mutation();
                    if let Ok(mut live) = state.owned.agent_wakes.processes.live.lock() {
                        live.remove(&id);
                    }
                    activity.record_prompt_finish_at(None);
                }
                let outcome = state
                    .retain_process_outcome(op, &notice_room, &notice_agent)
                    .await;
                if let crate::durable_state::agent_lifecycle::Outcome::Wakes(wakes) = outcome {
                    for wake in wakes {
                        let text = match (exited, wake.exit_code) {
                            (true, Some(code)) => format!(
                                "Watched process '{}' exited with code {code}",
                                wake.label
                            ),
                            (true, None) => format!("Wake alert: watched process '{}' is lost/unsettled; survivors may still be running and require operator cleanup", wake.label),
                            _ => format!("Watched process '{}' printed its match", wake.label),
                        };
                        state.wake_notice(&wake, text);
                        state.schedule_wake_delivery(&wake);
                    }
                }
                if exited {
                    break;
                }
            }
        });
    }
    pub(super) fn settle_failed_process_launch(&self, wake: &AgentWake, tail: String) {
        let state = self.clone();
        let wake = wake.clone();
        tokio::spawn(async move {
            state
                .retain_process_outcome(
                    Operation::ProcessExited {
                        id: wake.id.clone(),
                        exit_code: None,
                        tail,
                        now: crate::session::unix_epoch_ms(),
                    },
                    &wake.room_id,
                    &wake.agent_id,
                )
                .await;
            state.wake_notice(&wake,format!("Wake alert: watched process '{}' could not be launched or supervised; it is not relaunched",wake.label));
            state.schedule_wake_delivery(&wake);
        });
    }

    async fn retain_process_outcome(
        &self,
        op: Operation,
        room: &str,
        agent: &str,
    ) -> crate::durable_state::agent_lifecycle::Outcome {
        let mut alerted = false;
        loop {
            match self.apply_agent_process_receipt(op.clone()) {
                Ok(outcome) => return outcome,
                Err(_) => {
                    if !alerted {
                        self.owned.record_notice_for_agent(room,None,Some(agent),self.owned.attachment_store.list_session_attachment_ids(room),"Wake alert: watched process outcome could not be committed; retaining it and retrying");
                        alerted = true;
                    }
                    tokio::time::sleep(Duration::from_secs(1)).await;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    pub(super) fn test_resources() -> crate::runtime::process_resources::PreparedDomain {
        crate::runtime::process_resources::PreparedDomain::for_tests()
    }

    use super::*;

    /// Wake markers are host-wide; concurrent tests must not share one.
    fn w() -> String {
        format!(
            "a03-{}-{}",
            std::process::id(),
            std::thread::current().name().unwrap_or("test")
        )
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn a03_signal_guard_rejects_a_group_outside_the_owned_session() {
        use std::os::unix::process::CommandExt;
        let child = crate::process_spawn::command("sleep")
            .arg("30")
            .process_group(0)
            .spawn()
            .unwrap();
        let owned: Owned = Arc::new(std::sync::Mutex::new(OwnedChild::new(child, &w()).unwrap()));
        let refused = !signal_owned_session(&owned, 0);
        let mut child = owned.lock().unwrap();
        assert!(child.child.id() > 1);
        child.child.kill().unwrap();
        child.child.wait().unwrap();
        assert!(
            refused,
            "a handle without an isolated owned session cannot authorize a group signal"
        );
    }

    /// Managed isolation configured for one env-isolated test process.
    #[cfg(target_os = "linux")]
    fn managed_isolation_fixture() -> (PathBuf, PathBuf, PathBuf) {
        let scratch = std::env::temp_dir().join(format!("chariox-a03-iso-{}", std::process::id()));
        let (home, provider, workspace) = (
            scratch.join("chariox-home"),
            scratch.join("provider-home"),
            scratch.join("workspace"),
        );
        for dir in [&home.join("state"), &provider, &workspace] {
            std::fs::create_dir_all(dir).unwrap();
        }
        std::fs::write(home.join("state/vault"), "protected").unwrap();
        std::env::set_var("CHARIOX_MANAGED_PROVIDER_ISOLATION", "1");
        std::env::set_var("CHARIOX_MANAGED_PROVIDER_HOME", &provider);
        std::env::set_var("CHARIOX_MANAGED_PROVIDER_BWRAP", "/usr/bin/bwrap");
        std::env::set_var("CHARIOX_HOME", &home);
        (scratch, home, workspace.canonicalize().unwrap())
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn a03_watched_process_launches_through_managed_isolation() {
        crate::test_support::isolated_env_test!();
        let (scratch, _home, workspace) = managed_isolation_fixture();
        let argv: Vec<String> = vec!["/bin/sh".into(), "-c".into(), "true".into()];
        let removed: Vec<String> = ENV_ALLOW.iter().map(|k| k.to_string()).collect();
        let resources = test_resources();
        let watched = command(
            &w(),
            &argv,
            &workspace,
            &BTreeMap::new(),
            &removed,
            &resources,
        )
        .unwrap();
        let managed = crate::provider::managed_isolated_utility_command(
            "/bin/sh",
            argv[1..].to_vec(),
            BTreeMap::from([(
                super::super::agent_process_group::WAKE_MARKER_ENV.to_string(),
                w(),
            )]),
            Some(workspace.clone()),
            "managed-command",
        )
        .unwrap();
        std::env::remove_var("CHARIOX_MANAGED_PROVIDER_ISOLATION");
        let _ = std::fs::remove_dir_all(&scratch);
        let args = watched.get_args().collect::<Vec<_>>();
        let managed_args = managed.get_args().collect::<Vec<_>>();
        assert_eq!(
            watched.get_program(),
            "/bin/sh",
            "it joins its resource domain first"
        );
        assert_eq!(
            args[args.len() - managed_args.len() - 1..],
            [std::ffi::OsStr::new("/usr/bin/bwrap")]
                .into_iter()
                .chain(managed_args.iter().copied())
                .collect::<Vec<_>>(),
            "a watched process gets exactly the managed command isolation boundary"
        );
        assert_eq!(watched.get_current_dir(), managed.get_current_dir());
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn a03_unmanaged_watched_process_keeps_the_allowed_environment() {
        crate::test_support::isolated_env_test!();
        std::env::remove_var("CHARIOX_MANAGED_PROVIDER_ISOLATION");
        let env = BTreeMap::from([
            ("XDG_RUNTIME_DIR".to_string(), "/run/user/4242".to_string()),
            ("GH_CONFIG_DIR".to_string(), "/srv/gh".to_string()),
        ]);
        let argv: Vec<String> = vec!["/bin/sh".into(), "-c".into(), "true".into()];
        let resources = test_resources();
        let watched = command(&w(), &argv, &std::env::temp_dir(), &env, &[], &resources).unwrap();
        let envs: BTreeMap<_, _> = watched
            .get_envs()
            .filter_map(|(k, v)| Some((k.to_str()?.to_string(), v?.to_str()?.to_string())))
            .collect();
        for (key, value) in env {
            assert_eq!(envs.get(&key), Some(&value), "{key} reaches the process");
        }
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn a03_managed_watched_process_is_isolated_and_settles_its_namespace() {
        use std::io::BufRead;
        crate::test_support::isolated_env_test!();
        let (scratch, home, workspace) = managed_isolation_fixture();
        let vault = home.join("state/vault");
        let script = format!(
            "test -e {} && echo exposed || echo isolated; sleep 30 & sleep 31",
            vault.display()
        );
        let wake = format!("a03-iso-{}", std::process::id());
        let mut child = spawn(
            &wake,
            &["/bin/sh".into(), "-c".into(), script],
            &workspace,
            &BTreeMap::new(),
            &[],
            &super::tests::test_resources(),
        )
        .unwrap();
        let mut line = String::new();
        std::io::BufReader::new(child.stdout.take().unwrap())
            .read_line(&mut line)
            .unwrap();
        let processes = WatchedProcesses::default();
        let owned: Owned = Arc::new(std::sync::Mutex::new(
            OwnedChild::new(child, &wake).unwrap(),
        ));
        processes
            .live
            .lock()
            .unwrap()
            .insert(wake.clone(), owned.clone());
        let marked = || super::super::agent_process_group::marked_count_for_test(&wake);
        let running = marked();
        assert!(processes.terminate(&wake));
        let deadline = Instant::now() + Duration::from_secs(10);
        while owned.lock().unwrap().poll().ok().flatten().is_none() {
            assert!(Instant::now() < deadline, "isolated session must settle");
            std::thread::sleep(Duration::from_millis(50));
        }
        let survivors = marked();
        let _ = std::fs::remove_dir_all(&scratch);
        assert_eq!(line.trim(), "isolated", "protected state is hidden");
        assert!(running >= 2, "the workload ran inside the sandbox");
        assert_eq!(survivors, 0, "no sandboxed descendant outlives the session");
    }

    #[cfg(target_os = "linux")]
    fn nested_escaped_child(wake: &str) -> Child {
        // The setsid child owns another resisting child. Both retain the marker;
        // the grandchild becomes an eligible orphan only after its parent exits.
        let script = "import os,signal\nif os.fork()==0:\n os.setsid()\n signal.signal(signal.SIGTERM,signal.SIG_IGN)\n r,w=os.pipe()\n if os.fork()==0:\n  os.close(r)\n  os.write(w,b'r')\n  while True: signal.pause()\n os.close(w)\n os.read(r,1)\n print('ready',flush=True)\n while True: signal.pause()\nwhile True: signal.pause()";
        let mut child = spawn(
            wake,
            &["python3".into(), "-c".into(), script.into()],
            &std::env::temp_dir(),
            &BTreeMap::new(),
            &[],
            &super::tests::test_resources(),
        )
        .unwrap();
        let mut line = String::new();
        std::io::BufRead::read_line(
            &mut std::io::BufReader::new(child.stdout.take().unwrap()),
            &mut line,
        )
        .unwrap();
        assert_eq!(line.trim(), "ready");
        child
    }

    #[cfg(target_os = "linux")]
    fn cleanup_nested(wake: &str) {
        use super::super::agent_process_group::{marked_count_for_test, reap_orphans};
        let deadline = Instant::now() + Duration::from_secs(5);
        while marked_count_for_test(wake) != 0 {
            let _ = reap_orphans(&[(wake.to_owned(), 0)].into());
            assert!(
                Instant::now() < deadline,
                "MP-11 R2: own test subtree cleanup"
            );
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn review_r2_restart_confirms_the_nested_escaped_subtree() {
        use super::super::agent_process_group::{marked_count_for_test, reap_orphans};
        let wake = w();
        let mut child = nested_escaped_child(&wake);
        assert!(child.id() > 1);
        child.kill().unwrap();
        child.wait().unwrap();
        let stopped = reap_orphans(&[(wake.clone(), 0)].into());
        let survivors = marked_count_for_test(&wake);
        cleanup_nested(&wake);
        assert_eq!(
            survivors, 0,
            "MP-11 R2: recovery returned while escaped descendants still execute: {stopped:?}"
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn review_r2_cancellation_confirms_the_nested_escaped_subtree() {
        use super::super::agent_process_group::marked_count_for_test;
        let wake = w();
        let child = nested_escaped_child(&wake);
        let owned: Owned = Arc::new(std::sync::Mutex::new(
            OwnedChild::new(child, &wake).unwrap(),
        ));
        // The sandbox that keeps the kernel's state away, the command and its two descendants.
        assert_eq!(marked_count_for_test(&wake), 4);
        assert!(signal_owned_session(&owned, libc::SIGTERM));
        let deadline = Instant::now() + Duration::from_secs(5);
        while owned.lock().unwrap().poll().ok().flatten().is_none() {
            assert!(Instant::now() < deadline, "MP-11 R2: own leader settlement");
            std::thread::sleep(Duration::from_millis(20));
        }
        let survivors = marked_count_for_test(&wake);
        cleanup_nested(&wake);
        assert_eq!(
            survivors, 0,
            "MP-11 R2: physical settlement preceded subtree exit"
        );
    }

    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn review_r3_cancelled_child_retains_delegation_until_process_exit() {
        cancelled_child_retains_resources(false).await;
    }

    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn review_r3_matched_child_retains_delegation_and_workflow_until_process_exit() {
        cancelled_child_retains_resources(true).await;
    }

    #[cfg(target_os = "linux")]
    async fn cancelled_child_retains_resources(matched: bool) {
        use crate::durable_state::agent_lifecycle::{ExecutionState, Outcome};
        let worktree = crate::test_support::TestWorktree::new("review-r3");
        let mut config = crate::config::DaemonConfig::for_tests();
        config.room_agent_tools = true;
        let mut app = crate::DaemonApp::bootstrap(config).unwrap();
        let (room, parent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(worktree.session_request())
            .unwrap();
        crate::test_support::admit_room_test_turn(&mut app, room.id(), parent.id());
        let child = crate::app::KernelSessionService::new(&mut app)
            .spawn_agent(
                crate::agent::CreateAgentRequest::new(room.id(), "dev-stub")
                    .with_parent_agent_id(parent.id()),
            )
            .unwrap();
        let store = app.durable_state_store();
        let parent_task = app
            .sessions()
            .get_session(room.id())
            .unwrap()
            .active_prompt_for_agent(parent.id())
            .unwrap()
            .id()
            .to_owned();
        let now = crate::session::unix_epoch_ms();
        store
            .agent_lifecycle(Operation::Begin {
                owner: room.owner_user_id().into(),
                room: room.id().into(),
                agent: child.id().into(),
                prompt: "child-turn".into(),
                run: Some("run".into()),
                now,
            })
            .unwrap();
        // MP-08 / MP-11: mirror the admitted prompt's explicit correlation.
        // Beginning an arbitrary turn on the same agent grants no delegation.
        store
            .agent_lifecycle(Operation::BindDelegate {
                parent_task,
                child_task: "child-turn".into(),
            })
            .unwrap();
        assert_eq!(
            store
                .agent_tasks(Some(room.id()), Some(parent.id()))
                .unwrap()[0]
                .obligations[0]
                .completion_task_id
                .as_deref(),
            Some("child-turn")
        );
        let wake: AgentWake = serde_json::from_value(serde_json::json!({
            "id":w(), "task_id":"child-turn", "room_id":room.id(), "agent_id":child.id(), "registration_id":format!("completion-{}", w()), "kind":"process", "label":"resisting child", "state":"starting", "created_at_ms":now, "verified_at_ms":null, "next_due_ms":null, "interval_ms":null, "command":["python3"], "match_text":null, "matched_at_ms":null, "pid":null, "exit_code":null, "fire_count":0, "missed_fires":0, "last_fired_at_ms":null, "last_sequence":null, "last_delivery":null, "last_delivered_at_ms":null, "last_acknowledged_at_ms":null, "alerted_sequence":null
        })).unwrap();
        store
            .agent_lifecycle(Operation::CreateWake {
                task: "child-turn".into(),
                prompt: "child-turn".into(),
                wake: wake.clone(),
            })
            .unwrap();
        store
            .agent_lifecycle(Operation::BindWorkflow {
                task: "child-turn".into(),
                prompt: "child-turn".into(),
                run: "watched-workflow".into(),
                node: "node".into(),
            })
            .unwrap();
        let mut process = spawn(&wake.id, &["python3".into(), "-c".into(), "import signal; signal.signal(signal.SIGTERM,signal.SIG_IGN); print('ready',flush=True); signal.pause()".into()], worktree.path(), &BTreeMap::new(), &[], &test_resources()).unwrap();
        let mut line = String::new();
        std::io::BufRead::read_line(
            &mut std::io::BufReader::new(process.stdout.take().unwrap()),
            &mut line,
        )
        .unwrap();
        store
            .agent_lifecycle(Operation::ProcessStarted {
                id: wake.id.clone(),
                pid: process.id(),
                now,
            })
            .unwrap();
        store
            .agent_lifecycle(Operation::Yield {
                task: "child-turn".into(),
                prompt: "child-turn".into(),
                registrations: vec![wake.registration_id.clone()],
                cursor: 0,
                deadline: Some(now + 60_000),
                reason: "watch".into(),
                now,
            })
            .unwrap();
        store
            .agent_lifecycle(Operation::Settle {
                room: room.id().into(),
                agent: child.id().into(),
                prompt: "child-turn".into(),
                run: "run".into(),
                has_answer: true,
                cancelled: false,
                now,
            })
            .unwrap();
        if matched {
            store
                .agent_lifecycle(Operation::ProcessMatched {
                    id: wake.id.clone(),
                    line: "ready".into(),
                    now,
                })
                .unwrap();
        }
        let router = crate::runtime::router::CommandRouter::with_interactive_capacity_from_app(
            Arc::new(tokio::sync::Mutex::new(app)),
            1,
        );
        let state = router.runtime_state();
        let owned: Owned = Arc::new(std::sync::Mutex::new(
            OwnedChild::new(process, &wake.id).unwrap(),
        ));
        state
            .owned
            .agent_wakes
            .processes
            .live
            .lock()
            .unwrap()
            .insert(wake.id.clone(), owned.clone());
        let store = &state.owned.durable_state_store;
        let child_task = store
            .agent_tasks(Some(room.id()), Some(child.id()))
            .unwrap()
            .remove(0);
        assert_eq!(child_task.state, ExecutionState::Waiting);
        let Outcome::Task(cancelled) = store
            .agent_lifecycle(Operation::CancelTask {
                task: child_task.task_id,
                owner: room.owner_user_id().into(),
                revision: child_task.revision,
            })
            .unwrap()
        else {
            panic!()
        };
        state.cancel_agent_task_resources(&cancelled).await.unwrap();
        let after_cancel = store
            .agent_tasks(Some(room.id()), Some(parent.id()))
            .unwrap()[0]
            .obligations[0]
            .status
            .clone();
        state.sweep_agent_lifecycle().await.unwrap();
        let after_sweep = store
            .agent_tasks(Some(room.id()), Some(parent.id()))
            .unwrap()[0]
            .obligations[0]
            .status
            .clone();
        state
            .owned
            .agent_store
            .set_agent_state(child.id(), crate::agent::AgentState::Error)
            .unwrap();
        state.sweep_agent_lifecycle().await.unwrap();
        let after_error_sweep = store
            .agent_tasks(Some(room.id()), Some(parent.id()))
            .unwrap()[0]
            .obligations[0]
            .status
            .clone();
        let retained_child = store
            .agent_tasks(Some(room.id()), Some(parent.id()))
            .unwrap()[0]
            .obligations[0]
            .completion_task_id
            .clone();
        let workflow_unsettled = state
            .owned
            .workflow_agent_tasks_unsettled(room.id(), "watched-workflow")
            .unwrap();
        let still_running = owned.lock().unwrap().poll().unwrap().is_none();
        // Always settle our test process before reporting a regression.
        assert!(signal_owned_session(&owned, libc::SIGKILL));
        let deadline = Instant::now() + Duration::from_secs(5);
        while owned.lock().unwrap().poll().unwrap().is_none() {
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(20));
        }
        store
            .agent_lifecycle(Operation::ProcessExited {
                id: wake.id.clone(),
                exit_code: Some(137),
                tail: String::new(),
                now: crate::session::unix_epoch_ms(),
            })
            .unwrap();
        state.sweep_agent_lifecycle().await.unwrap();
        let settled = store
            .agent_tasks(Some(room.id()), Some(parent.id()))
            .unwrap()[0]
            .obligations[0]
            .status
            .clone();
        assert!(
            workflow_unsettled,
            "MP-11 R3: cancelled workflow still owns the watcher"
        );
        assert!(
            !state
                .owned
                .workflow_agent_tasks_unsettled(room.id(), "watched-workflow")
                .unwrap(),
            "MP-11 R3: physical exit releases the workflow"
        );
        assert!(
            still_running,
            "MP-11 R3: resisting watcher remains physically alive during cancellation"
        );
        assert_eq!(
            retained_child.as_deref(),
            Some("child-turn"),
            "MP-11 R3: cancellation and failure retain the admitted child identity"
        );
        assert_eq!(
            (after_cancel, after_sweep, after_error_sweep),
            ("open".to_owned(), "open".to_owned(), "open".to_owned()),
            "MP-11 R3: cancellation and delegate sweep must retain a child with owned resources"
        );
        assert_ne!(
            settled, "open",
            "MP-11 R3: physical exit releases the delegation"
        );
    }

    /// A yielded agent with one watched process per entry of `argvs`.
    #[cfg(target_os = "linux")]
    pub(super) struct YieldedWatches {
        _worktree: crate::test_support::TestWorktree,
        pub(super) room: String,
        pub(super) agent: String,
        pub(super) state: KernelRuntimeState,
        pub(super) wakes: Vec<AgentWake>,
    }

    #[cfg(target_os = "linux")]
    pub(super) async fn yield_on_watches(argvs: Vec<Vec<String>>) -> YieldedWatches {
        yield_on_watches_with(argvs, WatchRig::default()).await
    }

    /// How the watches of a rig are armed.
    #[cfg(target_os = "linux")]
    #[derive(Default)]
    pub(super) struct WatchRig {
        pub(super) match_text: Option<&'static str>,
        /// A worker's leased process: its wake lives in a receipt, not in the
        /// wake ledger, and no agent yields on it here.
        pub(super) leased: bool,
    }

    #[cfg(target_os = "linux")]
    pub(super) async fn yield_on_watches_with(
        argvs: Vec<Vec<String>>,
        rig: WatchRig,
    ) -> YieldedWatches {
        let worktree = crate::test_support::TestWorktree::new("watched-tail");
        let mut app =
            crate::DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests()).unwrap();
        let (room, agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(worktree.session_request())
            .unwrap();
        let now = crate::session::unix_epoch_ms();
        let store = app.durable_state_store();
        store
            .agent_lifecycle(Operation::Begin {
                owner: room.owner_user_id().into(),
                room: room.id().into(),
                agent: agent.id().into(),
                prompt: "tail-turn".into(),
                run: Some("run".into()),
                now,
            })
            .unwrap();
        let router = crate::runtime::router::CommandRouter::with_interactive_capacity_from_app(
            Arc::new(tokio::sync::Mutex::new(app)),
            1,
        );
        let state = router.runtime_state();
        let run = crate::provider::RuntimeProviderRun::from_control_capability_inference(
            "run",
            room.id().into(),
            Some(agent.id().to_string()),
            "codex".into(),
        );
        let mut wakes = Vec::new();
        for (n, argv) in argvs.into_iter().enumerate() {
            let wake: AgentWake = serde_json::from_value(serde_json::json!({
                "id":format!("{}-{n}", w()), "task_id":"tail-turn", "room_id":room.id(), "agent_id":agent.id(), "registration_id":format!("completion-{}-{n}", w()), "kind":"process", "label":format!("watched-{n}"), "state":"starting", "created_at_ms":now, "verified_at_ms":null, "next_due_ms":null, "interval_ms":null, "command":argv, "match_text":rig.match_text, "matched_at_ms":null, "pid":null, "exit_code":null, "fire_count":0, "missed_fires":0, "last_fired_at_ms":null, "last_sequence":null, "last_delivery":null, "last_delivered_at_ms":null, "last_acknowledged_at_ms":null, "alerted_sequence":null
            })).unwrap();
            if rig.leased {
                let receipt = crate::durable_state::agent_lifecycle::LeasedProcessReceipt {
                    job: None,
                    binding: crate::durable_state::agent_lifecycle::LeasedProcessBinding {
                        worker_kernel_id: "worker".into(),
                        worker_machine_id: "machine".into(),
                        execution_lease_id: "lease".into(),
                        leased_agent_id: "leased".into(),
                        home_prompt_id: "tail-turn".into(),
                        worker_provider_run_id: "run".into(),
                    },
                    command_digest: String::new(),
                    worker_boot_id: "boot".into(),
                    process_start_ticks: None,
                    matched_line: None,
                    output_tail: String::new(),
                    revision: 1,
                    backing_room: room.id().into(),
                    backing_agent: agent.id().into(),
                    wake: wake.clone(),
                };
                store
                    .agent_lifecycle(Operation::LeasedProcess(
                        crate::durable_state::agent_lifecycle::LeasedProcessMutation::Intent(
                            receipt,
                        ),
                    ))
                    .unwrap();
            }
            state
                .start_agent_process(
                    wake.clone(),
                    wake.command.clone(),
                    worktree.path().to_path_buf(),
                    "tail-turn",
                    "tail-turn",
                    &run,
                )
                .await
                .unwrap();
            wakes.push(wake);
        }
        let store = &state.owned.durable_state_store;
        if rig.leased {
            return YieldedWatches {
                room: room.id().into(),
                agent: agent.id().into(),
                _worktree: worktree,
                state,
                wakes,
            };
        }
        store
            .agent_lifecycle(Operation::Yield {
                task: "tail-turn".into(),
                prompt: "tail-turn".into(),
                registrations: wakes.iter().map(|w| w.registration_id.clone()).collect(),
                cursor: 0,
                deadline: Some(now + 60_000),
                reason: "watch".into(),
                now,
            })
            .unwrap();
        store
            .agent_lifecycle(Operation::Settle {
                room: room.id().into(),
                agent: agent.id().into(),
                prompt: "tail-turn".into(),
                run: "run".into(),
                has_answer: true,
                cancelled: false,
                now,
            })
            .unwrap();
        YieldedWatches {
            room: room.id().into(),
            agent: agent.id().into(),
            _worktree: worktree,
            state,
            wakes,
        }
    }

    #[cfg(target_os = "linux")]
    impl YieldedWatches {
        /// The first inbox event of the agent that `pick` returns, within 30 s.
        pub(super) async fn inbox_event<T>(
            &self,
            pick: impl Fn(&crate::durable_state::agent_lifecycle::InboxEvent) -> Option<T>,
        ) -> T {
            let store = &self.state.owned.durable_state_store;
            let deadline = Instant::now() + Duration::from_secs(30);
            loop {
                // The scheduler loop is not running in this rig.
                self.state.deliver_process_waits();
                let events = store.agent_inbox(&self.room, &self.agent, 0).unwrap();
                if let Some(found) = events.iter().find_map(&pick) {
                    return found;
                }
                assert!(
                    Instant::now() < deadline,
                    "the agent was never told: {:?} {:?}",
                    store
                        .agent_wakes(Some(&self.room), Some(&self.agent))
                        .unwrap()
                        .into_iter()
                        .map(|w| (w.state, w.exit_code))
                        .collect::<Vec<_>>(),
                    events.into_iter().map(|e| e.kind).collect::<Vec<_>>()
                );
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        }
    }

    /// Runs `argv` as a watched process of a yielded agent and returns the
    /// output tail its end delivers to the inbox.
    #[cfg(target_os = "linux")]
    async fn tail_of_watched_end(argv: Vec<String>) -> String {
        yield_on_watches(vec![argv])
            .await
            .inbox_event(|event| {
                event.payload["public_answer"]["output_tail"]
                    .as_str()
                    .map(str::to_owned)
            })
            .await
    }

    /// A watched command the memory controller stops reaches the agent with
    /// the reason, not as a bare exit 137.
    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn a_watched_command_stopped_for_memory_delivers_the_reason() {
        crate::test_support::isolated_env_test!();
        std::env::set_var("CHARIOX_PROCESS_MEMORY_LIMIT_MIB", "64");
        let grow =
            "import time\nb=[]\nwhile True:\n b.append(bytearray(8*1048576)); time.sleep(0.1)";
        let tail = tail_of_watched_end(vec!["python3".to_string(), "-c".into(), grow.into()]).await;
        std::env::remove_var("CHARIOX_PROCESS_MEMORY_LIMIT_MIB");
        assert!(
            tail.contains("stopped this command at its owner-set memory cap of 64 MiB"),
            "the agent was not told why: {tail:?}"
        );
    }

    /// A cgroup OOM kill (no kernel policy involved) reads as the memory
    /// controller's work, not as `kill -9`.
    #[cfg(target_os = "linux")]
    #[tokio::test]
    #[ignore = "sets memory.max on a cgroup unit: run with --ignored inside a throwaway delegated cgroup"]
    async fn a_watched_process_killed_by_the_memory_controller_delivers_the_reason() {
        crate::test_support::isolated_env_test!();
        let script = "unit=$(dirname /sys/fs/cgroup$(sed -n 's/^0:://p' /proc/self/cgroup)); \
            echo 0 > $unit/memory.swap.max; echo 134217728 > $unit/memory.max; \
            exec python3 -c 'import time; b=bytearray(512*1024*1024); time.sleep(30)'";
        let tail = tail_of_watched_end(vec!["bash".into(), "-c".into(), script.into()]).await;
        assert!(
            tail.contains("killed: out of memory"),
            "the agent was not told why: {tail:?}"
        );
    }

    /// The av7 stall: the agent waits for a file that a process of its own
    /// would have written, and the memory controller killed that process.
    #[cfg(target_os = "linux")]
    #[tokio::test]
    #[ignore = "sets memory.max on a cgroup unit: run with --ignored inside a throwaway delegated cgroup"]
    async fn an_agent_waiting_on_a_file_is_woken_when_its_own_process_is_oom_killed() {
        crate::test_support::isolated_env_test!();
        let waiting = vec![
            "bash".into(),
            "-c".into(),
            "until [ -f never-written ]; do sleep 0.2; done".into(),
        ];
        let script = "unit=$(dirname /sys/fs/cgroup$(sed -n 's/^0:://p' /proc/self/cgroup)); \
            echo 0 > $unit/memory.swap.max; echo 134217728 > $unit/memory.max; \
            exec python3 -c 'import time; b=bytearray(512*1024*1024); time.sleep(30)'";
        let producer = vec!["bash".into(), "-c".into(), script.into()];
        let rig = yield_on_watches(vec![waiting.clone(), producer]).await;
        let told = rig
            .inbox_event(|event| {
                (event.kind == "scope_process_killed")
                    .then(|| event.payload.to_string())
                    .filter(|payload| payload.contains(&rig.wakes[0].id))
            })
            .await;
        assert!(told.contains("killed: out of memory"), "{told}");
        let waiting_wake = rig
            .state
            .owned
            .durable_state_store
            .agent_wakes(Some(&rig.room), Some(&rig.agent))
            .unwrap()
            .into_iter()
            .find(|wake| wake.id == rig.wakes[0].id)
            .unwrap();
        assert_eq!(
            waiting_wake.state, "running",
            "the wait itself is not ended"
        );
    }

    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn a_signalled_watched_process_names_the_signal() {
        crate::test_support::isolated_env_test!();
        let tail =
            tail_of_watched_end(vec!["bash".into(), "-c".into(), "kill -TERM $$".into()]).await;
        assert!(
            tail.contains("killed by signal 15 (SIGTERM)"),
            "the agent was not told why: {tail:?}"
        );
    }

    #[test]
    fn a03_signal_guard_rejects_reserved_and_invalid_targets() {
        for pid in [0, 1, u32::MAX, (i32::MAX as u32) + 1] {
            assert!(!signal_session(pid, 1, 0), "pid {pid} must be rejected");
        }
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn a03_terminate_settles_the_owned_group_only_while_unreaped() {
        let processes = WatchedProcesses::default();
        let mut child = spawn(
            &w(),
            &["sh".into(), "-c".into(), "echo ready; exec sleep 30".into()],
            &std::env::temp_dir(),
            &BTreeMap::new(),
            &[],
            &super::tests::test_resources(),
        )
        .unwrap();
        // Asked once it runs: a SIGTERM that arrives while the sandbox is still
        // starting its program is dropped, and the SIGKILL after the grace ends it.
        std::io::BufRead::read_line(
            &mut std::io::BufReader::new(child.stdout.take().unwrap()),
            &mut String::new(),
        )
        .unwrap();
        let owned: Owned = Arc::new(std::sync::Mutex::new(OwnedChild::new(child, &w()).unwrap()));
        processes.live.lock().unwrap().insert(w(), owned.clone());
        assert!(processes.terminate(&w()));
        let deadline = Instant::now() + Duration::from_secs(5);
        let status = loop {
            if let Some(status) = owned.lock().unwrap().poll().unwrap() {
                break status;
            }
            assert!(Instant::now() < deadline, "terminated session must settle");
            std::thread::sleep(Duration::from_millis(20));
        };
        assert_eq!(
            crate::provider::state_confinement::confined_program_signal(&status),
            Some(libc::SIGTERM),
            "the program ends by the SIGTERM, not the SIGKILL that follows"
        );
        assert!(
            !processes.terminate(&w()),
            "a reaped leader is never signalled"
        );
        assert!(!processes.terminate("unknown"));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn a03_signal_guard_rejects_a_stale_birth_identity() {
        let child = spawn(
            &w(),
            &["sleep".into(), "30".into()],
            &std::env::temp_dir(),
            &BTreeMap::new(),
            &[],
            &super::tests::test_resources(),
        )
        .unwrap();
        let owned: Owned = Arc::new(std::sync::Mutex::new(OwnedChild::new(child, &w()).unwrap()));
        owned.lock().unwrap().birth += 1;
        assert!(
            !signal_owned_session(&owned, 0),
            "a reused or stale process identity must never authorize a signal"
        );
        let mut child = owned.lock().unwrap();
        assert!(child.child.id() > 1);
        child.child.kill().unwrap();
        child.child.wait().unwrap();
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn a03_termination_settles_a_resisting_descendant_after_its_leader_exits() {
        verify_resisting_descendant(false);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn a03_termination_settles_a_descendant_that_changes_its_process_group() {
        verify_resisting_descendant(true);
    }

    #[cfg(target_os = "linux")]
    fn verify_resisting_descendant(change_group: bool) {
        use std::io::BufRead;
        let script = "import os, signal\nr,w=os.pipe()\np=os.fork()\nif p==0:\n os.close(r)\n signal.signal(signal.SIGTERM,signal.SIG_IGN)\n os.write(w,b'r')\n os.close(w)\n while True: signal.pause()\nos.close(w)\nos.read(r,1)\nprint(p,flush=True)\nwhile True: signal.pause()";
        let script = if change_group {
            script.replace(
                "os.close(r)\n signal.signal",
                "os.close(r)\n os.setpgid(0,0)\n signal.signal",
            )
        } else {
            script.into()
        };
        let mut child = spawn(
            &w(),
            &["python3".into(), "-c".into(), script],
            &std::env::temp_dir(),
            &BTreeMap::new(),
            &[],
            &super::tests::test_resources(),
        )
        .unwrap();
        let mut line = String::new();
        std::io::BufReader::new(child.stdout.take().unwrap())
            .read_line(&mut line)
            .unwrap();
        let descendant: libc::pid_t = line.trim().parse().unwrap();
        assert!(descendant > 1);
        // A pidfd pins this test-created descendant for cleanup even if the
        // assertion fails; never use an unverified PID or group fallback.
        let fd = unsafe { libc::syscall(libc::SYS_pidfd_open, descendant, 0) as libc::c_int };
        assert!(fd >= 0);
        let owned: Owned = Arc::new(std::sync::Mutex::new(OwnedChild::new(child, &w()).unwrap()));
        assert!(signal_owned_session(&owned, libc::SIGTERM));
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut status = None;
        while Instant::now() < deadline {
            let mut child = owned.lock().unwrap();
            status = child.poll().ok().flatten();
            if status.is_some() {
                break;
            }
            drop(child);
            std::thread::sleep(Duration::from_millis(20));
        }
        let mut pfd = libc::pollfd {
            fd,
            events: libc::POLLIN,
            revents: 0,
        };
        let settled = unsafe { libc::poll(&mut pfd, 1, 1_000) > 0 };
        if !settled {
            // fd came from the reserved-target-checked child above.
            unsafe {
                libc::syscall(
                    libc::SYS_pidfd_send_signal,
                    fd,
                    libc::SIGKILL,
                    std::ptr::null::<libc::siginfo_t>(),
                    0,
                );
            }
        }
        unsafe {
            libc::close(fd);
        }
        if status.is_none() {
            let mut child = owned.lock().unwrap();
            assert!(child.child.id() > 1);
            let _ = child.child.kill();
            let _ = child.child.wait();
        }
        assert!(
            status.is_some() && settled,
            "leader exit is not settlement while an owned descendant still executes"
        );
    }
}

#[cfg(all(test, target_os = "linux"))]
mod security_tests {
    use super::*;

    #[test]
    fn security_f11_inherited_marker_does_not_authorize_an_unrelated_child() {
        use super::super::agent_process_group::reap_orphans;
        let wake = format!("security-unrelated-{:032x}", rand::random::<u128>());
        // An unrelated server starts a new session with an inherited marker.
        // Its live parent is this test, not the watched session or PID 1.
        let mut child = spawn(
            &wake,
            &["sleep".into(), "30".into()],
            &std::env::temp_dir(),
            &BTreeMap::new(),
            &[],
            &super::tests::test_resources(),
        )
        .unwrap();
        let stopped = reap_orphans(&BTreeMap::from([(wake, 0)])).unwrap();
        std::thread::sleep(Duration::from_millis(50));
        let survived = child.try_wait().unwrap().is_none();
        if survived {
            assert!(child.id() > 1);
            child.kill().unwrap();
            child.wait().unwrap();
        }
        assert!(
            stopped.is_empty() && survived,
            "a marker alone must not authorize killing a live unrelated server's child"
        );
    }

    #[test]
    fn security_f11_failed_ownership_has_a_bounded_lost_outcome() {
        let wake = format!("security-unsettled-{:032x}", rand::random::<u128>());
        let child = spawn(
            &wake,
            &["sleep".into(), "30".into()],
            &std::env::temp_dir(),
            &BTreeMap::new(),
            &[],
            &super::tests::test_resources(),
        )
        .unwrap();
        let mut owned = OwnedChild::new(child, &wake).unwrap();
        owned.birth += 1;
        let mut last = String::new();
        for _ in 0..60 {
            if let Err(error) = owned.poll() {
                last = error.to_string();
            }
        }
        assert!(owned.child.id() > 1);
        let _ = owned.child.kill();
        let _ = owned.child.wait();
        assert!(
            last.contains("process_lost"),
            "supervision must leave endless retry after a bound: {last}"
        );
    }

    #[test]
    fn security_f11_signal_failure_does_not_skip_later_members() {
        crate::test_support::isolated_env_test!();
        use std::os::fd::AsRawFd;
        let wake = format!("security-signal-{:032x}", rand::random::<u128>());
        let child = spawn(
            &wake,
            &[
                "/bin/sh".into(),
                "-c".into(),
                "sleep 30 & echo ready; wait".into(),
            ],
            &std::env::temp_dir(),
            &BTreeMap::new(),
            &[],
            &super::tests::test_resources(),
        )
        .unwrap();
        let mut owned = OwnedChild::new(child, &wake).unwrap();
        let mut line = String::new();
        std::io::BufRead::read_line(
            &mut std::io::BufReader::new(owned.child.stdout().unwrap()),
            &mut line,
        )
        .unwrap();
        let fd = std::fs::File::open("/dev/null").unwrap();
        let first_pin = fd.as_raw_fd();
        drop(fd);
        // Only deny SIGUSR1 on the first pinned fd, in this isolated test
        // process. Cleanup SIGKILL and every other syscall remain allowed.
        let ins = |code, jt, jf, k| libc::sock_filter { code, jt, jf, k };
        let mut filter = [
            ins(0x20, 0, 0, 0),
            ins(0x15, 0, 5, libc::SYS_pidfd_send_signal as u32),
            ins(0x20, 0, 0, 16),
            ins(0x15, 0, 3, first_pin as u32),
            ins(0x20, 0, 0, 24),
            ins(0x15, 0, 1, libc::SIGUSR1 as u32),
            ins(0x06, 0, 0, libc::SECCOMP_RET_ERRNO | libc::EPERM as u32),
            ins(0x06, 0, 0, libc::SECCOMP_RET_ALLOW),
        ];
        let program = libc::sock_fprog {
            len: filter.len() as u16,
            filter: filter.as_mut_ptr(),
        };
        assert_eq!(
            unsafe { libc::prctl(libc::PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) },
            0
        );
        assert_eq!(
            unsafe { libc::prctl(libc::PR_SET_SECCOMP, libc::SECCOMP_MODE_FILTER, &program) },
            0
        );
        let before = super::super::agent_process_group::marked_count_for_test(&wake);
        let complete = signal_session(owned.child.id(), owned.birth, libc::SIGUSR1);
        std::thread::sleep(Duration::from_millis(100));
        let after = super::super::agent_process_group::marked_count_for_test(&wake);
        signal_session(owned.child.id(), owned.birth, libc::SIGKILL);
        let deadline = Instant::now() + Duration::from_secs(3);
        while owned.poll().ok().flatten().is_none() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(20));
        }
        if owned.status.is_none() {
            assert!(owned.child.id() > 1);
            let _ = owned.child.kill();
            let _ = owned.child.wait();
        }
        assert!(!complete, "the aggregate result must retain the EPERM");
        assert!(
            after < before,
            "later pinned members must still receive their signal ({before}->{after})"
        );
    }
}
