//! Files on this machine for the owner's clients, agents and other machines: transfers run by the
//! kernel, and the file requests another kernel of the owner sends.
use super::*;
use crate::local::{FileTransferJob, FileTransferRef, StartFileTransferRequest};
use crate::machine_files::{
    serve, FileSource, Interrupted, Machine, PeerFiles, PeerLink, StateGuard,
};
use crate::transport::relay_peer::{RelayPeerRequest, RelayPeerResponse};

/// One slice is at most 4 MiB, so the wait covers it over a slow uplink.
const SLICE_TIMEOUT: Duration = Duration::from_secs(90);
/// How long a restarted kernel waits for the machine a transfer reads from to be reachable again.
const RESUME_WINDOW: Duration = Duration::from_secs(15 * 60);
const RESUME_RETRY: Duration = Duration::from_secs(5);

fn refusal(message: impl Into<String>) -> DaemonError {
    DaemonError::MachineRequest {
        code: "file_transfer_refused".into(),
        message: message.into(),
    }
}

impl KernelRuntimeState {
    /// The kernel's own state and its Vault, which Files never touches.
    pub(crate) fn machine_state_guard(&self) -> StateGuard {
        StateGuard::new(Some(
            &self
                .owned
                .config_projection
                .snapshot()
                .user_config
                .credential_vault
                .path,
        ))
    }

    pub(crate) fn this_machine(&self) -> Machine {
        Machine::new(self.machine_state_guard())
    }

    /// Starts a transfer whose destination is this machine. The source is this machine, or the
    /// kernel `request.source.kernel` names, which must be a kernel of the same owner.
    pub(crate) async fn start_file_transfer(
        &self,
        request: StartFileTransferRequest,
    ) -> Result<FileTransferJob, DaemonError> {
        let (source, machine) = self.file_transfer_source(&request).await?;
        Ok(self.file_transfers.start(request, source, machine))
    }

    async fn file_transfer_source(
        &self,
        request: &StartFileTransferRequest,
    ) -> Result<(FileSource, Machine), DaemonError> {
        let machine = self.this_machine();
        let source = match request.source.kernel.as_deref() {
            None => FileSource::Local(machine.clone()),
            Some(reference) => match self.resolve_peer_kernel(reference).await? {
                None => FileSource::Local(machine.clone()),
                Some(kernel) => FileSource::Peer(self.peer_files(&kernel, reference)?),
            },
        };
        Ok((source, machine))
    }

    /// Opens the jobs this kernel kept on disk: they are listed again, and those it stopped under
    /// continue from their staged bytes as soon as the machine they read from can be reached.
    pub(crate) fn resume_file_transfers(&self) {
        let dir = crate::config::default_state_dir().join("file-transfers");
        for interrupted in self.file_transfers.attach(dir) {
            let runtime = self.clone();
            tokio::spawn(async move { runtime.resume_file_transfer(interrupted).await });
        }
    }

    async fn resume_file_transfer(&self, interrupted: Interrupted) {
        let deadline = tokio::time::Instant::now() + RESUME_WINDOW;
        loop {
            match self.file_transfer_source(&interrupted.request).await {
                Ok((source, machine)) => {
                    self.file_transfers.resume(&interrupted, source, machine);
                    return;
                }
                Err(_) if tokio::time::Instant::now() < deadline => {
                    tokio::time::sleep(RESUME_RETRY).await;
                }
                Err(error) => {
                    self.file_transfers.fail(
                        &interrupted.job_id,
                        format!(
                            "The kernel restarted and could not reach the source again: {}. What was copied is kept; start the transfer again to continue it",
                            error_text(error)
                        ),
                    );
                    return;
                }
            }
        }
    }

    pub(crate) fn get_file_transfer(
        &self,
        job: &FileTransferRef,
    ) -> Result<FileTransferJob, DaemonError> {
        self.file_transfers
            .get(&job.job_id)
            .ok_or_else(|| unknown_transfer(&job.job_id))
    }

