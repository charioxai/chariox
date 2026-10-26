//! Home advances one durable move; peer preparation alone never releases queued work.
use super::*;
use crate::agent_move::peer::{AgentMovePeerRequest as Request, AgentMovePeerResponse as Response};
use crate::agent_move::{error as move_error, AgentMoveIdentity, HomeMoveOperation, HomeMovePhase};

impl KernelRuntimeState {
    pub(super) async fn recover_agent_moves_after_restart(&self) -> usize {
        for operation in self.owned.agent_moves.pending() {
            let agent = match self
                .owned
                .agent_store
                .get_agent(&operation.identity.home_agent_id)
            {
                Ok(agent) => agent,
                Err(_) => continue,
            };
            if let Err(error) = self
                .move_agent_transaction(
                    &operation.identity.home_session_id,
                    agent,
                    operation.target_ref,
                )
                .await
            {
                crate::logging::warn_with_fields(
                    "agent_move.recovery", "Durable move recovery remains fenced; retry the move after reconnecting its kernels",
                    serde_json::json!({"operation_id": operation.identity.operation_id, "error": error.to_string()}),
                );
            }
        }
        self.owned.agent_moves.pending().len()
    }

    pub(super) async fn move_agent_transaction(
        &self,
        session_id: &str,
        agent: crate::agent::AgentInstance,
        target_ref: Option<String>,
    ) -> Result<crate::agent::AgentInstance, DaemonError> {
        let _operation = self
            .owned
            .agent_moves
            .operation(format!("home:{session_id}:{}", agent.id()))
            .await;
        let agent = self.owned.agent_store.get_agent(agent.id())?;
        if let Some(previous) = self.owned.agent_moves.for_agent(session_id, agent.id()) {
            if matches!(previous.phase, HomeMovePhase::Complete { .. })
                && previous.target_ref == target_ref
            {
                let same_target = match (&target_ref, agent.remote_execution()) {
                    (None, None) => true,
                    (Some(_), Some(remote)) => {
                        remote.worker_kernel_id == previous.identity.target_kernel_id
                    }
                    _ => false,
                };
                if same_target {
                    return Ok(agent);
                }
            }
        }

        if agent.session_id() != session_id {
            return Err(move_error("The agent does not belong to this Room"));
        }
        let pending = self
            .owned
            .agent_moves
            .for_agent(session_id, agent.id())
            .filter(|operation| operation.phase.is_pending());
        let operation = if let Some(pending) = pending {
            if pending.target_ref != target_ref {
                return Err(move_error("This agent already has a pending move to another destination. Finish or roll back that operation first"));
            }
            pending
        } else {
            if target_ref.is_some() == agent.remote_execution().is_some() {
                return Err(move_error(if target_ref.is_some() {
                    "Return this agent home before moving it to another kernel"
                } else {
                    "This agent already executes at home"
                }));
            }
            let session = self.owned.session_store.get_session(session_id)?;
            let _claim = self
                .owned
                .prompt_state_owner
                .claim_idle_agent_move_transition(&session, agent.id())?;
            let config = self.owned.config_projection.snapshot();
            let (target_kernel_id, target_public_key) = if let Some(reference) = &target_ref {
                let (target, _) = self
                    .with_app_side_effect(|app| {
                        app.resolve_agent_move_destination(&agent, reference)
                    })
                    .await?;
                (target.kernel_id, target.public_key)
            } else {
                (config.daemon_id.clone(), config.relay_public_key.clone())
            };
            if target_ref.is_some() && target_kernel_id == config.daemon_id {
                return Err(move_error(
                    "This agent already executes on the selected home kernel",
                ));
            }
            let source_remote = agent.remote_execution().cloned();
            let (source_kernel_id, source_key) = if let Some(remote) = &source_remote {
                let (source, _) = self
                    .with_app_side_effect(|app| {
                        app.resolve_agent_move_kernel(&agent, &remote.worker_kernel_id)
                    })
                    .await?;
                let key = source.public_key;
                (remote.worker_kernel_id.clone(), key)
            } else {
                (config.daemon_id.clone(), config.relay_public_key.clone())
            };
            let generation = self
                .owned
                .agent_moves
                .for_agent(session_id, agent.id())
                .map_or(Ok(1), |old| {
                    old.identity
                        .generation
                        .checked_add(1)
                        .ok_or_else(|| move_error("Move generation exhausted"))
                })?;
            let identity = AgentMoveIdentity {
                operation_id: format!("move-{:032x}", rand::random::<u128>()),
                generation,
                home_kernel_id: config.daemon_id.clone(),
                home_session_id: session_id.into(),
                home_agent_id: agent.id().into(),
                owner_user_id: agent.owner_user_id().into(),
                source_kernel_id,
                target_kernel_id,
                home_key_thumbprint: crate::runtime::terminal_pairings::public_key_thumbprint(
                    &config.relay_public_key,
                ),
                source_key_thumbprint: crate::runtime::terminal_pairings::public_key_thumbprint(
                    &source_key,
                ),
                target_key_thumbprint: crate::runtime::terminal_pairings::public_key_thumbprint(
                    &target_public_key,
                ),
                target_public_key,
            };
            let operation = HomeMoveOperation {
                identity,
                revision: 0,
                original_agent: agent,
                target_ref,
                source_remote,
                phase: HomeMovePhase::Fenced,
            };
            let native = if let Some(remote) = &operation.source_remote {
                let context = crate::transport::relay_peer::RemoteSkillSyncContext {
                    home_kernel_id: operation.identity.home_kernel_id.clone(),
                    home_session_id: operation.identity.home_session_id.clone(),
                    home_agent_id: operation.identity.home_agent_id.clone(),
                    leased_agent_id: remote.leased_agent_id.clone(),
                };
                let response = self
                    .move_peer_source(
                        &operation,
                        Request::ProbeSource {
                            identity: operation.identity.clone(),
                            context,
                        },
                    )
                    .await?;
                let Response::SourceCompatible { native } = response else {
                    return Err(move_error("Source does not support durable Return; upgrade both kernels before retrying"));
                };
                native
            } else {
                self.probe_agent_move_source(session_id, operation.original_agent.id())
                    .await?
            };
            if operation.target_ref.is_some() {
                let reply = self
                    .move_peer_target(
                        &operation,
                        Request::Probe {
                            identity: operation.identity.clone(),
                            native,
                        },
                    )
                    .await?;
                if reply != Response::Compatible {
                    return Err(move_error("Target does not support durable Move; upgrade both kernels before retrying"));
                }
            } else {
                self.probe_agent_move_target(native, Some(&operation.original_agent))
                    .await?;
            }
            self.owned.agent_moves.save_home(operation)?
        };
        let operation_id = operation.identity.operation_id.clone();
        let result = self.resume_agent_move_operation(operation).await;
        if let Err(cause) = &result {
            if let Some(mut operation) = self.owned.agent_moves.home(&operation_id) {
                if matches!(
                    operation.phase,
                    HomeMovePhase::Fenced | HomeMovePhase::SourceStopped
                ) {
                    // The source decides under its operation lock whether capture occurred,
                    // including when its preparation reply was lost over the relay.
                    let current = self
                        .owned
                        .agent_store
                        .get_agent(&operation.identity.home_agent_id)?;
                    let bound = current.remote_execution().is_some_and(|binding| {
                        operation.target_ref.is_some()
                            && binding.worker_kernel_id == operation.identity.target_kernel_id
                    });
                    if !bound
                        && self
                            .release_agent_move_source(&operation, false, false)
                            .await?
                    {
                        operation.phase = HomeMovePhase::RolledBack {
                            reason: cause.to_string(),
                        };
                        self.owned.agent_moves.save_home(operation)?;
                    }
                }
            }
        }
        result
    }

