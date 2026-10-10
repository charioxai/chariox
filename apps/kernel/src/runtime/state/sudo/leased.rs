//! MP-08/MP-09/MP-10/MP-11 A10: sudo windows for leased agents. The home
//! kernel stays the only authority; over the existing lease the worker holds a
//! narrowing fence for one exact leased home prompt, and every privileged call
//! is forwarded home and rechecked against the home window there.
use super::*;
use crate::transport::relay_peer::{LeasedSudoGrant, RelayPeerRequest, RelayPeerResponse};

/// Peer protocol that carries `UpdateLeasedSudo`; older workers are refused.
pub(crate) const LEASED_SUDO_PEER_PROTOCOL_VERSION: u32 = 83;
/// A worker refusal meaning it lost the window (restart): the home ends it.
const LEASED_SUDO_LOST: &str = "worker no longer holds this sudo window";

/// Worker memory only, so a worker restart drops every fence (fail closed).
#[derive(Debug, Clone)]
pub(crate) struct WorkerSudoGrant {
    leased_agent_id: String,
    home_prompt_id: String,
    entry_id: String,
    revision: u64,
    deadline: Instant,
    catalog_ready: bool,
}

impl KernelRuntimeState {
    /// Worker: install, renew or end the fence of one leased agent. A fresh
    /// fence reloads an idle provider that caches its tool catalog, so the
    /// elevated turn lists `chariox_kernel_request` like a local one.
    pub(crate) async fn update_relay_leased_sudo(
        &self,
        leased_agent_id: &str,
        home_prompt_id: &str,
        grant: LeasedSudoGrant,
    ) -> Result<(), DaemonError> {
        let _operation = self.leased_agent_operations.lock(leased_agent_id).await;
        let leased = leased_agent_id.to_string();
        let (session, agent) = self
            .with_app_side_effect(move |app| {
                let mut runtime = crate::app::RemoteLeaseRuntime::new(app);
                runtime.consume_leased_agent_authorization(&leased)?;
                runtime
                    .leased_agent_backing(&leased)
                    .ok_or(DaemonError::LeasedAgentNotFound {
                        leased_agent_id: leased.clone(),
                    })
            })
            .await?;
        let (entry, refresh) = {
            let mut grants = self
                .owned
                .leased_sudo_grants
                .lock()
                .expect("leased sudo grants poisoned");
            if grant.remaining_ms == 0 {
                if grants
                    .get(&agent)
                    .is_some_and(|g| g.entry_id == grant.entry_id)
                {
                    grants.remove(&agent);
                }
                return Ok(());
            } else {
                let current = grants.get(&agent);
                if current
                    .is_some_and(|g| g.entry_id == grant.entry_id && g.revision > grant.revision)
                {
                    return Err(error("stale sudo window revision for this leased agent"));
                }
                let fresh = current.is_none_or(|g| g.entry_id != grant.entry_id);
                if fresh && !grant.initial {
                    return Err(error(LEASED_SUDO_LOST));
                }
                let catalog_ready = current.is_some_and(|g| !fresh && g.catalog_ready);
                let entry = grant.entry_id.clone();
                grants.insert(
                    agent.clone(),
                    WorkerSudoGrant {
                        leased_agent_id: leased_agent_id.into(),
                        home_prompt_id: home_prompt_id.into(),
                        entry_id: grant.entry_id,
                        revision: grant.revision,
                        deadline: Instant::now() + Duration::from_millis(grant.remaining_ms),
                        catalog_ready,
                    },
                );
                (entry, !catalog_ready)
            }
        };
        let warm = self
            .owned
            .provider_store
            .get_run_for_agent(&session, &agent)
            .is_some_and(|run| {
                crate::provider::provider_runtime_catalog_requires_reload(run.provider())
            });
        if refresh && warm {
            use super::super::provider_reload::{ProviderReloadOutcome, ProviderReloadReason};
            // MP-08/MP-10/MP-11 R947-2: Deferred is not a catalog receipt.
            // An interrupted/failed refresh stays unready, including on the
            // next update of the same entry. Only this confirmed attempt arms it.
            tokio::time::timeout(Duration::from_secs(30), async {
                loop {
                    if !self.worker_sudo_grant_open(&agent) {
                        return Err(error("worker sudo window ended during catalog refresh"));
                    }
                    let outcome = self
                        .reload_agent_provider_if_idle_for_reason(
                            &session,
                            &agent,
                            &ProviderReloadReason::RuntimeToolCatalog,
                        )
                        .await?;
                    if matches!(outcome, ProviderReloadOutcome::Deferred) {
                        tokio::time::sleep(Duration::from_millis(50)).await;
                        continue;
                    }
                    if matches!(outcome, ProviderReloadOutcome::Reloaded) {
                        while !self
                            .owned
                            .provider_store
                            .get_run_for_agent(&session, &agent)
                            .is_some_and(|run| {
                                run.state() == crate::provider::ProviderRunState::Running
                            })
                        {
                            tokio::time::sleep(Duration::from_millis(50)).await;
                        }
                    }
                    return Ok(());
                }
            })
            .await
            .map_err(|_| {
                error("worker provider catalog did not become ready for the sudo window")
            })??;
        }
        let mut grants = self
            .owned
            .leased_sudo_grants
            .lock()
            .expect("leased sudo grants poisoned");
        let grant = grants
            .get_mut(&agent)
            .filter(|grant| grant.entry_id == entry)
            .ok_or_else(|| error("worker sudo window ended during catalog refresh"))?;
        if Instant::now() >= grant.deadline {
            return Err(error("worker sudo window ended during catalog refresh"));
        }
        grant.catalog_ready = true;
        Ok(())
    }

