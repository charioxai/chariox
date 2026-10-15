//! MP-08 / MP-11: explicit HOME -> leased worker desktop grants (500/109).
//! No Room authority, Browser capability, command strings or credential transfer.
use super::kernel_browser_runtime::host_error;
use super::*;
use crate::local::KernelDesktopTarget;
use crate::transport::relay_peer::LeasedComputerGrant;
use std::time::Duration;

impl KernelRuntimeState {
    pub(super) async fn revoke_user_leased_computer(
        &self,
        user: &str,
        agent_id: Option<&str>,
    ) -> Result<(), DaemonError> {
        let home = self.owned.config_projection.snapshot().daemon_id;
        let ids = self
            .owned
            .leased_computer_grants
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .filter(|(id, grant)| {
                grant.home_kernel_id == home && agent_id.is_none_or(|agent| agent == id.as_str())
            })
            .map(|(id, _)| id.clone())
            .collect::<Vec<_>>();
        for id in ids {
            let agent = self.owned.agent_store.get_agent(&id)?;
            if self.provider_account_authority_owner_user_id(agent.owner_user_id()) == user {
                self.revoke_leased_computer(user, &id).await?;
            }
        }
        Ok(())
    }

    pub(super) async fn grant_leased_computer(
        &self,
        user: &str,
        agent_id: &str,
        target: KernelDesktopTarget,
    ) -> Result<serde_json::Value, DaemonError> {
        let agent = self.owned.agent_store.get_agent(agent_id)?;
        let session = self.owned.session_snapshot(agent.session_id())?;
        if self.provider_account_authority_owner_user_id(agent.owner_user_id()) != user
            || !session.has_member(agent.owner_user_id())
            || session.status() == crate::session::SessionStatus::Ended
        {
            return Err(host_error(
                "MP-11: not_granted: desktop grant requires the agent owner".into(),
            ));
        }
        let remote = agent
            .remote_execution()
            .ok_or_else(|| host_error("MP-08: desktop grant requires a leased agent".into()))?;
        if remote.relay_peer_protocol_version.is_none_or(|version| {
            version < crate::transport::relay_peer::MINIMUM_LEASED_COMPUTER_PEER_VERSION
        }) {
            return Err(host_error(
                "MP-08: leased Computer requires desktop peer protocol 109".into(),
            ));
        }
        for id in [&target.surface_id, &target.generation] {
            if id.is_empty() || id.len() > 128 {
                return Err(host_error("MP-11: invalid desktop target".into()));
            }
        }
        // Retire an earlier grant first, including its queued/in-flight admissions.
        self.revoke_leased_computer(user, agent_id).await?;
        let config = self.owned.config_projection.snapshot();
        let grant = LeasedComputerGrant {
            epoch: format!("computer-{:032x}", rand::random::<u128>()),
            home_kernel_id: config.daemon_id.clone(),
            home_session_id: session.id().into(),
            home_agent_id: agent_id.into(),
            leased_agent_id: remote.leased_agent_id.clone(),
            desktop_kernel_id: remote.worker_kernel_id.clone(),
            target,
        };
        {
            let _change = self
                .owned
                .leased_computer_changes
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            let mut grants = self
                .owned
                .leased_computer_grants
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            grants.insert(agent_id.into(), grant.clone());
            drop(grants);
            self.owned
                .kernel_browser_host
                .grant_desktop(user, agent_id, &grant.target.surface_id);
        }
        let response = self
            .send_worker_home_runtime_request(
                &config,
                &grant.desktop_kernel_id,
                RelayPeerRequest::SetLeasedComputerGrant {
                    leased_agent_id: grant.leased_agent_id.clone(),
                    epoch: grant.epoch.clone(),
                    grant: Some(grant.clone()),
                },
                Duration::from_secs(30),
            )
            .await;
        if !matches!(
            response,
            Ok(RelayPeerResponse::LeasedComputerGrantUpdated {})
        ) {
            self.retire_leased_computer_grant(user, agent_id, Some(&grant.epoch));
            return Err(response
                .err()
                .unwrap_or_else(|| host_error("MP-11: desktop rejected the typed grant".into())));
        }
        // An owner revocation or binding change during the round trip wins.
        if self
            .owned
            .leased_computer_grants
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(agent_id)
            != Some(&grant)
        {
            return Err(host_error(
                "MP-11: not_granted: desktop grant superseded".into(),
            ));
        }
        if self
            .owned
            .agent_store
            .get_agent(agent_id)?
            .remote_execution()
            != Some(remote)
            || !self.owned.kernel_browser_host.is_loaded_for(
                user,
                agent_id,
                crate::runtime::kernel_browser_host::KernelBrowserCapability::Computer,
            )
        {
            self.revoke_leased_computer_epoch(user, agent_id, Some(&grant.epoch))
                .await?;
            return Err(host_error(
                "MP-11: not_granted: desktop grant superseded".into(),
            ));
        }
        self.refresh_user_domain_grants();
        self.room_computer_grant_snapshot(user, &config.daemon_id)
    }

