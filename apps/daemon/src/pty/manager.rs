use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver};
use std::thread;

use portable_pty::{native_pty_system, Child, CommandBuilder, MasterPty, PtySize};

use crate::error::DaemonError;
use crate::provider::RuntimeProviderRun;

pub struct PtyManager {
    processes: BTreeMap<String, PtyProcess>,
}

pub struct PtySpawnRequest {
    pub provider_run_id: String,
    pub program: String,
    pub args: Vec<String>,
    pub working_directory: Option<PathBuf>,
    pub cols: u16,
    pub rows: u16,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PtyOutputChunk {
    pub provider_run_id: String,
    pub bytes: Vec<u8>,
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
}

impl PtyManager {
    pub fn new() -> Self {
        Self {
            processes: BTreeMap::new(),
        }
    }

    pub fn spawn_for_run(&mut self, run: &RuntimeProviderRun) -> Result<(), DaemonError> {
        if self.processes.contains_key(run.id()) {
            return Ok(());
        }

        let request = PtySpawnRequest {
            provider_run_id: run.id().to_string(),
            program: run.pty_program().to_string(),
            args: run.pty_args().to_vec(),
            working_directory: run.working_directory().cloned(),
            cols: 120,
            rows: 40,
        };

        self.spawn(request)
    }

    pub fn spawn(&mut self, request: PtySpawnRequest) -> Result<(), DaemonError> {
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

        let (output_tx, output_rx) = mpsc::channel();

        thread::spawn(move || {
            let mut buffer = [0_u8; 4096];

            while let Ok(size) = reader.read(&mut buffer) {
                if size == 0 {
                    break;
                }

                if output_tx.send(buffer[..size].to_vec()).is_err() {
                    break;
                }
            }
        });

        self.processes.insert(
            request.provider_run_id,
            PtyProcess {
                child,
                master: pair.master,
                writer,
                output_rx,
                exited: false,
            },
        );

        Ok(())
    }

    pub fn write_input(&mut self, provider_run_id: &str, input: &[u8]) -> Result<(), DaemonError> {
        let process = self.processes.get_mut(provider_run_id).ok_or_else(|| {
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
        let process = self.processes.get_mut(provider_run_id).ok_or_else(|| {
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
        let process = self.processes.get_mut(provider_run_id).ok_or_else(|| {
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
        let process = self.processes.get_mut(provider_run_id).ok_or_else(|| {
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
        self.processes.contains_key(provider_run_id)
    }

    pub fn remove_process(&mut self, provider_run_id: &str) -> Result<bool, DaemonError> {
        let mut process = match self.processes.remove(provider_run_id) {
            Some(process) => process,
            None => return Ok(false),
        };

        let status = process
            .child
            .try_wait()
            .map_err(|error| DaemonError::PtyCleanup {
                provider_run_id: provider_run_id.to_string(),
                message: error.to_string(),
            })?;

        if status.is_none() {
            process
                .child
                .kill()
                .map_err(|error| DaemonError::PtyCleanup {
                    provider_run_id: provider_run_id.to_string(),
                    message: error.to_string(),
                })?;
            process
                .child
                .wait()
                .map_err(|error| DaemonError::PtyCleanup {
                    provider_run_id: provider_run_id.to_string(),
                    message: error.to_string(),
                })?;
        }

        Ok(true)
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

    use crate::provider::{LaunchProviderRequest, ProviderLaunchResult, RuntimeProviderRun};

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
                process_label: "dev-stub:test".to_string(),
                pty_target: Some("stub-pty:session-1".to_string()),
                pty_program: "/bin/sh".to_string(),
                pty_args: vec!["-lc".to_string(), "cat".to_string()],
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