    pub(super) async fn resume_agent_move_operation(
        &self,
        mut operation: HomeMoveOperation,
    ) -> Result<crate::agent::AgentInstance, DaemonError> {
        loop {
            self.authorize_current_external_command()?;
            operation = match operation.phase.clone() {
                HomeMovePhase::Fenced | HomeMovePhase::SourceStopped => {
                    let capture = if let Some(remote) = &operation.source_remote {
                        let context = crate::transport::relay_peer::RemoteSkillSyncContext {
                            home_kernel_id: operation.identity.home_kernel_id.clone(),
                            home_session_id: operation.identity.home_session_id.clone(),
                            home_agent_id: operation.identity.home_agent_id.clone(),
                            leased_agent_id: remote.leased_agent_id.clone(),
                        };
                        let response = self
                            .move_peer_source(
                                &operation,
                                Request::PrepareSource {
                                    identity: operation.identity.clone(),
                                    context,
                                },
                            )
                            .await?;
                        let Response::SourcePrepared { capture } = response else {
                            return Err(move_error(
                                "Source did not return a durable checkpoint and Project capture",
                            ));
                        };
                        *capture
                    } else {
                        self.prepare_agent_move_source(
                            operation.identity.clone(),
                            &operation.identity.home_session_id,
                            &operation.identity.home_agent_id,
                        )
                        .await?
                    };
                    if capture.identity != operation.identity {
                        return Err(move_error("Source returned a different move generation"));
                    }
                    // Local preparation can durably record SourceStopped before returning.
                    operation = self
                        .owned
                        .agent_moves
                        .home(&operation.identity.operation_id)
                        .ok_or_else(|| move_error("Move journal disappeared"))?;
                    operation.phase = HomeMovePhase::Captured { capture };
                    self.owned.agent_moves.save_home(operation)?
                }
                HomeMovePhase::Captured { capture } => {
                    let receipt = self.materialize_agent_move(&operation, &capture).await?;
                    if receipt.identity != operation.identity
                        || receipt.archive_sha256 != capture.workspace.sha256
                        || receipt.target_project_id != capture.archive_project_id
                    {
                        return Err(move_error("Target receipt does not match the captured Project and move generation"));
                    }
                    receipt.primary_path()?;
                    operation.phase = HomeMovePhase::Materialized { capture, receipt };
                    self.owned.agent_moves.save_home(operation)?
                }
                HomeMovePhase::Materialized { capture, receipt } => {
                    let remote = if operation.target_ref.is_some() {
                        let current = self
                            .owned
                            .agent_store
                            .get_agent(&operation.identity.home_agent_id)?;
                        let rebound = match current.remote_execution() {
                            Some(binding) if binding.worker_kernel_id == operation.identity.target_kernel_id => current,
                            Some(_) => return Err(move_error("Agent execution changed during Move; reconcile the durable operation before retrying")),
                            None => self.with_app_side_effect(|app| app.bind_materialized_agent_move(&current, &operation.identity, receipt.primary_path()?, &|| self.authorize_current_external_command())).await?,
                        };
                        let primary = receipt.primary_path()?.display().to_string();
                        if rebound.worktree_id() != Some(primary.as_str()) {
                            self.with_app_side_effect(|app| {
                                let repaired = app.agents().update_agent_config(rebound.id(), None, None, None, Some(Some(primary)))?;
                                app.durable_state_store().append_event("agent.updated", Some(repaired.id().into()), serde_json::json!({"agent": repaired, "source": "agent_move_workspace_recovery"}))?;
                                Ok::<_, DaemonError>(())
                            }).await?;
                        }
                        rebound.remote_execution().cloned()
                    } else {
                        None
                    };
                    operation.phase = HomeMovePhase::Bound {
                        capture,
                        receipt,
                        remote,
                    };
                    self.owned.agent_moves.save_home(operation)?
                }
                HomeMovePhase::Bound {
                    capture,
                    receipt,
                    remote,
                } => {
                    if let Some(remote) = &remote {
                        let context = crate::transport::relay_peer::RemoteSkillSyncContext {
                            home_kernel_id: operation.identity.home_kernel_id.clone(),
                            home_session_id: operation.identity.home_session_id.clone(),
                            home_agent_id: operation.identity.home_agent_id.clone(),
                            leased_agent_id: remote.leased_agent_id.clone(),
                        };
                        let response = self
                            .move_peer_target(
                                &operation,
                                Request::AttachTarget {
                                    identity: operation.identity.clone(),
                                    context,
                                },
                            )
                            .await?;
                        if response == Response::AttachmentMissing {
                            // A worker restart before Attach was acknowledged loses only
                            // its uncommitted empty lease, never a user provider turn.
                            self.with_app_side_effect(|app| {
                                app.destroy_remote_execution_binding(remote, &|| Ok(()))?;
                                let mut agent =
                                    app.agents().get_agent(&operation.identity.home_agent_id)?;
                                agent.set_remote_execution(None);
                                app.agents().restore_agent(agent.clone());
                                app.durable_state_store().append_event(
                                    "agent.updated",
                                    Some(agent.id().into()),
                                    serde_json::json!({"agent":agent,"source":"agent_move_rebind"}),
                                )?;
                                Ok::<_, DaemonError>(())
                            })
                            .await?;
                            operation.phase = HomeMovePhase::Materialized { capture, receipt };
                            operation = self.owned.agent_moves.save_home(operation)?;
                            continue;
                        }
                        if response != Response::Attached {
                            return Err(move_error(
                                "Target did not acknowledge native import and Project membership",
                            ));
                        }
                    } else {
                        self.attach_agent_move_target(
                            &operation.identity,
                            &operation.identity.home_session_id,
                            &operation.identity.home_agent_id,
                            false,
                        )
                        .await?;
                    }
                    // The home fence remains closed across the authority change and its
                    // agent event. Restart repairs publication from this durable phase.
                    operation.phase = HomeMovePhase::Committed { receipt };
                    let _ = capture;
                    self.owned.agent_moves.save_home(operation)?
                }
                HomeMovePhase::Committed { receipt } => {
                    if operation.target_ref.is_some() {
                        let response = self
                            .move_peer_target(
                                &operation,
                                Request::ActivateTarget {
                                    identity: operation.identity.clone(),
                                },
                            )
                            .await?;
                        if response != Response::Activated {
                            return Err(move_error(
                                "Target activation is uncertain; retry this move generation",
                            ));
                        }
                    } else {
                        self.with_app_side_effect(|app| {
                            let mut agent =
                                app.agents().get_agent(&operation.identity.home_agent_id)?;
                            agent.set_remote_execution(None);
                            app.agents().restore_agent(agent.clone());
                            app.durable_state_store().append_event(
                                "agent.updated",
                                Some(agent.id().to_string()),
                                serde_json::json!({"agent":agent,"source":"agent_move_committed"}),
                            )?;
                            Ok::<_, DaemonError>(())
                        })
                        .await?;
                        self.owned
                            .session_snapshot(&operation.identity.home_session_id)?;
                        let mut target = self
                            .owned
                            .agent_moves
                            .target(&operation.identity.operation_id)
                            .ok_or_else(|| move_error("Committed move target is missing"))?;
                        if !target.ready {
                            return Err(move_error(
                                "Committed move target has no native readiness receipt",
                            ));
                        }
                        target.active = true;
                        self.owned.agent_moves.save_target(target)?;
                    }
                    self.release_agent_move_source(&operation, true, false)
                        .await?;
                    if let Some(source) = &operation.source_remote {
                        self.with_app_side_effect(|app| {
                            app.destroy_remote_execution_binding(source, &|| Ok(()))
                        })
                        .await?;
                    }
                    let agent_id = operation.identity.home_agent_id.clone();
                    operation.phase = HomeMovePhase::Complete { receipt };
                    self.owned.agent_moves.save_home(operation)?;
                    return self.owned.agent_store.get_agent(&agent_id);
                }
                HomeMovePhase::Cancelling => {
                    self.finish_agent_move_abort(operation).await?;
                    return Err(move_error("Move cancelled; source execution restored"));
                }
                HomeMovePhase::Complete { .. } => {
                    return self
                        .owned
                        .agent_store
                        .get_agent(&operation.identity.home_agent_id)
                }
                HomeMovePhase::RolledBack { reason } => return Err(move_error(reason)),
            };
        }
    }
}