    pub(super) async fn revoke_leased_computer(
        &self,
        user: &str,
        agent_id: &str,
    ) -> Result<(), DaemonError> {
        self.revoke_leased_computer_epoch(user, agent_id, None)
            .await
    }

    // Never hold the grant-map lock while retiring actors: admission callbacks
    // read that map. Serialize local changes separately, without any await.
    pub(super) fn retire_leased_computer_grant(
        &self,
        user: &str,
        agent_id: &str,
        expected_epoch: Option<&str>,
    ) -> Option<LeasedComputerGrant> {
        let _change = self
            .owned
            .leased_computer_changes
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let grant = {
            let mut grants = self
                .owned
                .leased_computer_grants
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            if expected_epoch
                .is_some_and(|epoch| grants.get(agent_id).is_none_or(|g| g.epoch != epoch))
            {
                return None;
            }
            grants.remove(agent_id)
        };
        if grant.is_some() {
            self.owned
                .kernel_browser_host
                .revoke_grants(user, Some(agent_id));
        }
        grant
    }

    async fn revoke_leased_computer_epoch(
        &self,
        user: &str,
        agent_id: &str,
        expected_epoch: Option<&str>,
    ) -> Result<(), DaemonError> {
        let agent = self.owned.agent_store.get_agent(agent_id)?;
        if self.provider_account_authority_owner_user_id(agent.owner_user_id()) != user {
            return Err(host_error(
                "MP-11: not_granted: only the owner may revoke desktop access".into(),
            ));
        }
        let grant = self.retire_leased_computer_grant(user, agent_id, expected_epoch);
        if let Some(grant) = grant {
            // HOME revocation is committed before transport; failed delivery cannot
            // authorize a subsequent call because the worker checks HOME each time.
            let config = self.owned.config_projection.snapshot();
            let response = self
                .send_worker_home_runtime_request(
                    &config,
                    &grant.desktop_kernel_id,
                    RelayPeerRequest::SetLeasedComputerGrant {
                        leased_agent_id: grant.leased_agent_id,
                        epoch: grant.epoch,
                        grant: None,
                    },
                    Duration::from_secs(30),
                )
                .await?;
            if !matches!(response, RelayPeerResponse::LeasedComputerGrantUpdated {}) {
                return Err(host_error(
                    "MP-11: desktop revocation was not acknowledged".into(),
                ));
            }
        }
        Ok(())
    }