    pub(crate) fn list_file_transfers(&self) -> Vec<FileTransferJob> {
        self.file_transfers.list()
    }

    pub(crate) fn cancel_file_transfer(
        &self,
        job: &FileTransferRef,
    ) -> Result<FileTransferJob, DaemonError> {
        self.file_transfers
            .cancel(&job.job_id)
            .ok_or_else(|| unknown_transfer(&job.job_id))
    }

    /// The files of `kernel`, a kernel of the same owner, as a source of this kernel's transfers.
    pub(crate) fn peer_files(&self, kernel: &str, name: &str) -> Result<PeerFiles, DaemonError> {
        let owner = self.file_peer_owner()?;
        let (runtime, kernel) = (self.clone(), kernel.to_owned());
        let link: PeerLink = Arc::new(move |request| {
            let (runtime, owner, kernel) = (runtime.clone(), owner.clone(), kernel.clone());
            Box::pin(async move {
                runtime
                    .peer_send(&owner, &kernel, request, SLICE_TIMEOUT)
                    .await
            })
        });
        Ok(PeerFiles::new(format!("The machine `{name}`"), link))
    }

    /// The Cloud account whose kernels this one may reach.
    pub(super) fn file_peer_owner(&self) -> Result<String, DaemonError> {
        let config = self.owned.config_projection.snapshot();
        let owner = config
            .cloud_relay
            .as_ref()
            .map(|profile| profile.user_id.clone())
            .filter(|owner| !owner.is_empty());
        #[cfg(test)]
        let owner = owner.or_else(|| Some("test-owner".to_owned()));
        owner.ok_or_else(|| {
            refusal(
                "this kernel is not signed in to your Chariox account, so it cannot reach your other machines",
            )
        })
    }

    /// Answers a file or transfer request of another kernel of the owner.
    pub(crate) async fn serve_machine_files(
        &self,
        request: RelayPeerRequest,
    ) -> Result<RelayPeerResponse, DaemonError> {
        match request {
            RelayPeerRequest::StartFileTransfer { request } => {
                if request.source.kernel.is_none() {
                    return Ok(RelayPeerResponse::FileRefused {
                        reason: "name the kernel that holds the source".into(),
                    });
                }
                Ok(match self.start_file_transfer(request).await {
                    Ok(job) => RelayPeerResponse::FileTransfer { job },
                    Err(error) => RelayPeerResponse::FileRefused {
                        reason: error_text(error),
                    },
                })
            }
            RelayPeerRequest::GetFileTransfer { job_id } => Ok(self
                .get_file_transfer(&FileTransferRef { job_id })
                .map_or_else(refused, |job| RelayPeerResponse::FileTransfer { job })),
            RelayPeerRequest::CancelFileTransfer { job_id } => Ok(self
                .cancel_file_transfer(&FileTransferRef { job_id })
                .map_or_else(refused, |job| RelayPeerResponse::FileTransfer { job })),
            request => {
                let operation = self.begin_file_operation()?;
                let machine = self.this_machine();
                tokio::task::spawn_blocking(move || {
                    let _operation = operation;
                    serve(&machine, request)
                })
                .await
                .map_err(|_| DaemonError::LocalTransport {
                    operation: "serve machine files",
                    message: "the file request stopped unexpectedly".into(),
                })
            }
        }
    }
}

fn refused(error: DaemonError) -> RelayPeerResponse {
    RelayPeerResponse::FileRefused {
        reason: error_text(error),
    }
}

pub(crate) fn error_text(error: DaemonError) -> String {
    match error {
        DaemonError::MachineRequest { message, .. } => message,
        other => other.to_string(),
    }
}

fn unknown_transfer(job_id: &str) -> DaemonError {
    refusal(format!(
        "no transfer `{job_id}` on this kernel; it may have been forgotten after it ended"
    ))
}
