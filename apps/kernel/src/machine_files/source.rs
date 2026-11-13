//! Where a transfer reads from: this machine, or another machine of the owner over the relay.
//! `serve` is the other side of the second: the machine that holds the files answers a kernel of
//! the same owner, one request at a time.
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use super::{Machine, StateGuard};
use crate::agent_file_copy::{read_slice, LinkError, RemoteSource, SourceSlice};
use crate::error::DaemonError;
use crate::local::{
    KernelDirectoryEntry, KernelFileStat, ListKernelDirectoryRequest, LocalDaemonResponse,
    StatKernelFileRequest,
};
use crate::runtime::kernel_files;
use crate::transport::relay_peer::{RelayPeerRequest, RelayPeerResponse, RemoteFileBytes};

type Answer = Pin<Box<dyn Future<Output = Result<RelayPeerResponse, DaemonError>> + Send>>;

/// Sends one request to the kernel that holds the source files.
pub(crate) type PeerLink = Arc<dyn Fn(RelayPeerRequest) -> Answer + Send + Sync>;

/// Worker answers that no retry can change.
const DEFINITIVE_LINK_CODES: [&str; 3] = [
    "unauthorized",
    "invalid_request",
    "kernel_runtime_role_denied",
];

/// The files of another kernel of the owner.
#[derive(Clone)]
pub(crate) struct PeerFiles {
    link: PeerLink,
    /// What the owner calls that machine, for errors.
    name: String,
}

impl PeerFiles {
    pub(crate) fn new(name: impl Into<String>, link: PeerLink) -> Self {
        Self {
            link,
            name: name.into(),
        }
    }

    async fn ask(&self, request: RelayPeerRequest) -> Result<RelayPeerResponse, LinkError> {
        match (self.link)(request).await {
            Ok(RelayPeerResponse::FileRefused { reason }) => Err(LinkError::Refused(reason)),
            Ok(response) => Ok(response),
            Err(DaemonError::RelayTransport { code, message, .. })
                if DEFINITIVE_LINK_CODES.contains(&code.as_str()) =>
            {
                Err(LinkError::Refused(format!(
                    "{} refused the request: {message}",
                    self.name
                )))
            }
            Err(error) => Err(LinkError::Offline(error.to_string())),
        }
    }
}

fn unexpected() -> LinkError {
    LinkError::Refused("The other machine answered with something unexpected".to_owned())
}

/// The source of a transfer.
#[derive(Clone)]
pub(crate) enum FileSource {
    Local(Machine),
    Peer(PeerFiles),
}

impl FileSource {
    pub(crate) async fn stat(&self, path: &str) -> Result<KernelFileStat, LinkError> {
        match self {
            Self::Local(machine) => {
                let (machine, path) = (machine.clone(), path.to_owned());
                blocking(move || stat_path(machine.guard(), &path)).await
            }
            Self::Peer(peer) => match peer
                .ask(RelayPeerRequest::StatMachinePath {
                    path: path.to_owned(),
                })
                .await?
            {
                RelayPeerResponse::MachinePathStat { stat } => Ok(stat),
                _ => Err(unexpected()),
            },
        }
    }

    /// A page of the folder's entries from entry `offset`, and whether more follow.
    pub(crate) async fn list(
        &self,
        path: &str,
        offset: u64,
    ) -> Result<(Vec<KernelDirectoryEntry>, bool), LinkError> {
        match self {
            Self::Local(machine) => {
                let (machine, path) = (machine.clone(), path.to_owned());
                blocking(move || list_path(machine.guard(), &path, offset)).await
            }
            Self::Peer(peer) => match peer
                .ask(RelayPeerRequest::ListMachineDirectory {
                    path: path.to_owned(),
                    offset,
                })
                .await?
            {
                RelayPeerResponse::MachineDirectory { entries, truncated } => {
                    Ok((entries, truncated))
                }
                _ => Err(unexpected()),
            },
        }
    }

    /// Deletes a file whose revision is still `expect_revision`, or an empty folder.
    pub(crate) async fn delete(
        &self,
        path: &str,
        expect_revision: Option<&str>,
    ) -> Result<(), LinkError> {
        match self {
            Self::Local(machine) => {
                let (machine, path, expect) = (
                    machine.clone(),
                    path.to_owned(),
                    expect_revision.map(str::to_owned),
                );
                blocking(move || delete_path(machine.guard(), &path, expect.as_deref())).await
            }
            Self::Peer(peer) => match peer
                .ask(RelayPeerRequest::DeleteMachinePath {
                    path: path.to_owned(),
                    expect_revision: expect_revision.map(str::to_owned),
                })
                .await?
            {
                RelayPeerResponse::MachinePathDeleted => Ok(()),
                _ => Err(unexpected()),
            },
        }
    }
}

async fn blocking<T: Send + 'static>(
    work: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Result<T, LinkError> {
    crate::shutdown::spawn_blocking("Files source I/O", work)
        .await
        .map_err(|_| LinkError::Refused("The file request stopped unexpectedly".to_owned()))?
        .map_err(LinkError::Refused)
}

