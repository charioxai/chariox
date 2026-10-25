//! Bounded private artifact transport and pinned peer operations for Move.
use super::*;
use crate::agent_move::peer::{
    AgentMoveChunk, AgentMovePeerRequest as Request, AgentMovePeerResponse as Response,
};
use crate::agent_move::{
    error as move_error, AgentMoveArtifactKind, AgentMoveCapture, AgentMoveMaterialization,
    HomeMoveOperation, MOVE_CHUNK_BYTES,
};
impl KernelRuntimeState {
    pub(super) async fn materialize_agent_move(
        &self,
        operation: &HomeMoveOperation,
        capture: &AgentMoveCapture,
    ) -> Result<AgentMoveMaterialization, DaemonError> {
        if operation.target_ref.is_some() {
            let response = self
                .move_peer_target(
                    operation,
                    Request::BeginTarget {
                        capture: capture.clone(),
                    },
                )
                .await?;
            if response != Response::TargetPrepared {
                return Err(move_error("Target did not reserve a fresh import root"));
            }
        } else {
            self.begin_agent_move_target(capture.clone())?;
        }
        for kind in [
            AgentMoveArtifactKind::Workspace,
            AgentMoveArtifactKind::Native,
        ] {
            let length = match kind {
                AgentMoveArtifactKind::Workspace => capture.workspace.size_bytes,
                AgentMoveArtifactKind::Native => match &capture.native {
                    Some(native) => native.size_bytes,
                    None => continue,
                },
            };
            let mut offset = 0;
            while offset < length {
                let chunk = if operation.source_remote.is_some() {
                    let reply = self
                        .move_peer_source(
                            operation,
                            Request::ReadChunk {
                                identity: operation.identity.clone(),
                                kind,
                                offset,
                            },
                        )
                        .await?;
                    let Response::Chunk { chunk } = reply else {
                        return Err(move_error("Source returned an invalid artifact chunk"));
                    };
                    chunk
                } else {
                    AgentMoveChunk {
                        offset,
                        bytes: self.owned.agent_moves.read_chunk(capture, kind, offset)?,
                    }
                };
                if chunk.offset != offset
                    || chunk.bytes.is_empty()
                    || chunk.bytes.len() > MOVE_CHUNK_BYTES
                    || offset.saturating_add(chunk.bytes.len() as u64) > length
                {
                    return Err(move_error(
                        "Source artifact chunk does not match its declared bounds",
                    ));
                }
                let next = offset + chunk.bytes.len() as u64;
                if operation.target_ref.is_some() {
                    let reply = self
                        .move_peer_target(
                            operation,
                            Request::WriteChunk {
                                identity: operation.identity.clone(),
                                kind,
                                chunk,
                            },
                        )
                        .await?;
                    if reply != Response::ChunkWritten {
                        return Err(move_error(
                            "Target chunk acknowledgement is uncertain; retry the same move",
                        ));
                    }
                } else {
                    self.owned.agent_moves.write_chunk(
                        &operation.identity.operation_id,
                        kind,
                        offset,
                        &chunk.bytes,
                    )?;
                }
                offset = next;
            }
        }
        if operation.target_ref.is_some() {
            let response = self
                .move_peer_target(
                    operation,
                    Request::PublishTarget {
                        identity: operation.identity.clone(),
                    },
                )
                .await?;
            match response {
                Response::Published { receipt } => Ok(receipt),
                _ => Err(move_error(
                    "Target did not publish a verified Project receipt",
                )),
            }
        } else {
            let store = self.owned.agent_moves.clone();
            let config = self.owned.config_projection.snapshot();
            let operation = operation.identity.operation_id.clone();
            tokio::task::spawn_blocking(move || store.publish_target(&operation, &config))
                .await
                .map_err(|_| move_error("Move publication stopped; retry the durable operation"))?
        }
    }

    pub(super) async fn release_agent_move_source(
        &self,
        operation: &HomeMoveOperation,
        retired: bool,
    ) -> Result<(), DaemonError> {
        let request = if retired {
            Request::RetireSource {
                identity: operation.identity.clone(),
            }
        } else {
            Request::ReleaseSource {
                identity: operation.identity.clone(),
            }
        };
        if operation.source_remote.is_some() {
            let response = self.move_peer_source(operation, request).await?;
            if response
                != if retired {
                    Response::SourceRetired
                } else {
                    Response::SourceReleased
                }
            {
                return Err(move_error(
                    "Source did not acknowledge the requested Move boundary; retry this generation",
                ));
            }
        } else if let Some(mut source) = self
            .owned
            .agent_moves
            .source(&operation.identity.operation_id)
        {
            if retired {
                source.retired = true;
            } else {
                source.released = true;
            }
            self.owned.agent_moves.save_source(source)?;
        }
        Ok(())
    }

    pub(super) async fn move_peer_target(
        &self,
        operation: &HomeMoveOperation,
        request: Request,
    ) -> Result<Response, DaemonError> {
        if operation.target_ref.is_none() {
            return Err(move_error("Move target is local"));
        }
        let (target, relay) = self
            .with_app_side_effect(|app| {
                app.resolve_agent_move_kernel(
                    &operation.original_agent,
                    &operation.identity.target_kernel_id,
                )
            })
            .await?;
        if target.kernel_id != operation.identity.target_kernel_id
            || crate::runtime::terminal_pairings::public_key_thumbprint(&target.public_key)
                != operation.identity.target_key_thumbprint
        {
            return Err(move_error("Selected target kernel identity changed; reconnect the original target before retrying"));
        }
        let config = relay.unwrap_or_else(|| self.owned.config_projection.snapshot());
        self.send_agent_move_peer(&config, &target.kernel_id, request)
            .await
    }

    pub(super) async fn move_peer_source(
        &self,
        operation: &HomeMoveOperation,
        request: Request,
    ) -> Result<Response, DaemonError> {
        let source = operation
            .source_remote
            .as_ref()
            .ok_or_else(|| move_error("Move source is local"))?;
        let config = self
            .with_app_side_effect(|app| app.relay_config_for_remote_execution(source))
            .await;
        self.send_agent_move_peer(&config, &source.worker_kernel_id, request)
            .await
    }

    async fn send_agent_move_peer(
        &self,
        config: &crate::config::DaemonConfig,
        kernel: &str,
        request: Request,
    ) -> Result<Response, DaemonError> {
        let response = crate::transport::relay_client::send_peer_request_via_temporary_connection_with_timeout(config,
            ClientTarget { daemon_id: Some(kernel.into()), daemon_alias: None },
            RelayPeerRequest::AgentMove { request }, Duration::from_secs(1800)).await?;
        match response {
            RelayPeerResponse::AgentMove { response } => Ok(response),
            _ => Err(move_error(
                "Peer does not support durable Move; upgrade both kernels before retrying",
            )),
        }
    }
}
