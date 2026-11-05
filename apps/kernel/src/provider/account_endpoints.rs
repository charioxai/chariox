//! Each provider's supervisor owns all account helper children. Request threads
//! send commands and await bounded replies without retaining a shared guard.
use std::collections::{BTreeMap, VecDeque};
use std::process::Child;
use std::sync::mpsc::{self, Receiver, SyncSender};
use std::time::{Duration, Instant};

use crate::error::DaemonError;
use crate::runtime::{child_shutdown, stall_watchdog};

pub(super) struct ManagedAccountEndpoint {
    pub endpoint: String,
    pub child: Child,
    #[cfg(target_os = "linux")]
    processes: crate::runtime::owned_process_signals::OwnedProcessHandles,
    #[cfg(target_os = "linux")]
    witnesses: Option<crate::runtime::owned_process_signals::OwnedProcessSignals>,
    #[cfg(target_os = "linux")]
    pin_failed: bool,
}

impl ManagedAccountEndpoint {
    pub fn new(endpoint: String, child: Child) -> Self {
        #[cfg(target_os = "linux")]
        let witnesses =
            match crate::runtime::owned_process_signals::OwnedProcessSignals::for_child(&child) {
                Ok(witnesses) => Some(witnesses),
                Err(error) => {
                    report(
                        "account_endpoint",
                        "observe_startup_tree",
                        &error.to_string(),
                    );
                    None
                }
            };
        Self {
            endpoint,
            child,
            #[cfg(target_os = "linux")]
            processes: Default::default(),
            #[cfg(target_os = "linux")]
            witnesses,
            #[cfg(target_os = "linux")]
            pin_failed: true,
        }
    }

    fn pin_processes(&mut self) -> Result<(), DaemonError> {
        #[cfg(target_os = "linux")]
        {
            // Keep birth witnesses and the previous pidfds if a later snapshot
            // or pin fails. Missing pidfds never imply that descendants exited.
            self.pin_failed = true;
            (|| {
                if self.witnesses.is_none() {
                    self.witnesses = Some(
                        crate::runtime::owned_process_signals::OwnedProcessSignals::for_child(
                            &self.child,
                        )?,
                    );
                }
                self.witnesses
                    .as_mut()
                    .expect("captured startup ownership")
                    .pin_owned_processes_into(&mut self.processes)
            })()
            .map_err(|error: std::io::Error| DaemonError::LocalTransport {
                operation: "pin_account_helper",
                message: format!("cannot retain account helper process ownership: {error}"),
            })?;
            self.pin_failed = false;
        }
        Ok(())
    }

    #[cfg(target_os = "linux")]
    fn tree_exited(&self) -> bool {
        self.processes.all_exited()
            && (!self.pin_failed
                || self
                    .witnesses
                    .as_ref()
                    .is_some_and(|witnesses| witnesses.all_exited().unwrap_or(false)))
    }

    fn all_exited(&mut self) -> bool {
        let reaped = matches!(self.child.try_wait(), Ok(Some(_)));
        #[cfg(target_os = "linux")]
        return reaped && self.tree_exited();
        #[cfg(not(target_os = "linux"))]
        reaped
    }
}

type Reply<T> = SyncSender<Result<T, DaemonError>>;
type Launch = Box<dyn FnOnce() -> Result<ManagedAccountEndpoint, DaemonError> + Send>;

enum Command {
    Ensure {
        key: String,
        healthy: fn(&str) -> bool,
        launch: Launch,
        timeout: Duration,
        reply: Reply<String>,
    },
    Invalidate {
        key: String,
        reply: Reply<()>,
    },
    Shutdown {
        reply: Reply<()>,
    },
    #[cfg(test)]
    Install {
        key: String,
        endpoint: ManagedAccountEndpoint,
        reply: Reply<()>,
    },
}

pub(super) struct AccountEndpoints {
    sender: SyncSender<Command>,
    name: &'static str,
}

const REPLY_LIMIT: Duration = Duration::from_secs(30);

impl AccountEndpoints {
    pub fn new(name: &'static str) -> Self {
        let (sender, receiver) = mpsc::sync_channel(64);
        std::thread::Builder::new()
            .name(name.into())
            .spawn(move || supervise(name, receiver))
            .expect("account endpoint supervisor must start");
        Self { sender, name }
    }

