use std::collections::{BTreeMap, BTreeSet};
use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, Mutex};
use std::thread;

use portable_pty::{native_pty_system, Child, CommandBuilder, MasterPty, PtySize};
use tokio::sync::Notify;

use crate::error::DaemonError;
use crate::provider::RuntimeProviderRun;

const PTY_OUTPUT_QUEUE_LIMIT: usize = 1024;

pub struct PtyManager {
    process_aliases: BTreeMap<String, String>,
    processes: BTreeMap<String, PtyProcess>,
    output_signal: PtyOutputSignal,
}

pub struct PtySpawnRequest {
    pub process_key: String,
    pub provider_run_id: String,
    pub program: String,
    pub args: Vec<String>,
    pub env: BTreeMap<String, String>,
    pub env_remove: Vec<String>,
    pub working_directory: Option<PathBuf>,
    pub cols: u16,
    pub rows: u16,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PtyOutputChunk {
    pub provider_run_id: String,
    pub bytes: Vec<u8>,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct PtyOutputSignal {
    inner: Arc<PtyOutputSignalState>,
}

#[derive(Debug, Default)]
struct PtyOutputSignalState {
    sequence: AtomicU64,
    notify: Notify,
    provider_runs_by_process: Mutex<BTreeMap<String, BTreeSet<String>>>,
    preferred_provider_run_by_process: Mutex<BTreeMap<String, String>>,
    ready_processes: Mutex<BTreeSet<String>>,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum PtyProcessState {
    Running,
    Exited,
}

struct PtyProcess {
    child: Box<dyn Child + Send + Sync>,
    master: Box<dyn MasterPty + Send>,
    writer: Box<dyn Write + Send>,
    output_rx: Receiver<Vec<u8>>,
    exited: bool,
    reference_count: usize,
}

impl PtyManager {
    pub fn new() -> Self {
        Self {
            process_aliases: BTreeMap::new(),
            processes: BTreeMap::new(),
            output_signal: PtyOutputSignal::default(),
        }
    }

    pub(crate) fn output_signal(&self) -> PtyOutputSignal {
        self.output_signal.clone()
    }

    pub fn spawn_for_run(&mut self, run: &RuntimeProviderRun) -> Result<(), DaemonError> {
        if let Some(process_key) = self.process_aliases.get(run.id()) {
            self.output_signal.prefer_alias(process_key, run.id());
            return Ok(());
        }

        let process_key = run.pty_target().unwrap_or(run.id()).to_string();
        if let Some(process) = self.processes.get_mut(&process_key) {
            process.reference_count += 1;
            self.process_aliases
                .insert(run.id().to_string(), process_key.clone());
            self.output_signal
                .register_alias(&process_key, run.id().to_string());
            return Ok(());
        }

        let program = run.pty_program().ok_or_else(|| DaemonError::PtySpawn {
            provider_run_id: run.id().to_string(),
            message: "provider run does not define a managed PTY program".to_string(),
        })?;

        let mut env = run.pty_env().clone();
        env.insert(
            "ARROBA_MANAGED_PROVIDER_PROCESS".to_string(),
            "1".to_string(),
        );
        env.insert("ARROBA_PROVIDER_RUN_ID".to_string(), run.id().to_string());
        env.insert(
            "ARROBA_PROVIDER_PROCESS_KEY".to_string(),
            process_key.clone(),
        );

        let request = PtySpawnRequest {
            process_key,
            provider_run_id: run.id().to_string(),
            program: program.to_string(),
            args: run.pty_args().to_vec(),
            env,
            env_remove: run.pty_env_remove().to_vec(),
            working_directory: run.working_directory().cloned(),
            cols: 120,
            rows: 40,
        };

        self.spawn(request)
    }

    pub fn spawn(&mut self, request: PtySpawnRequest) -> Result<(), DaemonError> {
        if let Some(process_key) = self.process_aliases.get(&request.provider_run_id) {
            self.output_signal
                .prefer_alias(process_key, &request.provider_run_id);
            return Ok(());
        }
        if let Some(process) = self.processes.get_mut(&request.process_key) {
            process.reference_count += 1;
            self.process_aliases
                .insert(request.provider_run_id.clone(), request.process_key.clone());
            self.output_signal
                .register_alias(&request.process_key, request.provider_run_id);
            return Ok(());
        }

        let pty_system = native_pty_system();
        let pair = pty_system
            .openpty(PtySize {
                rows: request.rows,
                cols: request.cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|error| DaemonError::PtySpawn {
                provider_run_id: request.provider_run_id.clone(),
                message: error.to_string(),
            })?;

        let mut command = CommandBuilder::new(request.program);
        for arg in request.args {
            command.arg(arg);
        }
        for (name, _) in std::env::vars() {
            if crate::secret::secret_like_env_name(&name) {
                command.env_remove(name);
            }
        }
        for key in request.env_remove {
            command.env_remove(key);
        }
        for (key, value) in request.env {
            command.env(key, value);
        }
        if let Some(working_directory) = request.working_directory {
            command.cwd(working_directory);
        }

        let child = pair
            .slave
            .spawn_command(command)
            .map_err(|error| DaemonError::PtySpawn {
                provider_run_id: request.provider_run_id.clone(),
                message: error.to_string(),
            })?;

        let mut reader = pair
            .master
            .try_clone_reader()
            .map_err(|error| DaemonError::PtySpawn {
                provider_run_id: request.provider_run_id.clone(),
                message: error.to_string(),
            })?;
        let writer = pair
            .master
            .take_writer()
            .map_err(|error| DaemonError::PtySpawn {
                provider_run_id: request.provider_run_id.clone(),
                message: error.to_string(),
            })?;

        let (output_tx, output_rx) = mpsc::sync_channel(PTY_OUTPUT_QUEUE_LIMIT);

        self.output_signal
            .register_alias(&request.process_key, request.provider_run_id.clone());
        let output_signal = self.output_signal.clone();
        let output_process_key = request.process_key.clone();
        thread::spawn(move || {
            let mut buffer = [0_u8; 4096];

            while let Ok(size) = reader.read(&mut buffer) {
                if size == 0 {
                    break;
                }

                if output_tx.send(buffer[..size].to_vec()).is_err() {
                    break;
                }
                output_signal.record_output(&output_process_key);
            }
            output_signal.record_output(&output_process_key);
        });

        self.processes.insert(
            request.process_key.clone(),
            PtyProcess {
                child,
                master: pair.master,
                writer,
                output_rx,
                exited: false,
                reference_count: 1,
            },
        );
        self.process_aliases
            .insert(request.provider_run_id, request.process_key);

        Ok(())
    }

    pub fn write_input(&mut self, provider_run_id: &str, input: &[u8]) -> Result<(), DaemonError> {
        let process_key = self.resolve_process_key(provider_run_id)?;
        let process = self.processes.get_mut(&process_key).ok_or_else(|| {
            DaemonError::PtyProcessNotFound {
                provider_run_id: provider_run_id.to_string(),
            }
        })?;

        process
            .writer
            .write_all(input)
            .and_then(|_| process.writer.flush())
            .map_err(|error| DaemonError::PtyWrite {
                provider_run_id: provider_run_id.to_string(),
                message: error.to_string(),
            })
    }

    pub fn resize(
        &mut self,
        provider_run_id: &str,
        cols: u16,
        rows: u16,
    ) -> Result<(), DaemonError> {
        let process_key = self.resolve_process_key(provider_run_id)?;
        let process = self.processes.get_mut(&process_key).ok_or_else(|| {
            DaemonError::PtyProcessNotFound {
                provider_run_id: provider_run_id.to_string(),
            }
        })?;

        process
            .master
            .resize(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|error| DaemonError::PtyResize {
                provider_run_id: provider_run_id.to_string(),
                cols,
                rows,
                message: error.to_string(),
            })
    }

    pub fn drain_output(
        &mut self,
        provider_run_id: &str,
    ) -> Result<Vec<PtyOutputChunk>, DaemonError> {
        let process_key = self.resolve_process_key(provider_run_id)?;
        let process = self.processes.get_mut(&process_key).ok_or_else(|| {
            DaemonError::PtyProcessNotFound {
                provider_run_id: provider_run_id.to_string(),
            }
        })?;

        let mut chunks = Vec::new();

        while let Ok(bytes) = process.output_rx.try_recv() {
            chunks.push(PtyOutputChunk {
                provider_run_id: provider_run_id.to_string(),
                bytes,
            });
        }

        Ok(chunks)
    }

    pub fn poll_process_state(
        &mut self,
        provider_run_id: &str,
    ) -> Result<PtyProcessState, DaemonError> {
        let process_key = self.resolve_process_key(provider_run_id)?;
        let process = self.processes.get_mut(&process_key).ok_or_else(|| {
            DaemonError::PtyProcessNotFound {
                provider_run_id: provider_run_id.to_string(),
            }
        })?;

        if process.exited {
            return Ok(PtyProcessState::Exited);
        }

        let status = process
            .child
            .try_wait()
            .map_err(|error| DaemonError::PtyCleanup {
                provider_run_id: provider_run_id.to_string(),
                message: error.to_string(),
            })?;

        if status.is_some() {
            process.exited = true;
            Ok(PtyProcessState::Exited)
        } else {
            Ok(PtyProcessState::Running)
        }
    }

    pub fn has_process(&self, provider_run_id: &str) -> bool {
        self.process_aliases.contains_key(provider_run_id)
    }

    pub fn process_key(&self, provider_run_id: &str) -> Result<String, DaemonError> {
        self.resolve_process_key(provider_run_id)
    }

    pub fn process_id(&self, provider_run_id: &str) -> Result<Option<u32>, DaemonError> {
        let process_key = self.resolve_process_key(provider_run_id)?;
        let process =
            self.processes
                .get(&process_key)
                .ok_or_else(|| DaemonError::PtyProcessNotFound {
                    provider_run_id: provider_run_id.to_string(),
                })?;
        Ok(process.child.process_id())
    }

    pub fn remove_process(&mut self, provider_run_id: &str) -> Result<bool, DaemonError> {
        let Some(process_key) = self.process_aliases.remove(provider_run_id) else {
            return Ok(false);
        };
        self.output_signal
            .unregister_alias(&process_key, provider_run_id);

        self.remove_process_by_key(&process_key, Some(provider_run_id))
    }

    pub fn remove_process_by_key(
        &mut self,
        process_key: &str,
        provider_run_id: Option<&str>,
    ) -> Result<bool, DaemonError> {
        let Some(process) = self.processes.get_mut(process_key) else {
            return Ok(false);
        };
        if process.reference_count > 1 && provider_run_id.is_some() {
            process.reference_count -= 1;
            return Ok(true);
        }

        self.output_signal.unregister_process(process_key);
        self.process_aliases
            .retain(|_, alias_process_key| alias_process_key != process_key);
        let mut process =
            self.processes
                .remove(process_key)
                .ok_or_else(|| DaemonError::PtyProcessNotFound {
                    provider_run_id: provider_run_id.unwrap_or(process_key).to_string(),
                })?;

        let status = process
            .child
            .try_wait()
            .map_err(|error| DaemonError::PtyCleanup {
                provider_run_id: provider_run_id.unwrap_or(process_key).to_string(),
                message: error.to_string(),
            })?;

        if status.is_none() {
            process
                .child
                .kill()
                .map_err(|error| DaemonError::PtyCleanup {
                    provider_run_id: provider_run_id.unwrap_or(process_key).to_string(),
                    message: error.to_string(),
                })?;
            process
                .child
                .wait()
                .map_err(|error| DaemonError::PtyCleanup {
                    provider_run_id: provider_run_id.unwrap_or(process_key).to_string(),
                    message: error.to_string(),
                })?;
        }

        Ok(true)
    }

    fn resolve_process_key(&self, provider_run_id: &str) -> Result<String, DaemonError> {
        self.process_aliases
            .get(provider_run_id)
            .cloned()
            .ok_or_else(|| DaemonError::PtyProcessNotFound {
                provider_run_id: provider_run_id.to_string(),
            })
    }
}

impl PtyOutputSignal {
    pub(crate) fn sequence(&self) -> u64 {
        self.inner.sequence.load(Ordering::Acquire)
    }

    pub(crate) async fn wait_for_change_after(&self, sequence: u64) {
        if self.sequence() != sequence {
            return;
        }
        let notified = self.inner.notify.notified();
        if self.sequence() != sequence {
            return;
        }
        notified.await;
    }

    pub(crate) fn take_ready_provider_run_ids(&self) -> BTreeSet<String> {
        let ready_processes = {
            let mut ready = self
                .inner
                .ready_processes
                .lock()
                .expect("PTY ready-process set poisoned");
            std::mem::take(&mut *ready)
        };
        let preferred = self
            .inner
            .preferred_provider_run_by_process
            .lock()
            .expect("PTY preferred process-alias map poisoned");
        ready_processes
            .into_iter()
            .filter_map(|process_key| preferred.get(&process_key).cloned())
            .collect()
    }

    fn register_alias(&self, process_key: &str, provider_run_id: String) {
        self.inner
            .provider_runs_by_process
            .lock()
            .expect("PTY process-alias map poisoned")
            .entry(process_key.to_string())
            .or_default()
            .insert(provider_run_id.clone());
        self.inner
            .preferred_provider_run_by_process
            .lock()
            .expect("PTY preferred process-alias map poisoned")
            .insert(process_key.to_string(), provider_run_id);
    }

    fn prefer_alias(&self, process_key: &str, provider_run_id: &str) {
        self.inner
            .preferred_provider_run_by_process
            .lock()
            .expect("PTY preferred process-alias map poisoned")
            .insert(process_key.to_string(), provider_run_id.to_string());
    }

    fn unregister_alias(&self, process_key: &str, provider_run_id: &str) {
        let mut aliases = self
            .inner
            .provider_runs_by_process
            .lock()
            .expect("PTY process-alias map poisoned");
        let fallback = if let Some(provider_runs) = aliases.get_mut(process_key) {
            provider_runs.remove(provider_run_id);
            if provider_runs.is_empty() {
                aliases.remove(process_key);
                None
            } else {
                provider_runs.last().cloned()
            }
        } else {
            None
        };
        drop(aliases);
        let mut preferred = self
            .inner
            .preferred_provider_run_by_process
            .lock()
            .expect("PTY preferred process-alias map poisoned");
        if preferred.get(process_key).map(String::as_str) == Some(provider_run_id) {
            if let Some(fallback) = fallback {
                preferred.insert(process_key.to_string(), fallback);
            } else {
                preferred.remove(process_key);
            }
        }
    }

    fn unregister_process(&self, process_key: &str) {
        self.inner
            .provider_runs_by_process
            .lock()
            .expect("PTY process-alias map poisoned")
            .remove(process_key);
        self.inner
            .preferred_provider_run_by_process
            .lock()
            .expect("PTY preferred process-alias map poisoned")
            .remove(process_key);
        self.inner
            .ready_processes
            .lock()
            .expect("PTY ready-process set poisoned")
            .remove(process_key);
    }

    fn record_output(&self, process_key: &str) {
        self.inner
            .ready_processes
            .lock()
            .expect("PTY ready-process set poisoned")
            .insert(process_key.to_string());
        self.inner.sequence.fetch_add(1, Ordering::AcqRel);
        self.inner.notify.notify_waiters();
    }
}

impl Default for PtyManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use std::thread;
    use std::time::Duration;

    use crate::provider::{
        AgentEndpointMode, LaunchProviderRequest, ProviderLaunchResult, RuntimeProviderRun,
    };

    use super::PtyManager;

    fn test_run() -> RuntimeProviderRun {
        RuntimeProviderRun::new(
            "provider-run-1",
            &LaunchProviderRequest::new(
                "session-1",
                "dev-stub",
                "claude-code",
                "default",
                "sonnet",
            ),
            ProviderLaunchResult {
                endpoint_mode: AgentEndpointMode::Managed,
                process_label: "dev-stub:test".to_string(),
                pty_target: Some("stub-pty:session-1".to_string()),
                pty_program: Some("/bin/sh".to_string()),
                pty_args: vec!["-lc".to_string(), "cat".to_string()],
                pty_env: std::collections::BTreeMap::new(),
                pty_env_remove: Vec::new(),
                working_directory: None,
                structured_endpoint: None,
            },
        )
    }

    fn shared_target_run(id: &str) -> RuntimeProviderRun {
        RuntimeProviderRun::new(
            id,
            &LaunchProviderRequest::new(
                "session-1",
                "dev-stub",
                "claude-code",
                "default",
                "sonnet",
            ),
            ProviderLaunchResult {
                endpoint_mode: AgentEndpointMode::Managed,
                process_label: format!("dev-stub:{id}"),
                pty_target: Some("stub-pty:session-1".to_string()),
                pty_program: Some("/bin/sh".to_string()),
                pty_args: vec!["-lc".to_string(), "cat".to_string()],
                pty_env: std::collections::BTreeMap::new(),
                pty_env_remove: Vec::new(),
                working_directory: None,
                structured_endpoint: None,
            },
        )
    }

    #[test]
    fn spawns_writes_and_resizes_a_pty_process() {
        let run = test_run();
        let mut manager = PtyManager::new();

        manager
            .spawn_for_run(&run)
            .expect("pty-backed provider process should spawn");
        manager
            .resize(run.id(), 100, 30)
            .expect("pty resize should succeed");
        manager
            .write_input(run.id(), b"hello from pty\n")
            .expect("pty input should be written");

        let output = wait_for_output(&mut manager, run.id());

        assert!(!output.is_empty());
        let combined = output
            .into_iter()
            .flat_map(|chunk| chunk.bytes)
            .collect::<Vec<u8>>();
        let combined = String::from_utf8_lossy(&combined);
        assert!(combined.contains("hello from pty"));

        manager
            .remove_process(run.id())
            .expect("pty process cleanup should succeed");
        assert!(!manager.has_process(run.id()));
    }

    #[tokio::test]
    async fn pty_output_signal_wakes_when_reader_enqueues_output() {
        let run = test_run();
        let mut manager = PtyManager::new();

        manager
            .spawn_for_run(&run)
            .expect("pty-backed provider process should spawn");
        let signal = manager.output_signal();
        let sequence = signal.sequence();
        manager
            .write_input(run.id(), b"wake from pty\n")
            .expect("pty input should be written");

        tokio::time::timeout(
            Duration::from_secs(2),
            signal.wait_for_change_after(sequence),
        )
        .await
        .expect("PTY output signal should wake after reader enqueues bytes");
        assert_eq!(
            signal.take_ready_provider_run_ids(),
            [run.id().to_string()].into_iter().collect()
        );
        assert!(signal.take_ready_provider_run_ids().is_empty());

        let output = wait_for_output(&mut manager, run.id());
        let combined = output
            .into_iter()
            .flat_map(|chunk| chunk.bytes)
            .collect::<Vec<u8>>();
        assert!(String::from_utf8_lossy(&combined).contains("wake from pty"));

        manager
            .remove_process(run.id())
            .expect("pty process cleanup should succeed");
    }

    #[test]
    fn reuses_shared_pty_targets_until_last_provider_run_is_removed() {
        let first_run = shared_target_run("provider-run-1");
        let second_run = shared_target_run("provider-run-2");
        let mut manager = PtyManager::new();

        manager
            .spawn_for_run(&first_run)
            .expect("first shared PTY run should spawn");
        manager
            .spawn_for_run(&second_run)
            .expect("second shared PTY run should reuse the existing process");

        assert_eq!(manager.processes.len(), 1);
        assert_eq!(manager.process_aliases.len(), 2);

        manager
            .write_input(second_run.id(), b"shared pty\n")
            .expect("shared PTY should accept input from the second run alias");
        let output = wait_for_output(&mut manager, second_run.id());
        let combined = output
            .into_iter()
            .flat_map(|chunk| chunk.bytes)
            .collect::<Vec<u8>>();
        assert!(String::from_utf8_lossy(&combined).contains("shared pty"));
        assert_eq!(
            manager.output_signal().take_ready_provider_run_ids(),
            [second_run.id().to_string()].into_iter().collect()
        );

        manager
            .remove_process(first_run.id())
            .expect("removing first alias should succeed");
        assert!(!manager.has_process(first_run.id()));
        assert!(manager.has_process(second_run.id()));
        assert_eq!(manager.processes.len(), 1);

        manager
            .remove_process(second_run.id())
            .expect("removing final alias should stop the shared process");
        assert!(!manager.has_process(second_run.id()));
        assert!(manager.processes.is_empty());
        assert!(manager.process_aliases.is_empty());
    }

    fn wait_for_output(
        manager: &mut PtyManager,
        provider_run_id: &str,
    ) -> Vec<super::PtyOutputChunk> {
        let deadline = std::time::Instant::now() + Duration::from_secs(2);

        loop {
            let output = manager
                .drain_output(provider_run_id)
                .expect("pty output should be readable");
            if !output.is_empty() {
                return output;
            }

            assert!(
                std::time::Instant::now() < deadline,
                "timed out waiting for PTY output"
            );
            thread::sleep(Duration::from_millis(10));
        }
    }
}