    pub(crate) async fn install_leased_computer_grant(
        &self,
        leased_agent_id: &str,
        epoch: &str,
        grant: Option<LeasedComputerGrant>,
    ) -> Result<(), DaemonError> {
        if !cfg!(target_os = "linux") || self.slice_kernel_id().is_some() {
            return Err(host_error(
                "MP-08: leased Computer requires an ordinary Linux desktop kernel".into(),
            ));
        }
        let agent = self
            .with_app_side_effect(|app| {
                crate::app::RemoteLeaseRuntime::new(app)
                    .leased_computer_agent(leased_agent_id, grant.as_ref())
            })
            .await?;
        let user = self.provider_account_authority_owner_user_id(agent.owner_user_id());
        // A delayed duplicate/revocation cannot retire a newer grant epoch.
        {
            let _change = self
                .owned
                .leased_computer_changes
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            let mut grants = self
                .owned
                .leased_computer_grants
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            if let Some(grant) = grant.as_ref() {
                if grant.epoch != epoch {
                    return Err(host_error("MP-11: Computer grant epoch mismatch".into()));
                }
                if grants.get(agent.id()) == Some(grant)
                    && self.owned.kernel_browser_host.is_loaded_for(
                        &user,
                        agent.id(),
                        crate::runtime::kernel_browser_host::KernelBrowserCapability::Computer,
                    )
                {
                    return Ok(());
                }
                grants.insert(agent.id().into(), grant.clone());
            } else {
                if grants
                    .get(agent.id())
                    .is_none_or(|current| current.epoch != epoch)
                {
                    return Ok(());
                }
                grants.remove(agent.id());
            }
            drop(grants);
            self.owned
                .kernel_browser_host
                .revoke_grants(&user, Some(agent.id()));
        }
        if let Some(grant) = grant {
            if grant.epoch.is_empty() || grant.epoch.len() > 128 {
                return Err(host_error("MP-11: invalid Computer grant epoch".into()));
            }
            let current = self
                .kernel_browser_operation(
                    &user,
                    None,
                    "host.computer",
                    serde_json::json!({"op":"state"}),
                )
                .await?;
            if current["surface_id"].as_str() != Some(&grant.target.surface_id)
                || current["generation"].as_str() != Some(&grant.target.generation)
            {
                return Err(host_error(
                    "MP-11: stale native surface or generation".into(),
                ));
            }
            {
                let _change = self
                    .owned
                    .leased_computer_changes
                    .lock()
                    .unwrap_or_else(|e| e.into_inner());
                let current = self
                    .owned
                    .leased_computer_grants
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .get(agent.id())
                    .cloned();
                if current.as_ref() != Some(&grant) {
                    return Err(host_error(
                        "MP-11: not_granted: desktop grant superseded".into(),
                    ));
                }
                self.owned.kernel_browser_host.grant_desktop(
                    &user,
                    agent.id(),
                    &grant.target.surface_id,
                );
            }
            self.refresh_user_domain_grants();
        }
        Ok(())
    }

    pub(crate) fn check_leased_computer_grant(
        &self,
        context: &crate::transport::relay_peer::RemoteExtensionInvocationContext,
        epoch: &str,
    ) -> Result<LeasedComputerGrant, DaemonError> {
        let agent = super::tool_dispatch::authorize_remote_home_context_for_peer(self, context)?;
        self.refresh_user_domain_grants();
        let user = self.provider_account_authority_owner_user_id(agent.owner_user_id());
        let session = self.owned.session_snapshot(agent.session_id())?;
        let grant = self
            .owned
            .leased_computer_grants
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(agent.id())
            .cloned()
            .ok_or_else(|| host_error("MP-11: not_granted: HOME desktop grant revoked".into()))?;
        if grant.epoch != epoch
            || grant.home_kernel_id != context.home_kernel_id
            || grant.home_session_id != context.home_session_id
            || grant.home_agent_id != context.home_agent_id
            || grant.leased_agent_id != context.leased_agent_id
            || grant.desktop_kernel_id != context.worker_kernel_id.as_deref().unwrap_or_default()
            || !self.owned.kernel_browser_host.is_loaded_for(
                &user,
                agent.id(),
                crate::runtime::kernel_browser_host::KernelBrowserCapability::Computer,
            )
            || !session.has_member(agent.owner_user_id())
            || session.status() == crate::session::SessionStatus::Ended
        {
            return Err(host_error(
                "MP-11: not_granted: stale or ended HOME desktop grant".into(),
            ));
        }
        Ok(grant)
    }

