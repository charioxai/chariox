//! The worker's side of `chariox.file.copy`: the home that holds the lease reads and writes files
//! in its leased agent's workspace, one slice per request.
use super::*;
use crate::agent_file_copy::{discard, read_slice, write_slice, Workspace, WriteSlice};
use crate::app::RemoteLeaseRuntime;
use crate::transport::relay_peer::{RelayPeerRequest, RelayPeerResponse, RemoteFileBytes};

impl KernelRuntimeState {
    pub(crate) async fn serve_leased_agent_file(
        &self,
        request: RelayPeerRequest,
    ) -> Result<RelayPeerResponse, DaemonError> {
        let leased_agent_id = match &request {
            RelayPeerRequest::ReadLeasedAgentFile {
                leased_agent_id, ..
            }
            | RelayPeerRequest::WriteLeasedAgentFile {
                leased_agent_id, ..
            }
            | RelayPeerRequest::DiscardLeasedAgentFile {
                leased_agent_id, ..
            } => leased_agent_id.clone(),
            _ => {
                return Err(DaemonError::LocalTransport {
                    operation: "serve leased agent file",
                    message: "not a leased agent file request".into(),
                })
            }
        };
        let workspace = self
            .with_app_side_effect(move |app| {
                let mut runtime = RemoteLeaseRuntime::new(app);
                runtime.consume_leased_agent_authorization(&leased_agent_id)?;
                runtime.leased_agent_workspace(&leased_agent_id)
            })
            .await?;
        tokio::task::spawn_blocking(move || serve(&workspace, request))
            .await
            .map_err(|_| DaemonError::LocalTransport {
                operation: "serve leased agent file",
                message: "the file request stopped unexpectedly".into(),
            })
    }
}

pub(super) fn serve(workspace: &Workspace, request: RelayPeerRequest) -> RelayPeerResponse {
    let refused = |reason| RelayPeerResponse::FileRefused { reason };
    match request {
        RelayPeerRequest::ReadLeasedAgentFile { path, offset, .. } => {
            match read_slice(workspace, &path, offset) {
                Ok(slice) => RelayPeerResponse::FileSliceRead {
                    size_bytes: slice.size_bytes,
                    sha256: slice.sha256,
                    revision: slice.revision,
                    content: RemoteFileBytes::encode(&slice.bytes),
                },
                Err(reason) => refused(reason),
            }
        }
        RelayPeerRequest::WriteLeasedAgentFile {
            path,
            offset,
            total_size,
            sha256,
            overwrite,
            content,
            ..
        } => {
            let Some(bytes) = content.decode() else {
                return refused("The slice arrived damaged".into());
            };
            match write_slice(
                workspace,
                &path,
                &WriteSlice {
                    offset,
                    total_size,
                    sha256: &sha256,
                    overwrite,
                    bytes: &bytes,
                },
            ) {
                Ok(staged) => RelayPeerResponse::LeasedAgentFileWritten {
                    received: staged.received,
                    sha256: staged.sha256,
                },
                Err(reason) => refused(reason),
            }
        }
        RelayPeerRequest::DiscardLeasedAgentFile { path, .. } => {
            discard(workspace, &path);
            RelayPeerResponse::LeasedAgentFileDiscarded
        }
        _ => refused("not a leased agent file request".into()),
    }
}