    fn request<T>(&self, make: impl FnOnce(Reply<T>) -> Command) -> Result<T, DaemonError> {
        let _watch = stall_watchdog::watch(self.name, "supervisor_reply");
        let (reply, receiver) = mpsc::sync_channel(1);
        self.sender
            .try_send(make(reply))
            .map_err(|error| DaemonError::LocalTransport {
                operation: "account_endpoint_supervisor",
                message: format!("{} command was not accepted: {error}", self.name),
            })?;
        receiver
            .recv_timeout(REPLY_LIMIT)
            .map_err(|error| DaemonError::LocalTransport {
                operation: "account_endpoint_supervisor",
                message: format!("{} reply deadline failed: {error}; retry", self.name),
            })?
    }

    pub fn ensure(
        &self,
        key: String,
        healthy: fn(&str) -> bool,
        timeout: Duration,
        launch: impl FnOnce() -> Result<ManagedAccountEndpoint, DaemonError> + Send + 'static,
    ) -> Result<String, DaemonError> {
        self.request(|reply| Command::Ensure {
            key,
            healthy,
            timeout,
            launch: Box::new(launch),
            reply,
        })
    }

    pub fn invalidate(&self, key: String) {
        if let Err(error) = self.request(|reply| Command::Invalidate { key, reply }) {
            report(self.name, "invalidate", &error.to_string());
        }
    }

    pub fn shutdown(&self) {
        if let Err(error) = self.request(|reply| Command::Shutdown { reply }) {
            report(self.name, "shutdown", &error.to_string());
        }
    }

    #[cfg(test)]
    pub fn install(&self, key: String, endpoint: ManagedAccountEndpoint) {
        self.request(|reply| Command::Install {
            key,
            endpoint,
            reply,
        })
        .unwrap();
    }
}

fn supervise(name: &'static str, receiver: Receiver<Command>) {
    let mut endpoints = BTreeMap::<String, ManagedAccountEndpoint>::new();
    let mut unreaped = VecDeque::<ManagedAccountEndpoint>::new();
    loop {
        // Retry one retained tree per iteration, so transient inspection or FD
        // failures recover without an unbounded sweep ahead of new commands.
        if let Some(mut endpoint) = unreaped.pop_front() {
            if !endpoint.all_exited() {
                if let Err(error) = endpoint.pin_processes() {
                    report(name, "retry_helper_ownership", &error.to_string());
                }
                let _ = stop(name, endpoint, &mut unreaped);
            }
        }
        match receiver.recv_timeout(Duration::from_millis(250)) {
            Ok(Command::Ensure {
                key,
                healthy,
                launch,
                timeout,
                reply,
            }) => {
                let result = ensure(
                    name,
                    &mut endpoints,
                    &mut unreaped,
                    key,
                    healthy,
                    timeout,
                    launch,
                );
                let _ = reply.send(result);
            }
            Ok(Command::Invalidate { key, reply }) => {
                let result = match endpoints.remove(&key) {
                    Some(endpoint) => stop(name, endpoint, &mut unreaped),
                    None => Ok(()),
                };
                let _ = reply.send(result);
            }
            Ok(Command::Shutdown { reply }) => {
                let mut result = Ok(());
                for (_, endpoint) in std::mem::take(&mut endpoints) {
                    if let Err(error) = stop(name, endpoint, &mut unreaped) {
                        result = Err(error);
                    }
                }
                let _ = reply.send(result);
            }
            #[cfg(test)]
            Ok(Command::Install {
                key,
                mut endpoint,
                reply,
            }) => {
                let pinned = endpoint.pin_processes();
                if let Some(old) = endpoints.insert(key, endpoint) {
                    let _ = stop(name, old, &mut unreaped);
                }
                let _ = reply.send(pinned);
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                for (_, endpoint) in std::mem::take(&mut endpoints) {
                    let _ = stop(name, endpoint, &mut unreaped);
                }
                if unreaped.is_empty() {
                    return;
                }
            }
        }
    }
}