    pub(super) fn kernel_computer_agent(
        &self,
        run: &crate::provider::RuntimeProviderRun,
    ) -> Option<crate::agent::AgentInstance> {
        if !self
            .owned
            .provider_run_projection
            .is_leased_provider_run(run.id())
        {
            return self.kernel_browser_agent(run);
        }
        if run.state() == crate::provider::ProviderRunState::Ended
            || self.slice_kernel_id().is_some()
        {
            return None;
        }
        let agent = self
            .owned
            .agent_store
            .get_agent(run.agent_instance_id()?)
            .ok()?;
        let user = self.provider_account_authority_owner_user_id(agent.owner_user_id());
        let session = self.owned.session_snapshot(run.session_id()).ok()?;
        let grant = self
            .owned
            .leased_computer_grants
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(agent.id())
            .cloned()?;
        if grant.desktop_kernel_id != self.owned.config_projection.snapshot().daemon_id
            || agent.session_id() != run.session_id()
            || !session.has_member(agent.owner_user_id())
            || session.status() == crate::session::SessionStatus::Ended
            || !self.owned.kernel_browser_host.is_loaded_for(
                &user,
                agent.id(),
                crate::runtime::kernel_browser_host::KernelBrowserCapability::Computer,
            )
        {
            return None;
        }
        Some(agent)
    }

    pub(super) async fn leased_computer_tool_authority(
        &self,
        run: &crate::provider::RuntimeProviderRun,
    ) -> Result<Option<LeasedComputerGrant>, DaemonError> {
        if !self
            .owned
            .provider_run_projection
            .is_leased_provider_run(run.id())
        {
            return Ok(None);
        }
        self.refresh_user_domain_grants();
        let agent = self.kernel_computer_agent(run).ok_or_else(|| {
            host_error("MP-11: not_granted: explicit leased desktop grant required".into())
        })?;
        let grant = self
            .owned
            .leased_computer_grants
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(agent.id())
            .cloned()
            .ok_or_else(|| host_error("MP-11: not_granted: desktop grant revoked".into()))?;
        let context = self
            .with_app_side_effect(|app| {
                crate::app::RemoteLeaseRuntime::new(app)
                    .leased_extension_invocation_context_for_runtime_provider_run(run)
            })
            .await
            .ok_or_else(|| {
                host_error("MP-11: not_granted: live leased provider required".into())
            })?;
        if grant.home_kernel_id != context.home_kernel_id
            || grant.home_session_id != context.home_session_id
            || grant.home_agent_id != context.home_agent_id
            || grant.leased_agent_id != context.leased_agent_id
        {
            return Err(host_error(
                "MP-11: not_granted: Computer lease binding changed".into(),
            ));
        }
        let config = self.owned.config_projection.snapshot();
        let checked = self
            .send_worker_home_runtime_request(
                &config,
                &grant.home_kernel_id,
                RelayPeerRequest::CheckLeasedComputerGrant {
                    context,
                    epoch: grant.epoch.clone(),
                },
                Duration::from_secs(30),
            )
            .await;
        match checked {
            Ok(RelayPeerResponse::LeasedComputerGrantChecked { grant: current })
                if current == grant =>
            {
                Ok(Some(grant))
            }
            result => {
                self.retire_leased_computer_grant(
                    &self.provider_account_authority_owner_user_id(agent.owner_user_id()),
                    agent.id(),
                    Some(&grant.epoch),
                );
                Err(result.err().unwrap_or_else(|| {
                    host_error("MP-11: not_granted: HOME desktop authority changed".into())
                }))
            }
        }
    }
}
