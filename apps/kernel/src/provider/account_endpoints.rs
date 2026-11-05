//! Each provider's supervisor owns all account helper children. Request threads
//! send commands and await bounded replies without retaining a shared guard.
use std::collections::BTreeMap;
use std::process::Child;
use std::sync::mpsc::{self, Receiver, SyncSender};
use std::time::{Duration, Instant};

use crate::error::DaemonError;
use crate::runtime::{child_shutdown, stall_watchdog};

pub(super) struct ManagedAccountEndpoint {
    pub endpoint: String,
    pub child: Child,
    #[cfg(target_os = "linux")]
    processes: Option<crate::runtime::owned_process_signals::OwnedProcessHandles>,
}

impl ManagedAccountEndpoint {
    pub fn new(endpoint: String, child: Child) -> Self {
        Self {
            endpoint,
            child,
            #[cfg(target_os = "linux")]
            processes: None,
        }
    }

    fn pin_processes(&mut self) -> Result<(), DaemonError> {
        #[cfg(target_os = "linux")]
        {
            self.processes = Some(
                crate::runtime::owned_process_signals::OwnedProcessSignals::for_child(&self.child)
                    .and_then(|mut signals| signals.pin_owned_processes())
                    .map_err(|error| DaemonError::LocalTransport {
                        operation: "pin_account_helper",
                        message: format!("cannot retain account helper process ownership: {error}"),
                    })?,
            );
        }
        Ok(())
    }

    fn all_exited(&mut self) -> bool {
        let reaped = matches!(self.child.try_wait(), Ok(Some(_)));
        #[cfg(target_os = "linux")]
        return reaped
            && self
                .processes
                .as_ref()
                .is_none_or(|processes| processes.all_exited());
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
    let mut unreaped = Vec::<ManagedAccountEndpoint>::new();
    loop {
        // An uninterruptible child never pins a request thread or the owner.
        unreaped.retain_mut(|endpoint| !endpoint.all_exited());
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
                for (_, endpoint) in endpoints {
                    let _ = stop(name, endpoint, &mut unreaped);
                }
                return;
            }
        }
    }
}

fn ensure(
    name: &'static str,
    endpoints: &mut BTreeMap<String, ManagedAccountEndpoint>,
    unreaped: &mut Vec<ManagedAccountEndpoint>,
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
        if healthy(&endpoint.endpoint) {
            if let Err(error) = endpoint.pin_processes() {
                let _ = stop(name, endpoint, unreaped);
                return Err(error);
            }
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
    unreaped: &mut Vec<ManagedAccountEndpoint>,
) -> Result<(), DaemonError> {
    #[cfg(target_os = "linux")]
    if let Some(processes) = &endpoint.processes {
        if let Err(error) = processes.signal(libc::SIGTERM) {
            report(name, "signal_helper_tree", &error.to_string());
        }
        let deadline = Instant::now() + Duration::from_millis(250);
        while !processes.all_exited() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        if let Err(error) = processes.signal(libc::SIGKILL) {
            report(name, "kill_helper_tree", &error.to_string());
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
            unreaped.push(endpoint);
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