fn ensure(
    name: &'static str,
    endpoints: &mut BTreeMap<String, ManagedAccountEndpoint>,
    unreaped: &mut VecDeque<ManagedAccountEndpoint>,
    key: String,
    healthy: fn(&str) -> bool,
    timeout: Duration,
    launch: Launch,
) -> Result<String, DaemonError> {
    let _watch = stall_watchdog::watch(name, "ensure");
    if let Some(endpoint) = endpoints.remove(&key) {
        if healthy(&endpoint.endpoint) {
            let address = endpoint.endpoint.clone();
            endpoints.insert(key, endpoint);
            return Ok(address);
        }
        stop(name, endpoint, unreaped)?;
    }
    let mut endpoint = launch()?;
    let diagnostic =
        super::startup_diagnostic::ProviderStartupDiagnostic::capture(&mut endpoint.child);
    let deadline = Instant::now() + timeout;
    let failure = loop {
        if let Err(error) = endpoint.pin_processes() {
            let _ = stop(name, endpoint, unreaped);
            return Err(error);
        }
        if healthy(&endpoint.endpoint) {
            let address = endpoint.endpoint.clone();
            endpoints.insert(key, endpoint);
            return Ok(address);
        }
        match endpoint.child.try_wait() {
            Ok(Some(status)) => {
                break format!(
                    "helper exited during startup: {status}; {}",
                    diagnostic.summary()
                )
            }
            Err(error) => break format!("helper startup poll failed: {error}"),
            Ok(None) => {}
        }
        if Instant::now() >= deadline {
            break format!("helper startup deadline exceeded; {}", diagnostic.summary());
        }
        std::thread::sleep(Duration::from_millis(100));
    };
    // A failed health check may have outlived a fork by the wrapper. Observe
    // that last startup interval before stopping or reaping its direct child.
    if let Err(error) = endpoint.pin_processes() {
        report(name, "observe_failed_startup", &error.to_string());
    }
    if let Err(error) = stop(name, endpoint, unreaped) {
        report(name, "startup_cleanup", &error.to_string());
    }
    Err(DaemonError::LocalTransport {
        operation: "ensure_account_endpoint",
        message: format!("{name}: {failure}"),
    })
}

fn stop(
    name: &'static str,
    mut endpoint: ManagedAccountEndpoint,
    unreaped: &mut VecDeque<ManagedAccountEndpoint>,
) -> Result<(), DaemonError> {
    #[cfg(target_os = "linux")]
    {
        if let Err(error) = endpoint.processes.signal(libc::SIGTERM) {
            report(name, "signal_helper_tree", &error.to_string());
        }
        if endpoint.pin_failed {
            if let Some(witnesses) = endpoint.witnesses.as_mut() {
                if let Err(error) = witnesses.signal_owned_processes(libc::SIGTERM) {
                    report(name, "signal_unpinned_helper_tree", &error.to_string());
                }
            }
        }
        let deadline = Instant::now() + Duration::from_millis(250);
        while !endpoint.tree_exited() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        if let Err(error) = endpoint.processes.signal(libc::SIGKILL) {
            report(name, "kill_helper_tree", &error.to_string());
        }
    }
    #[cfg(target_os = "linux")]
    if endpoint.pin_failed {
        if let Some(witnesses) = endpoint.witnesses.as_mut() {
            // Pinning can fail after observing descendants (for example under
            // descriptor pressure). Recheck each retained birth before KILL.
            if let Err(error) = witnesses.kill_owned_processes() {
                report(name, "kill_unpinned_helper_tree", &error.to_string());
            }
        }
    }
    #[cfg(all(unix, not(target_os = "linux")))]
    let mut descendants =
        crate::runtime::owned_process_signals::OwnedProcessSignals::for_child(&endpoint.child).ok();
    let result = child_shutdown::stop(&mut endpoint.child, "stop_account_endpoint");
    #[cfg(all(unix, not(target_os = "linux")))]
    if let Some(signals) = descendants.as_mut() {
        if let Err(error) = signals.kill_owned_processes() {
            report(name, "kill_helper_tree", &error.to_string());
        }
    }
    if result.is_ok() {
        let deadline = Instant::now() + Duration::from_secs(1);
        while !endpoint.all_exited() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
    }
    match result {
        Ok(()) if endpoint.all_exited() => Ok(()),
        result => {
            let error = result.err().unwrap_or_else(|| {
                std::io::Error::new(
                    std::io::ErrorKind::TimedOut,
                    "account helper descendants have not exited",
                )
            });
            report(
                name,
                "stop_account_endpoint",
                &format!(
                    "child {}: {error}; supervisor retains reap ownership",
                    endpoint.child.id()
                ),
            );
            unreaped.push_back(endpoint);
            Err(DaemonError::LocalTransport {
                operation: "stop_account_endpoint",
                message: format!("{name}: {error}"),
            })
        }
    }
}