impl RemoteSource for FileSource {
    async fn read(&self, path: &str, offset: u64) -> Result<SourceSlice, LinkError> {
        match self {
            Self::Local(machine) => {
                let (machine, path) = (machine.clone(), path.to_owned());
                blocking(move || read_slice(&machine, &path, offset)).await
            }
            Self::Peer(peer) => match peer
                .ask(RelayPeerRequest::ReadMachineFile {
                    path: path.to_owned(),
                    offset,
                })
                .await?
            {
                RelayPeerResponse::FileSliceRead {
                    size_bytes,
                    sha256,
                    revision,
                    content,
                } => Ok(SourceSlice {
                    size_bytes,
                    sha256,
                    revision,
                    bytes: content
                        .decode()
                        .ok_or_else(|| LinkError::Refused("A slice arrived damaged".to_owned()))?,
                }),
                _ => Err(unexpected()),
            },
        }
    }

    fn peer(&self) -> String {
        match self {
            Self::Local(_) => "This machine".to_owned(),
            Self::Peer(peer) => peer.name.clone(),
        }
    }
}

/// The answer to a file request of another kernel of the owner, from the files of this machine.
pub(crate) fn serve(machine: &Machine, request: RelayPeerRequest) -> RelayPeerResponse {
    let refused = |reason| RelayPeerResponse::FileRefused { reason };
    match request {
        RelayPeerRequest::StatMachinePath { path } => match stat_path(machine.guard(), &path) {
            Ok(stat) => RelayPeerResponse::MachinePathStat { stat },
            Err(reason) => refused(reason),
        },
        RelayPeerRequest::ListMachineDirectory { path, offset } => {
            match list_path(machine.guard(), &path, offset) {
                Ok((entries, truncated)) => {
                    RelayPeerResponse::MachineDirectory { entries, truncated }
                }
                Err(reason) => refused(reason),
            }
        }
        RelayPeerRequest::ReadMachineFile { path, offset } => {
            match read_slice(machine, &path, offset) {
                Ok(slice) => RelayPeerResponse::FileSliceRead {
                    size_bytes: slice.size_bytes,
                    sha256: slice.sha256,
                    revision: slice.revision,
                    content: RemoteFileBytes::encode(&slice.bytes),
                },
                Err(reason) => refused(reason),
            }
        }
        RelayPeerRequest::DeleteMachinePath {
            path,
            expect_revision,
        } => match delete_path(machine.guard(), &path, expect_revision.as_deref()) {
            Ok(()) => RelayPeerResponse::MachinePathDeleted,
            Err(reason) => refused(reason),
        },
        _ => refused("not a file request".to_owned()),
    }
}

/// A DaemonError of the Kernel Files service as the text a refusal carries.
fn reason(error: DaemonError) -> String {
    match error {
        DaemonError::MachineRequest { message, .. } => message,
        other => other.to_string(),
    }
}

fn stat_path(guard: &StateGuard, path: &str) -> Result<KernelFileStat, String> {
    match kernel_files::stat(
        &StatKernelFileRequest {
            path: path.to_owned(),
            with_sha256: false,
        },
        guard,
    ) {
        Ok(LocalDaemonResponse::KernelFileStat { stat }) => Ok(stat),
        Ok(_) => Err("unexpected answer".to_owned()),
        Err(error) => Err(reason(error)),
    }
}

pub(crate) fn list_path(
    guard: &StateGuard,
    path: &str,
    offset: u64,
) -> Result<(Vec<KernelDirectoryEntry>, bool), String> {
    match kernel_files::list(
        &ListKernelDirectoryRequest {
            path: path.to_owned(),
            offset,
        },
        guard,
    ) {
        Ok(LocalDaemonResponse::KernelDirectoryListed {
            entries, truncated, ..
        }) => Ok((entries, truncated)),
        Ok(_) => Err("unexpected answer".to_owned()),
        Err(error) => Err(reason(error)),
    }
}

/// Removes `path` if it is an empty folder, or a file whose revision is `expect_revision`.
fn delete_path(
    guard: &StateGuard,
    path: &str,
    expect_revision: Option<&str>,
) -> Result<(), String> {
    let absolute = Machine::absolute(path)?;
    guard.check(&absolute)?;
    let metadata =
        std::fs::symlink_metadata(&absolute).map_err(|_| format!("`{path}` does not exist"))?;
    if metadata.is_dir() {
        return std::fs::remove_dir(&absolute)
            .map_err(|_| format!("`{path}` is a folder that is not empty; nothing was deleted"));
    }
    match expect_revision {
        Some(expected) if crate::agent_file_copy::revision(&metadata) == expected => {
            std::fs::remove_file(&absolute)
                .map_err(|error| format!("`{path}` could not be deleted: {error}"))
        }
        Some(_) => Err(format!(
            "`{path}` changed after it was copied; it was not deleted"
        )),
        None => Err(format!("`{path}` is a file; say which version to delete")),
    }
}
