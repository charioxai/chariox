//! MP-08 / MP-10 / MP-11: harness-native skills of a leased worker, asked by the home kernel.
use super::skills_dispatch::ReportedWorkerRun;
use super::*;
use crate::app::RemoteLeaseRuntime;
use crate::skill::v2::{NativeSkillDetails, NativeSkillEntry, SkillSelection};
use crate::transport::relay_peer::{
    LeasedSkillNativeInventory, RelayPeerRequest, RelayPeerResponse, RemoteSkillSyncContext,
};

fn failure(message: &str) -> DaemonError {
    DaemonError::LocalTransport {
        operation: "skills.v2.remote_inventory",
        message: message.into(),
    }
}

/// One agent's native entries on its execution kernel.
pub(super) struct AgentNativeEntries {
    pub entries: Vec<NativeSkillEntry>,
    pub revision: String,
}

impl KernelRuntimeState {
    /// Native entries behind a turn: the local provider run, or the leased worker.
    pub(super) fn turn_native_entries(
        &self,
        authority: &crate::skill::turn_authority::SkillTurnAuthority,
        agent_id: &str,
    ) -> Result<Vec<NativeSkillEntry>, DaemonError> {
        if authority.is_leased() {
            return Ok(self
                .agent_native_entries(agent_id, authority.catalog_revision, false)?
                .map(|native| native.entries)
                .unwrap_or_default());
        }
        let run = self
            .owned
            .provider_store
            .get_run(&authority.provider_run_id)?;
        Ok(self
            .local_skill_inventory_for_run(&run, authority.catalog_revision, false)?
            .entries)
    }

    /// Context left in the provider; unknown for a leased turn (the worker owns usage).
    pub(super) fn turn_remaining_context(
        &self,
        authority: &crate::skill::turn_authority::SkillTurnAuthority,
    ) -> Option<u64> {
        if authority.is_leased() {
            return None;
        }
        let usage = self
            .owned
            .provider_store
            .get_run(&authority.provider_run_id)
            .ok()?
            .usage();
        usage
            .context_window
            .zip(usage.context_tokens)
            .map(|(capacity, used)| capacity.saturating_sub(used))
    }

    /// `None` until the agent's provider has started on its execution kernel.
    /// Blocking: call from a blocking Skills request or a blocking context only.
    pub(super) fn agent_native_entries(
        &self,
        agent_id: &str,
        catalog_revision: u64,
        refresh: bool,
    ) -> Result<Option<AgentNativeEntries>, DaemonError> {
        let agent = self.owned.agent_store.get_agent(agent_id)?;
        if agent.remote_execution().is_some() {
            let (inventory, _) = tokio::runtime::Handle::current().block_on(
                self.recovering_remote_skill_native_inventory(
                    &agent,
                    catalog_revision,
                    refresh,
                    None,
                ),
            )?;
            return Ok(inventory.map(|inventory| AgentNativeEntries {
                entries: inventory.entries,
                revision: inventory.revision,
            }));
        }
        let session = self.owned.session_store.get_session(agent.session_id())?;
        if self
            .owned
            .provider_store
            .get_run_for_agent(session.id(), agent.id())
            .is_none()
        {
            return Ok(None);
        }
        let inventory = self.local_skill_inventory(agent_id, catalog_revision, refresh)?;
        Ok(Some(AgentNativeEntries {
            entries: inventory.entries,
            revision: inventory.revision,
        }))
    }

    /// Dispatch-time dedupe input: what the worker's harness already exposes.
    pub(super) async fn remote_dispatch_native_entries(
        &self,
        agent: &crate::agent::AgentInstance,
    ) -> Result<(Vec<NativeSkillEntry>, ReportedWorkerRun), DaemonError> {
        if !super::skills_catalog_runtime::catalog_available(&self.owned.durable_state_store) {
            return Ok((vec![], ReportedWorkerRun::Unknown));
        }
        let revision = self.skills_catalog_service()?.storage.snapshot()?.revision;
        let (inventory, _) = self
            .remote_skill_native_inventory(agent, revision, false, None)
            .await?;
        if let (Some(inventory), Some(remote)) = (&inventory, agent.remote_execution()) {
            super::skills_dispatch::record_worker_compactions(
                self,
                agent.session_id(),
                agent.id(),
                &remote.leased_agent_id,
                &inventory.compaction_ids,
            )?;
        }
        Ok(match inventory {
            None => (vec![], ReportedWorkerRun::NoRun),
            Some(inventory) => (
                inventory.entries,
                inventory
                    .provider_run_id
                    .map_or(ReportedWorkerRun::Unknown, ReportedWorkerRun::Live),
            ),
        })
    }