fn report(name: &'static str, operation: &'static str, error: &str) {
    crate::logging::error_with_fields(
        name,
        "account endpoint supervision failed",
        serde_json::json!({"operation": operation, "error": error}),
    );
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;
    use std::io::BufRead;
    use std::os::unix::process::CommandExt;
    use std::process::Stdio;

    fn startup_fixture() -> (
        crate::test_support::TestWorktree,
        Child,
        u32,
        crate::runtime::owned_process_signals::OwnedProcessSignals,
    ) {
        let root = crate::test_support::TestWorktree::new("failed-account-startup");
        let descendant_path = root.path().join("descendant");
        let descendant = format!(
            "import os,signal,time,pathlib; signal.signal(signal.SIGTERM,signal.SIG_IGN); pathlib.Path({:?}).write_text(str(os.getpid())); time.sleep(30)",
            descendant_path.to_string_lossy()
        );
        let script = format!(
            "import signal,time,subprocess,sys,pathlib; signal.signal(signal.SIGTERM,signal.SIG_IGN); subprocess.Popen([sys.executable,'-c',{:?}]); p=pathlib.Path({:?});\nwhile not p.exists(): time.sleep(.01)\nprint('ready',flush=True); time.sleep(30)",
            descendant, descendant_path.to_string_lossy()
        );
        let mut command = crate::process_spawn::command("python3");
        command
            .args(["-c", &script])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        unsafe {
            command.pre_exec(|| {
                if libc::setsid() < 0 {
                    Err(std::io::Error::last_os_error())
                } else {
                    Ok(())
                }
            });
        }
        let mut child = command.spawn().unwrap();
        let mut line = String::new();
        std::io::BufReader::new(child.stdout.take().unwrap())
            .read_line(&mut line)
            .unwrap();
        assert_eq!(line.trim(), "ready");
        let pid: u32 = std::fs::read_to_string(descendant_path)
            .unwrap()
            .parse()
            .unwrap();
        let cleanup =
            crate::runtime::owned_process_signals::OwnedProcessSignals::for_child(&child).unwrap();
        (root, child, pid, cleanup)
    }

    #[test]
    fn failed_readiness_settles_term_ignoring_descendant() {
        let (_root, child, _pid, mut cleanup) = startup_fixture();
        let owner = AccountEndpoints::new("failed-startup-fixture");
        let result = owner.ensure(
            "failed-readiness".into(),
            |_| false,
            Duration::from_millis(150),
            move || Ok(ManagedAccountEndpoint::new("unused".into(), child)),
        );
        let descendant_survived = !cleanup.all_exited().unwrap();
        // The fail-first run must clean its exact observed tree as well.
        cleanup.kill_owned_processes().unwrap();
        assert!(result.is_err(), "readiness must fail");
        assert!(
            !descendant_survived,
            "failed startup abandoned its server descendant"
        );
    }

    #[test]
    fn failed_pidfd_pin_keeps_startup_cleanup_ownership() {
        use crate::runtime::bounded_process::BoundedCommandExt as _;
        let helper = format!(
            "{}::pidfd_failure_helper",
            module_path!().split_once("::").unwrap().1
        );
        let output = crate::process_spawn::command(std::env::current_exe().unwrap())
            .args([
                "--ignored",
                "--exact",
                &helper,
                "--test-threads=1",
                "--nocapture",
            ])
            .output_with_deadline(Duration::from_secs(10))
            .unwrap();
        assert!(
            output.status.success(),
            "{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(
            String::from_utf8_lossy(&output.stdout).contains("1 passed; 0 failed"),
            "the descriptor failure helper must actually run"
        );
    }

    #[test]
    #[ignore = "subprocess-only: changes its own descriptor limit"]
    fn pidfd_failure_helper() {
        let (_root, child, _pid, mut cleanup) = startup_fixture();
        let mut endpoint = ManagedAccountEndpoint::new("unused".into(), child);
        let mut original = std::mem::MaybeUninit::<libc::rlimit>::uninit();
        assert_eq!(
            unsafe { libc::getrlimit(libc::RLIMIT_NOFILE, original.as_mut_ptr()) },
            0
        );
        let original = unsafe { original.assume_init() };
        let limited = libc::rlimit {
            rlim_cur: 0,
            rlim_max: original.rlim_max,
        };
        assert_eq!(unsafe { libc::setrlimit(libc::RLIMIT_NOFILE, &limited) }, 0);
        let failed_pin = endpoint.pin_processes();
        assert_eq!(
            unsafe { libc::setrlimit(libc::RLIMIT_NOFILE, &original) },
            0
        );
        let mut unreaped = VecDeque::new();
        let stopped = stop("pin-failure-fixture", endpoint, &mut unreaped);
        let descendant_survived = !cleanup.all_exited().unwrap();
        cleanup.kill_owned_processes().unwrap();
        assert!(
            failed_pin.is_err(),
            "pin must fail under descriptor exhaustion"
        );
        assert!(
            stopped.is_ok(),
            "owned startup cleanup must recover after a failed pin"
        );
        assert!(unreaped.is_empty());
        assert!(
            !descendant_survived,
            "failed pin lost its startup descendant"
        );
    }
}