    /// Worker listing: a live fence lists the tool for its leased agent only.
    pub(super) fn worker_sudo_grant_open(&self, backing_agent: &str) -> bool {
        self.owned
            .leased_sudo_grants
            .lock()
            .expect("leased sudo grants poisoned")
            .get(backing_agent)
            .is_some_and(|grant| Instant::now() < grant.deadline)
    }

    /// Worker: a leased run's privileged call passes its own fence, then goes
    /// home. `None` means `run` is not leased and stays on the local path.
    pub(crate) async fn try_forward_leased_kernel_request(
        &self,
        run: &crate::provider::RuntimeProviderRun,
        arguments: serde_json::Value,
    ) -> Result<Option<crate::transport::runtime_tools::RuntimeToolResult>, DaemonError> {
        let Some(context) = self.leased_forward_context(run).await? else {
            return Ok(None);
        };
        let live = run.agent_instance_id().is_some_and(|agent| {
            self.owned
                .leased_sudo_grants
                .lock()
                .expect("leased sudo grants poisoned")
                .get(agent)
                .is_some_and(|grant| {
                    grant.catalog_ready
                        && Instant::now() < grant.deadline
                        && grant.leased_agent_id == context.leased_agent_id
                        && context.home_prompt_id.as_deref() == Some(&grant.home_prompt_id)
                })
        });
        if !live {
            return Err(error(
                "this leased turn has no live sudo window on this worker",
            ));
        }
        self.forward_meta_runtime_tool(context, "chariox_kernel_request", arguments)
            .await
            .map(Some)
    }

    /// Home: the leased agent whose current worker run is `run_id`.
    pub(super) fn leased_agent_for_projected_run(
        &self,
        run_id: &str,
    ) -> Option<crate::agent::AgentInstance> {
        run_id.strip_prefix("leased:")?;
        self.owned
            .agent_store
            .list_agents()
            .into_iter()
            .find(|agent| {
                agent.remote_execution().is_some_and(|remote| {
                    remote
                        .active_worker_provider_run_id
                        .as_deref()
                        .is_some_and(|worker| {
                            crate::provider::projected_leased_provider_run_id(
                                &remote.leased_agent_id,
                                worker,
                            ) == run_id
                        })
                })
            })
    }

    /// Home: the live window for a forwarded privileged call. The forwarded
    /// context already names the current lease, worker run and running home
    /// prompt; the window must be bound to exactly that prompt.
    pub(crate) fn sudo_for_leased_context(
        &self,
        context: &crate::transport::relay_peer::RemoteWorkspaceLiveSyncContext,
    ) -> Result<KernelSudoTurn, DaemonError> {
        self.authorize_forwarded_workspace_context(context)?;
        let run = crate::provider::projected_leased_provider_run_id(
            &context.leased_agent_id,
            &context.worker_provider_run_id,
        );
        let turn = self.sudo_for_run(&run)?;
        if context.home_prompt_id.is_none() || turn.prompt_id != context.home_prompt_id {
            return Err(error("this provider turn has no sudo authority"));
        }
        Ok(turn)
    }

    /// Home: the fence for a leased dispatch about to be submitted, if the
    /// prompt is the running turn bound to a live window of its agent.
    pub(in crate::runtime::state) fn leased_sudo_fence(
        &self,
        dispatch: &crate::app::KernelRemotePromptDispatch,
    ) -> Option<LeasedSudoGrant> {
        let session = self
            .owned
            .session_store
            .get_session(&dispatch.session_id)
            .ok()?;
        let (entry, prompt) = self
            .owned
            .prompt_state_owner
            .sudo_bound_prompt(&session, &dispatch.agent_id)?;
        if prompt != dispatch.prompt_id {
            return None;
        }
        let turn = self
            .owned
            .sudo_turns
            .lock()
            .expect("access state poisoned")
            .get(&entry)
            .cloned()?;
        let remaining = turn.deadline?.checked_duration_since(Instant::now())?;
        let initial = !self
            .owned
            .leased_sudo_fenced
            .lock()
            .expect("leased sudo fences poisoned")
            .contains(&turn.entry_id);
        (turn.agent_id == dispatch.agent_id && self.sudo_live(&turn)).then(|| LeasedSudoGrant {
            entry_id: turn.entry_id,
            revision: turn.revision,
            remaining_ms: u64::try_from(remaining.as_millis())
                .unwrap_or(u64::MAX)
                .max(1),
            initial,
        })
    }