    /// Picker and inventory reads meet a lease the restarted worker no longer knows
    /// before any prompt does: rebind it as a prompt dispatch would, then ask again.
    /// Dispatch has its own recovery loop and uses the plain request.
    pub(super) async fn recovering_remote_skill_native_inventory(
        &self,
        agent: &crate::agent::AgentInstance,
        catalog_revision: u64,
        refresh: bool,
        details_for: Option<SkillSelection>,
    ) -> Result<
        (
            Option<LeasedSkillNativeInventory>,
            Option<NativeSkillDetails>,
        ),
        DaemonError,
    > {
        match self
            .remote_skill_native_inventory(agent, catalog_revision, refresh, details_for.clone())
            .await
        {
            Err(error)
                if super::remote_prompt_worker_submission_runtime::remote_prompt_error_should_refresh_binding(&error) =>
            {
                let agent = self.refresh_remote_agent_binding_authorized(agent.id()).await?;
                self.remote_skill_native_inventory(&agent, catalog_revision, refresh, details_for)
                    .await
            }
            result => result,
        }
    }

    /// Home side: the leased worker owns the harness, so it answers.
    pub(super) async fn remote_skill_native_inventory(
        &self,
        agent: &crate::agent::AgentInstance,
        catalog_revision: u64,
        refresh: bool,
        details_for: Option<SkillSelection>,
    ) -> Result<
        (
            Option<LeasedSkillNativeInventory>,
            Option<NativeSkillDetails>,
        ),
        DaemonError,
    > {
        let remote = agent
            .remote_execution()
            .ok_or_else(|| failure("worker binding missing"))?;
        if !remote.relay_peer_protocol_compatible() {
            return Err(failure(
                "the leased worker predates the Skills compaction report; rebind the remote agent",
            ));
        }
        let mut config = self.owned.config_projection.snapshot();
        if let (Some(url), Some(token)) = (&remote.relay_url, &remote.relay_token) {
            config.apply_remote_relay_override(url.clone(), token.clone());
        }
        let request = RelayPeerRequest::ReadLeasedSkillNativeInventory {
            context: RemoteSkillSyncContext {
                home_kernel_id: config.daemon_id.clone(),
                home_session_id: agent.session_id().into(),
                home_agent_id: agent.id().into(),
                leased_agent_id: remote.leased_agent_id.clone(),
            },
            catalog_revision,
            refresh,
            details_for,
        };
        let target = chariox_relay::protocol::ClientTarget {
            daemon_id: Some(remote.worker_kernel_id.clone()),
            daemon_alias: None,
        };
        let timeout = std::time::Duration::from_millis(config.relay_request_timeout_ms);
        let response = match self.connected_relay_state_for_config(&config).await {
            Some(state) => {
                crate::transport::relay_client::send_peer_request_via_connected_relay_with_timeout(
                    &config, &state, target, request, timeout,
                )
                .await
            }
            None => {
                crate::transport::relay_client::send_peer_request_via_temporary_connection_with_timeout(
                    &config, target, request, timeout,
                )
                .await
            }
        }?;
        match response {
            RelayPeerResponse::LeasedSkillNativeInventory { inventory, details } => {
                Ok((inventory, details))
            }
            _ => Err(failure("unexpected worker Skills inventory response")),
        }
    }

    /// A native composer on a leased worker takes `$name` before the home dispatches
    /// any turn, so the lease receives its skills when the provider launches. A
    /// failed transfer refuses the launch rather than leaving typed skills unresolved.
    pub(super) async fn ensure_native_launch_skills(
        &self,
        config: &crate::config::DaemonConfig,
        agent: &crate::agent::AgentInstance,
        remote: &crate::agent::RemoteAgentBinding,
    ) -> Result<(), DaemonError> {
        let Some(set) =
            super::skills_dispatch::prepare_remote_launch_skills(self, agent.session_id())?
        else {
            return Ok(());
        };
        let context = RemoteSkillSyncContext {
            home_kernel_id: config.daemon_id.clone(),
            home_session_id: agent.session_id().into(),
            home_agent_id: agent.id().into(),
            leased_agent_id: remote.leased_agent_id.clone(),
        };
        self.ensure_remote_skill_set(config, remote, &context, &set)
            .await
            .map(|_| ())
    }

