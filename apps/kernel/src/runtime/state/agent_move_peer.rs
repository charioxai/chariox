//! Peer-side preparation is fenced independently of the home connection.
use super::*;
use crate::agent_move::peer::{
    AgentMoveChunk, AgentMovePeerRequest as Request, AgentMovePeerResponse as Response,
};
use crate::agent_move::{error as move_error, AgentMoveCapture, AgentMoveIdentity};

impl KernelRuntimeState {
    pub(crate) async fn agent_move_peer_request(
        &self,
        request: Request,
        caller: Option<crate::app::LeaseCallerBinding>,
    ) -> Result<Response, DaemonError> {
        let identity = request.identity().clone();
        let _operation = self
            .owned
            .agent_moves
            .operation(format!("peer:{}", identity.operation_id))
            .await;
        let config = self.owned.config_projection.snapshot();
        let source_action = matches!(
            &request,
            Request::ProbeSource { .. }
                | Request::PrepareSource { .. }
                | Request::ReadChunk { .. }
                | Request::RetireSource { .. }
                | Request::ReleaseSource { .. }
        );
        let expected_kernel = if source_action {
            &identity.source_kernel_id
        } else {
            &identity.target_kernel_id
        };
        let expected_key = if source_action {
            &identity.source_key_thumbprint
        } else {
            &identity.target_key_thumbprint
        };
        if expected_kernel != &config.daemon_id
            || expected_key
                != &crate::runtime::terminal_pairings::public_key_thumbprint(
                    &config.relay_public_key,
                )
        {
            return Err(move_error(
                "Move peer identity changed; restore the selected kernel before retrying",
            ));
        }
        if crate::runtime::terminal_pairings::public_key_thumbprint(&identity.target_public_key)
            != identity.target_key_thumbprint
        {
            return Err(move_error("Move target public identity is invalid"));
        }
        let source_probe = matches!(&request, Request::ProbeSource { .. });
        match request {
            Request::Probe { native, .. } => {
                self.probe_agent_move_target(native, None).await?;
                Ok(Response::Compatible)
            }
            Request::CreateLease {
                home_agent_metaagent,
                ..
            } => {
                let lease = self
                    .with_app_side_effect(|app| {
                        app.create_agent_move_lease(&identity, home_agent_metaagent, caller)
                    })
                    .await?;
                Ok(Response::LeaseCreated {
                    lease,
                    relay_peer_protocol_version:
                        crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION,
                })
            }
            Request::AbortTarget { .. } => {
                self.abort_agent_move_target(&identity).await?;
                Ok(Response::TargetAborted)
            }
            Request::ProbeSource { context, .. } | Request::PrepareSource { context, .. } => {
                let source = self
                    .with_app_side_effect(|app| {
                        let mut runtime = crate::app::RemoteLeaseRuntime::new(app);
                        runtime.consume_leased_agent_authorization(&context.leased_agent_id)?;
                        runtime.leased_project_target(
                            &context.leased_agent_id,
                            &context.home_session_id,
                            &context.home_agent_id,
                            None,
                        )
                    })
                    .await?;
                if context.home_kernel_id != identity.home_kernel_id
                    || source.home_session_id != identity.home_session_id
                    || source.home_agent_id != identity.home_agent_id
                {
                    return Err(move_error("Move source does not match the home lease"));
                }
                if source_probe {
                    let native = self
                        .probe_agent_move_source(
                            &source.backing_session_id,
                            &source.backing_agent_id,
                        )
                        .await?;
                    return Ok(Response::SourceCompatible { native });
                }
                let capture = self
                    .prepare_agent_move_source(
                        identity,
                        &source.backing_session_id,
                        &source.backing_agent_id,
                    )
                    .await?;
                Ok(Response::SourcePrepared {
                    capture: Box::new(capture),
                })
            }
            Request::ReadChunk { kind, offset, .. } => {
                let source = self
                    .owned
                    .agent_moves
                    .source(&identity.operation_id)
                    .ok_or_else(|| move_error("Move source is not prepared"))?;
                if source.identity != identity {
                    return Err(move_error("Move source generation changed"));
                }
                let capture = source
                    .capture
                    .ok_or_else(|| move_error("Move source capture is not ready"))?;
                let bytes = self.owned.agent_moves.read_chunk(&capture, kind, offset)?;
                Ok(Response::Chunk {
                    chunk: AgentMoveChunk { offset, bytes },
                })
            }
            Request::BeginTarget { capture } => {
                self.begin_agent_move_target(*capture)?;
                Ok(Response::TargetPrepared)
            }
            Request::WriteChunk { kind, chunk, .. } => {
                self.ensure_agent_move_target_identity(&identity)?;
                self.owned.agent_moves.write_chunk(
                    &identity.operation_id,
                    kind,
                    chunk.offset,
                    &chunk.bytes,
                )?;
                Ok(Response::ChunkWritten)
            }
            Request::PublishTarget { .. } => {
                self.ensure_agent_move_target_identity(&identity)?;
                let store = self.owned.agent_moves.clone();
                let operation = identity.operation_id;
                let receipt =
                    tokio::task::spawn_blocking(move || store.publish_target(&operation, &config))
                        .await
                        .map_err(|_| {
                            move_error(
                                "Move import task stopped; query the durable operation and retry",
                            )
                        })??;
                Ok(Response::Published {
                    receipt: Box::new(receipt),
                })
            }
            Request::AttachTarget {
                context,
                requirements,
                ..
            } => {
                let bound = self
                    .with_app_side_effect(|app| {
                        let mut runtime = crate::app::RemoteLeaseRuntime::new(app);
                        runtime.consume_leased_agent_authorization(&context.leased_agent_id)?;
                        match runtime.leased_project_target(
                            &context.leased_agent_id,
                            &context.home_session_id,
                            &context.home_agent_id,
                            None,
                        ) {
                            Ok(bound) => Ok(Some(bound)),
                            Err(DaemonError::LeasedAgentNotFound { .. }) => Ok(None),
                            Err(error) => Err(error),
                        }
                    })
                    .await?;
                let Some(bound) = bound else {
                    return Ok(Response::AttachmentMissing);
                };
                if context.home_kernel_id != identity.home_kernel_id
                    || bound.home_session_id != identity.home_session_id
                    || bound.home_agent_id != identity.home_agent_id
                {
                    return Err(move_error("Move target does not match the home lease"));
                }
                self.prepare_agent_move_capabilities(
                    &identity,
                    &context.leased_agent_id,
                    &requirements,
                )
                .await?;
                self.attach_agent_move_target(
                    &identity,
                    &bound.backing_session_id,
                    &bound.backing_agent_id,
                    true,
                )
                .await?;
                self.with_app_side_effect(|app| {
                    app.retain_agent_move_backing(&identity.operation_id, &context.leased_agent_id)
                })
                .await?;
                let receipt = self
                    .finish_agent_move_readiness(&identity, &requirements)
                    .await?;
                Ok(Response::Attached {
                    receipt: Box::new(receipt),
                })
            }
            Request::ActivateTarget { readiness, .. } => {
                let receipt = self
                    .activate_agent_move_target(&identity, &readiness)
                    .await?;
                Ok(Response::Activated {
                    receipt: Box::new(receipt),
                })
            }
            Request::RetireSource { .. } | Request::ReleaseSource { .. } => {
                let Some(mut source) = self.owned.agent_moves.source(&identity.operation_id) else {
                    if matches!(request, Request::ReleaseSource { .. }) {
                        // A delayed Prepare must not recreate a fence after cancellation.
                        self.owned
                            .agent_moves
                            .save_source(crate::agent_move::MoveSource {
                                identity: identity.clone(),
                                session_id: identity.home_session_id.clone(),
                                agent_id: identity.home_agent_id.clone(),
                                provider_run_id: None,
                                stopped: false,
                                released: true,
                                capture: None,
                                retired: false,
                            })?;
                        return Ok(Response::SourceReleased);
                    }
                    return Err(move_error("Move source is not prepared"));
                };
                if source.identity != identity {
                    return Err(move_error("Move source generation changed"));
                }
                let retired = matches!(request, Request::RetireSource { .. });
                if matches!(
                    request,
                    Request::ReleaseSource {
                        allow_capture: false,
                        ..
                    }
                ) && source.capture.is_some()
                {
                    return Ok(Response::SourceCaptured);
                }
                if retired {
                    source.retired = true;
                } else {
                    if source.retired {
                        return Err(move_error("A committed Move source cannot be released; Return through a new generation"));
                    }
                    source.released = true;
                }
                self.owned.agent_moves.save_source(source)?;
                Ok(if retired {
                    Response::SourceRetired
                } else {
                    Response::SourceReleased
                })
            }
        }
    }

    pub(super) fn begin_agent_move_target(
        &self,
        capture: AgentMoveCapture,
    ) -> Result<(), DaemonError> {
        #[cfg(test)]
        let parent = self
            .owned
            .config_projection
            .snapshot()
            .private_runtime_state_root()
            .join("move-workspaces");
        #[cfg(not(test))]
        let parent = std::env::var_os("HOME")
            .map(PathBuf::from)
            .ok_or_else(|| {
                move_error("HOME is unavailable; configure the target user's home directory")
            })?
            .join("chariox-moves");
        self.owned.agent_moves.begin_target(capture, &parent)?;
        Ok(())
    }

    pub(super) fn ensure_agent_move_target_identity(
        &self,
        identity: &AgentMoveIdentity,
    ) -> Result<crate::agent_move::MoveTarget, DaemonError> {
        let target = self
            .owned
            .agent_moves
            .target(&identity.operation_id)
            .ok_or_else(|| move_error("Move target is not prepared"))?;
        if target.retired {
            return Err(move_error(
                "This Move target was cancelled; start a new generation",
            ));
        }
        if target.capture.identity != *identity {
            return Err(move_error("Move target generation changed"));
        }
        Ok(target)
    }
}
