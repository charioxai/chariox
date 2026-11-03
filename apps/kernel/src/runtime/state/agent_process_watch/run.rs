//! MP-08 / MP-09 / MP-10 / MP-11 VAUDIT S2: a kernel command run to completion
//! for a Vault tool. It launches exactly like a watched process (managed
//! isolation, its own session, the agent's resource domain, settled at kernel
//! shutdown) but its raw output goes back to the kernel caller only, which
//! decides what the model may see.
use super::*;
use std::io::Write;
use zeroize::Zeroizing;

/// Raw output kept per stream; a Vault tool never needs more.
const OUTPUT_LIMIT: usize = 1 << 20;
/// Termination asks for SIGTERM, then SIGKILL after 3 s; settlement then has this long.
const SETTLE_LIMIT: Duration = Duration::from_secs(30);

pub(in crate::runtime::state) struct KernelCommand {
    pub(in crate::runtime::state) argv: Vec<String>,
    pub(in crate::runtime::state) cwd: PathBuf,
    /// Added to the launch after isolation, so values never become arguments.
    pub(in crate::runtime::state) env: Vec<(String, Zeroizing<String>)>,
    pub(in crate::runtime::state) stdin: Option<Zeroizing<String>>,
    pub(in crate::runtime::state) timeout: Duration,
}

#[derive(Debug, PartialEq, Eq)]
pub(in crate::runtime::state) enum KernelCommandExit {
    Exited(i32),
    /// The command outlived its timeout and was stopped.
    TimedOut,
    /// Physical settlement could not be confirmed.
    Lost,
}

pub(in crate::runtime::state) struct KernelCommandOutput {
    pub(in crate::runtime::state) exit: KernelCommandExit,
    pub(in crate::runtime::state) stdout: Zeroizing<Vec<u8>>,
    pub(in crate::runtime::state) stderr: Zeroizing<Vec<u8>>,
    pub(in crate::runtime::state) truncated: bool,
}

type Captured = Arc<std::sync::Mutex<(Zeroizing<Vec<u8>>, bool)>>;

fn collect(mut stream: impl Read + Send + 'static) -> (Captured, std::thread::JoinHandle<()>) {
    let captured: Captured = Arc::default();
    let sink = captured.clone();
    let reader = std::thread::spawn(move || {
        let mut chunk = Zeroizing::new([0u8; 8192]);
        while let Ok(read) = stream.read(chunk.as_mut()) {
            if read == 0 {
                break;
            }
            let Ok(mut sink) = sink.lock() else { break };
            let room = OUTPUT_LIMIT.saturating_sub(sink.0.len());
            sink.0.extend_from_slice(&chunk[..read.min(room)]);
            sink.1 |= read > room;
        }
    });
    (captured, reader)
}

fn taken(captured: &Captured) -> (Zeroizing<Vec<u8>>, bool) {
    captured.lock().map_or_else(
        |_| (Zeroizing::new(Vec::new()), true),
        |mut c| (std::mem::take(&mut c.0), c.1),
    )
}

impl KernelRuntimeState {
    /// Blocking: call from a blocking task. The caller has already checked
    /// the run and its authority; this owns launch, timeout and settlement.
    pub(in crate::runtime::state) fn run_kernel_command(
        &self,
        run: &crate::provider::RuntimeProviderRun,
        request: KernelCommand,
    ) -> Result<KernelCommandOutput, DaemonError> {
        let failed = |e: &dyn std::fmt::Display| {
            crate::runtime::kernel_access::error(format!("command launch failed: {e}"))
        };
        let id = format!("vault-command-{:016x}", rand::random::<u64>());
        let resources = crate::runtime::process_resources::PreparedDomain::new(
            run.agent_instance_id().unwrap_or(run.id()),
        )
        .map_err(|e| failed(&e))?;
        let mut command = command(
            &id,
            &request.argv,
            &request.cwd,
            run.pty_env(),
            run.pty_env_remove(),
            &resources,
        )
        .map_err(|e| failed(&e))?;
        for (name, value) in &request.env {
            command.env(name, value.as_str());
        }
        if request.stdin.is_some() {
            command.stdin(Stdio::piped());
        }
        let mut child = spawn_owned_session(command).map_err(|e| failed(&e))?;
        if let (Some(mut pipe), Some(input)) = (child.stdin.take(), request.stdin) {
            // A command that never reads stdin must not stall the kernel.
            std::thread::spawn(move || {
                let _ = pipe.write_all(input.as_bytes());
            });
        }
        let streams = [
            child.stdout.take().map(collect),
            child.stderr.take().map(collect),
        ];
        let pid = child.id();
        let owned: Owned = Arc::new(std::sync::Mutex::new(
            OwnedChild::new(child, &id).map_err(|e| failed(&e))?,
        ));
        if let Ok(domain) = resources.start(pid) {
            domain.bind_key(&id);
            if let Some(agent) = run.agent_instance_id() {
                domain.list_as(
                    run.session_id(),
                    crate::runtime::process_resources::SessionProcessOwner::Agent(agent.into()),
                );
            }
            if let Ok(mut child) = owned.lock() {
                child.resources = Some(domain);
            }
        }
        let processes = &self.owned.agent_wakes.processes;
        if let Ok(mut live) = processes.live.lock() {
            live.insert(id.clone(), owned.clone());
        }
        let deadline = Instant::now() + request.timeout;
        let mut stopped = false;
        let exit = loop {
            match owned.lock().map(|mut c| c.poll()) {
                Ok(Ok(Some(status))) if !stopped => {
                    break KernelCommandExit::Exited(
                        status
                            .code()
                            .or_else(|| status.signal().map(|s| 128 + s))
                            .unwrap_or(-1),
                    )
                }
                Ok(Ok(Some(_))) => break KernelCommandExit::TimedOut,
                Ok(Ok(None)) => {}
                _ if owned.lock().map_or(true, |c| c.lost) => break KernelCommandExit::Lost,
                _ => {}
            }
            let now = Instant::now();
            if !stopped && now >= deadline {
                stopped = true;
                processes.terminate(&id);
            }
            if stopped && now >= deadline + SETTLE_LIMIT {
                break KernelCommandExit::Lost;
            }
            std::thread::sleep(Duration::from_millis(50));
        };
        if exit != KernelCommandExit::Lost {
            if let Ok(mut live) = processes.live.lock() {
                live.remove(&id);
            }
        }
        // A detached grandchild may keep a pipe open; take what arrived.
        let drained = Instant::now() + Duration::from_secs(1);
        for (_, reader) in streams.iter().flatten() {
            while !reader.is_finished() && Instant::now() < drained {
                std::thread::sleep(Duration::from_millis(20));
            }
        }
        let [stdout, stderr] = streams.map(|stream| {
            stream.map_or_else(|| (Zeroizing::new(Vec::new()), false), |(c, _)| taken(&c))
        });
        Ok(KernelCommandOutput {
            exit,
            truncated: stdout.1 || stderr.1,
            stdout: stdout.0,
            stderr: stderr.0,
        })
    }
}