    /// A native composer resolves typed `$name` from the view the home pushed last.
    /// After a published catalog change every lease gets the new view, so a skill set
    /// Off or installed since is honoured at once instead of at the next home turn.
    /// The home does not know which worker composers are open (a binding's active run
    /// id is cleared when a typed turn completes while the composer stays), so every
    /// lease is tried: a worker that cannot be reached is logged and skipped, and its
    /// next dispatched turn carries the view again. Returns how many were tried.
    pub(super) async fn refresh_leased_native_skill_views(&self) -> usize {
        let config = self.owned.config_projection.snapshot();
        let mut attempted = 0;
        for agent in self.owned.agent_store.list_agents() {
            let Some(remote) = agent.remote_execution().cloned() else {
                continue;
            };
            let mut relay_config = config.clone();
            if let (Some(url), Some(token)) = (remote.relay_url.clone(), remote.relay_token.clone())
            {
                relay_config.apply_remote_relay_override(url, token);
            }
            attempted += 1;
            if let Err(error) = self
                .ensure_native_launch_skills(&relay_config, &agent, &remote)
                .await
            {
                crate::logging::warn_with_fields(
                    "daemon.skills",
                    "leased native Skills view was not refreshed",
                    serde_json::json!({"agent_id": agent.id(), "error": error.to_string()}),
                );
            }
        }
        attempted
    }

    /// Home side: send what the lease's worker must hold and return its verified
    /// receipt. `config` already carries the lease's relay override.
    pub(super) async fn ensure_remote_skill_set(
        &self,
        config: &crate::config::DaemonConfig,
        remote: &crate::agent::RemoteAgentBinding,
        context: &RemoteSkillSyncContext,
        set: &super::skills_dispatch::RemoteSkillSet,
    ) -> Result<Vec<crate::transport::relay_peer::RemoteSkillMaterialization>, DaemonError> {
        let request = RelayPeerRequest::EnsureRemoteSkillPackages {
            execution_lease_id: remote.execution_lease_id.clone(),
            manifest: set.manifest.clone(),
            context: context.clone(),
            packages: set.packages.clone(),
            skill_names: set.names.clone(),
        };
        let target = chariox_relay::protocol::ClientTarget {
            daemon_id: Some(remote.worker_kernel_id.clone()),
            daemon_alias: None,
        };
        let timeout = std::time::Duration::from_millis(config.relay_request_timeout_ms);
        let response = match self.connected_relay_state_for_config(config).await {
            Some(state) => {
                crate::transport::relay_client::send_peer_request_via_connected_relay_with_timeout(
                    config, &state, target, request, timeout,
                )
                .await
            }
            None => {
                crate::transport::relay_client::send_peer_request_via_temporary_connection_with_timeout(
                    config, target, request, timeout,
                )
                .await
            }
        }?;
        crate::skill::transfer::verify_transfer_receipt(
            context,
            &remote.execution_lease_id,
            &set.manifest,
            response,
        )
    }

    /// Worker side: only the authenticated home of this lease may ask.
    pub(crate) async fn read_leased_skill_native_inventory(
        &self,
        context: RemoteSkillSyncContext,
        catalog_revision: u64,
        refresh: bool,
        details_for: Option<SkillSelection>,
    ) -> Result<
        (
            Option<LeasedSkillNativeInventory>,
            Option<NativeSkillDetails>,
        ),
        DaemonError,
    > {
        let target = self
            .with_app_side_effect(move |app| {
                let mut runtime = RemoteLeaseRuntime::new(app);
                runtime.consume_leased_agent_authorization(&context.leased_agent_id)?;
                runtime.leased_project_target(
                    &context.leased_agent_id,
                    &context.home_session_id,
                    &context.home_agent_id,
                    None,
                )
            })
            .await?;
        // A compaction the worker's harness made since the last turn is in its transcript only
        // until observed; read it now so the home hears of it before it plans this turn.
        crate::runtime::external_provider_session_control::refresh_attached_external_provider_histories_for_session(
            &self.app,
            Some(self),
            &target.backing_session_id,
        )
        .await;
        let state = self.clone();
        tokio::task::spawn_blocking(move || {
            let Some(run) = state
                .owned
                .provider_store
                .get_run_for_agent(&target.backing_session_id, &target.backing_agent_id)
            else {
                return Ok((None, None));
            };
            let inventory = state.local_skill_inventory_for_run(&run, catalog_revision, refresh)?;
            let compaction_ids = match super::context_handoff::native_session_id(&run) {
                Some(native_session) => state
                    .owned
                    .operational_history_store
                    .observed_agent_compaction_ids(
                        &target.backing_session_id,
                        &target.backing_agent_id,
                        run.adapter_key(),
                        run.account_profile(),
                        native_session,
                    )?,
                None => vec![],
            };
            let details = details_for
                .map(|selection| {
                    super::skills_native_inventory::native_details(
                        &inventory,
                        &selection,
                        state.owned.config_projection.snapshot().host_machine_id,
                    )
                })
                .transpose()?;
            Ok((
                Some(LeasedSkillNativeInventory {
                    entries: inventory.entries,
                    revision: inventory.revision,
                    provider_run_id: Some(run.id().to_string()),
                    compaction_ids,
                }),
                details,
            ))
        })
        .await
        .map_err(|_| failure("worker Skills inventory did not complete"))?
    }
}