    /// Home: before an elevated leased turn is submitted, its worker fence
    /// must be confirmed. A worker that lost the window ends it here; the turn
    /// then runs as regular work, never with authority the worker cannot see.
    pub(in crate::runtime::state) async fn fence_leased_turn(
        &self,
        dispatch: &crate::app::KernelRemotePromptDispatch,
    ) -> Result<(), DaemonError> {
        let Some(grant) = self.leased_sudo_fence(dispatch) else {
            return Ok(());
        };
        let entry = grant.entry_id.clone();
        match self
            .push_leased_sudo(&dispatch.agent_id, &dispatch.prompt_id, grant)
            .await
        {
            Ok(()) => {
                self.owned
                    .leased_sudo_fenced
                    .lock()
                    .expect("leased sudo fences poisoned")
                    .insert(entry);
                Ok(())
            }
            Err(error) if error.to_string().contains(LEASED_SUDO_LOST) => {
                let ended = self
                    .owned
                    .sudo_turns
                    .lock()
                    .expect("access state poisoned")
                    .remove(&entry);
                if let Some(turn) = ended {
                    self.finish_sudo_window(&turn, "worker_restarted")?;
                }
                Ok(())
            }
            Err(error) => Err(error),
        }
    }

    /// Home: send one fence to the agent's worker over the existing lease.
    pub(in crate::runtime::state) async fn push_leased_sudo(
        &self,
        agent_id: &str,
        home_prompt_id: &str,
        grant: LeasedSudoGrant,
    ) -> Result<(), DaemonError> {
        let Some(remote) = self
            .owned
            .agent_store
            .get_agent(agent_id)?
            .remote_execution()
            .cloned()
        else {
            return Ok(());
        };
        let mut config = self.config_snapshot().await;
        if let (Some(url), Some(token)) = (remote.relay_url.clone(), remote.relay_token.clone()) {
            config.apply_remote_relay_override(url, token);
        }
        match crate::transport::relay_client::send_peer_request_via_temporary_connection(
            &config,
            chariox_relay::protocol::ClientTarget {
                daemon_id: Some(remote.worker_kernel_id.clone()),
                daemon_alias: None,
            },
            RelayPeerRequest::UpdateLeasedSudo {
                leased_agent_id: remote.leased_agent_id,
                home_prompt_id: home_prompt_id.into(),
                grant,
            },
        )
        .await?
        {
            RelayPeerResponse::LeasedSudoUpdated => Ok(()),
            other => Err(error(format!("unexpected leased sudo response: {other:?}"))),
        }
    }

    /// Home: end or renew a leased window's worker fence without blocking the
    /// caller. A lost update is fail closed: the worker fence never outlives
    /// the time the home last granted, and the home rechecks every call.
    pub(super) fn spawn_leased_sudo_update(&self, turn: &KernelSudoTurn, ended: bool) {
        let fenced = {
            let mut fenced = self
                .owned
                .leased_sudo_fenced
                .lock()
                .expect("leased sudo fences poisoned");
            if ended {
                // An end is always sent: a fence whose confirmation was lost
                // must not wait for its own deadline.
                fenced.remove(&turn.entry_id);
                true
            } else {
                fenced.contains(&turn.entry_id)
            }
        };
        // A window not yet fenced on its worker gets its first fence with
        // its first leased turn.
        if turn.placement.is_none() || !fenced {
            return;
        }
        // Name the turn currently bound to the window (a later continuation
        // of the same work), not only the window's first prompt.
        let bound = self
            .owned
            .session_store
            .get_session(&turn.session_id)
            .ok()
            .and_then(|session| {
                self.owned
                    .prompt_state_owner
                    .sudo_bound_prompt(&session, &turn.agent_id)
            })
            .filter(|(entry, _)| *entry == turn.entry_id)
            .map(|(_, prompt)| prompt);
        let Some(prompt) = bound.or_else(|| turn.prompt_id.clone()) else {
            return;
        };
        let remaining_ms = if ended {
            0
        } else {
            turn.deadline
                .and_then(|deadline| deadline.checked_duration_since(Instant::now()))
                .map(|left| u64::try_from(left.as_millis()).unwrap_or(u64::MAX).max(1))
                .unwrap_or(0)
        };
        let grant = LeasedSudoGrant {
            entry_id: turn.entry_id.clone(),
            revision: turn.revision,
            remaining_ms,
            initial: false,
        };
        let state = self.clone();
        let agent = turn.agent_id.clone();
        tokio::spawn(async move {
            if let Err(error) = state.push_leased_sudo(&agent, &prompt, grant).await {
                crate::logging::warn_with_fields(
                "daemon.agent_lifecycle",
                "MP-08/MP-09/MP-10/MP-11 A10: worker sudo fence update not confirmed; it expires on its own deadline",
                serde_json::json!({"error": crate::secret_redaction::redact_secrets(&error.to_string())}),
            );
            }
        });
    }
}
