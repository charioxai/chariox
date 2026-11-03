//! An agent exposes an installed App's tools to itself by asking for it: the
//! kernel checks that the App is installed for the agent's Room and binds it
//! at once, with no person in the loop. The binding is only exposure; every
//! call still goes through the installation's own trust and capability checks.
use super::*;
use crate::extension::{ExtensionGrant, ExtensionKind};

#[derive(PartialEq, Eq)]
enum AppGrantOrigin {
    Requested,
    DevAutomatic,
}

impl KernelRuntimeState {
    /// Exposes the installation to the agent and refreshes the agent's tool
    /// catalog when the App already lists tools.
    pub(crate) async fn grant_agent_app(
        &self,
        agent_ref: &str,
        grant: ExtensionGrant,
        caller_user_id: &str,
    ) -> Result<crate::agent::AgentInstance, DaemonError> {
        let agent = self
            .grant_agent_app_for_tool(agent_ref, grant, caller_user_id)
            .await?;
        #[cfg(any(target_os = "macos", all(target_os = "linux", target_env = "gnu")))]
        if self
            .app_control()
            .has_active_apps_for_agent(&self.app_reach(&agent))
        {
            self.refresh_agent_runtime_tool_catalog(agent.session_id(), agent.id())
                .await?;
        }
        Ok(agent)
    }

    /// The authenticated agent tool schedules its own continuation after the
    /// same grant path; it must not also schedule a separate provider reload.
    pub(crate) async fn grant_agent_app_for_tool(
        &self,
        agent_ref: &str,
        grant: ExtensionGrant,
        caller_user_id: &str,
    ) -> Result<crate::agent::AgentInstance, DaemonError> {
        match self
            .checked_app_grant(
                agent_ref,
                grant,
                caller_user_id,
                false,
                AppGrantOrigin::Requested,
            )
            .await?
        {
            Ok(agent) => Ok(agent),
            Err(NotBound::Busy) => Err(app_binding_error("App control is busy")),
            Err(NotBound::Refused) => Err(app_binding_error(
                "App installation is unavailable to this agent or its publisher verification is no longer valid",
            )),
        }
    }

    /// The durable dev installation exposes itself to the agents of its own
    /// Room. An agent whose owner revoked the binding is not bound again.
    pub(in crate::runtime::state) async fn grant_room_dev_app(
        &self,
        agent: &crate::agent::AgentInstance,
        installation: &str,
    ) -> Result<Option<crate::agent::AgentInstance>, DaemonError> {
        let owner = self.app_reach(agent).owner;
        let record = self
            .owned
            .durable_state_store
            .get_app_installation(&owner, installation)
            .map_err(|_| app_binding_error("Dev installation is unavailable"))?;
        if record.channel != chariox_app_runtime::installation::channel::InstallationChannel::Dev
            || !record.visible_in_room(Some(agent.session_id()))
        {
            return Err(app_binding_error(
                "Dev installation belongs to another Room",
            ));
        }
        if !self
            .owned
            .durable_state_store
            .dev_app_automatic_binding_enabled(agent.id(), installation)?
        {
            return Ok(None);
        }
        match self
            .checked_app_grant(
                agent.id(),
                ExtensionGrant::app(installation),
                agent.owner_user_id(),
                false,
                AppGrantOrigin::DevAutomatic,
            )
            .await?
        {
            Ok(agent) => Ok(Some(agent)),
            Err(NotBound::Busy) => Err(app_binding_error("App control is busy")),
            Err(NotBound::Refused) => Err(app_binding_error("Dev App is no longer available")),
        }
    }

    /// A fork copies its source's App bindings through the same checked and
    /// audited grant as any binding. It waits briefly for an App admission
    /// slot. None when the binding is not copied: the check refuses the App
    /// now (uninstalled, publisher revoked) or App control stayed busy; the
    /// fork itself still completes.
    pub(in crate::runtime::state) async fn copy_agent_app_grant(
        &self,
        agent_id: &str,
        grant: ExtensionGrant,
        caller_user_id: &str,
    ) -> Result<Option<crate::agent::AgentInstance>, DaemonError> {
        let installation = grant.name.clone();
        Ok(
            match self
                .checked_app_grant(
                    agent_id,
                    grant,
                    caller_user_id,
                    true,
                    AppGrantOrigin::Requested,
                )
                .await?
            {
                Ok(agent) => Some(agent),
                Err(NotBound::Busy) => {
                    tracing::warn!(%installation, "App control stayed busy; the fork does not copy this App binding");
                    None
                }
                Err(NotBound::Refused) => {
                    tracing::warn!(%installation, "App binding refused; the fork does not copy it");
                    None
                }
            },
        )
    }

    /// The App binding path: the installation is active, trusted and
    /// reachable from the agent's Room under an App admission slot (waiting
    /// for one when `wait_for_slot`), then the binding with its durable event,
    /// audit, leased manifest sync and workflow copy invalidation.
    async fn checked_app_grant(
        &self,
        agent_ref: &str,
        grant: ExtensionGrant,
        caller_user_id: &str,
        wait_for_slot: bool,
        origin: AppGrantOrigin,
    ) -> Result<Result<crate::agent::AgentInstance, NotBound>, DaemonError> {
        self.authorize_current_external_command()?;
        grant.validate_app_binding()?;
        self.owned
            .ensure_agent_extension_authority(agent_ref, caller_user_id, "bind agent App")?;
        let agent = self
            .owned
            .agent_store
            .get_agent(agent_ref)
            .or_else(|_| self.owned.agent_store.get_agent_by_ref(agent_ref))?;
        let permit = if wait_for_slot {
            tokio::time::timeout(COPY_ADMISSION_WAIT, self.app_control().admit())
                .await
                .ok()
                .flatten()
        } else {
            self.app_control().try_admit().ok()
        };
        let Some(permit) = permit else {
            return Ok(Err(NotBound::Busy));
        };
        let reach = self.app_reach(&agent);
        let store = self.owned.durable_state_store.clone();
        let installation_id = grant.name.clone();
        #[cfg(any(target_os = "macos", all(target_os = "linux", target_env = "gnu")))]
        let control = self.app_control().clone();
        let state = self.clone();
        let checked = tokio::task::spawn_blocking(move || {
            state.authorize_current_external_command()?;
            let _permit = permit;
            let binding =
                store.check_app_binding_in_room(&reach.owner, &installation_id, Some(&reach.room));
            // MP-11 SB-03: writer completion cannot revive a stale provider.
            state.authorize_current_external_command()?;
            #[cfg(any(target_os = "macos", all(target_os = "linux", target_env = "gnu")))]
            let reachable = binding.is_ok() && control.can_reach(&reach, &installation_id);
            #[cfg(not(any(target_os = "macos", all(target_os = "linux", target_env = "gnu"))))]
            let reachable = binding.is_ok();
            // Under the same slot: its tools are listed at once, even before
            // the App's first start. The dormant catalog is the owner's, and
            // only agents bound to the App list it.
            #[cfg(any(target_os = "macos", all(target_os = "linux", target_env = "gnu")))]
            if reachable {
                control.seed_dormant(&reach.owner, &installation_id);
            }
            Ok::<_, DaemonError>(reachable)
        })
        .await
        .map_err(|_| app_binding_error("App binding check did not complete"))??;
        if !checked {
            return Ok(Err(NotBound::Refused));
        }
        self.authorize_current_external_command()?;
        let agent = {
            let mut agents = self.owned.agent_store.write();
            let current = agents.get_agent(agent.id())?;
            if current.owner_user_id() != agent.owner_user_id()
                || current.session_id() != agent.session_id()
                || current.remote_execution() != agent.remote_execution()
            {
                return Err(app_binding_error("App target changed"));
            }
            if origin == AppGrantOrigin::Requested {
                // Serialize the choice with an explicit revoke. The later grant
                // audit may arrive after a revoke and cannot re-enable dev.
                self.owned.durable_state_store.append_event(
                    "app.binding.owner_granted",
                    Some(agent.id().into()),
                    serde_json::json!({"installation_id":grant.name}),
                )?;
            }
            if current.has_extension_grant(ExtensionKind::App, &grant.name) {
                return Ok(Ok(current));
            }
            agents.grant_extension(agent.id(), grant.clone())?
        };
        self.owned.session_snapshot(agent.session_id())?;
        self.append_agent_durable_event(
            "agent.extension_granted",
            &agent,
            Some(&format!("app:{}", grant.name)),
        )
        .await?;
        self.append_home_extension_grant_audit_event(
            "home_extension.grant.created",
            &agent,
            caller_user_id,
            &grant,
        )?;
        self.sync_remote_extension_manifest_for_agent(&agent, Some(caller_user_id), Some(false))
            .await?;
        self.invalidate_workflow_copies_after_source_agent_change(agent.session_id(), agent.id())?;
        #[cfg(any(target_os = "macos", all(target_os = "linux", target_env = "gnu")))]
        self.app_control()
            .note_unlisted_binding(&self.app_reach(&agent), &grant.name);
        Ok(Ok(agent))
    }

    /// Agents whose App tool listing left out a bound App that has since
    /// started get the same refresh as after a grant: a leased agent's
    /// manifest, then the provider's catalog, which lists under the usual
    /// admission rules.
    #[cfg(any(target_os = "macos", all(target_os = "linux", target_env = "gnu")))]
    pub(crate) fn refresh_started_app_catalogs(&self) {
        for agent_id in self.app_control().take_started_app_refreshes() {
            let Ok(agent) = self.owned.agent_store.get_agent(&agent_id) else {
                continue;
            };
            let state = self.clone();
            tokio::spawn(async move {
                if let Err(error) = state
                    .sync_remote_extension_manifest_for_agent(&agent, None, Some(false))
                    .await
                {
                    tracing::debug!(%error, "started App manifest sync failed");
                }
                if let Err(error) = state
                    .refresh_agent_runtime_tool_catalog(agent.session_id(), agent.id())
                    .await
                {
                    tracing::debug!(%error, "started App catalog refresh failed");
                }
            });
        }
    }

    /// Preserve the broken binding for inspectors while the inactive App has
    /// no authority or tools. The existing reinstall gate removes it before
    /// an approved release can become active, so reinstall never regrants it.
    pub(crate) async fn refresh_uninstalled_app_bindings(&self, owner: &str, installation: &str) {
        self.app_control()
            .views()
            .forget_installation(owner, installation);
        for agent in self.owned.agent_store.list_agents() {
            if self.app_reach(&agent).owner != owner
                || !agent.has_extension_grant(ExtensionKind::App, installation)
            {
                continue;
            }
            if let Err(error) = self.invalidate_workflow_copies_after_source_agent_change(
                agent.session_id(),
                agent.id(),
            ) {
                tracing::debug!(%error, "uninstalled App workflow copy invalidation failed");
            }
            if let Err(error) = self
                .sync_remote_extension_manifest_for_agent(&agent, Some(owner), Some(true))
                .await
            {
                tracing::debug!(%error, "uninstalled App manifest refresh failed");
            }
            if let Err(error) = self
                .refresh_agent_runtime_tool_catalog(agent.session_id(), agent.id())
                .await
            {
                tracing::debug!(%error, "uninstalled App catalog refresh failed");
            }
        }
    }

    pub(super) async fn revoke_agent_app(
        &self,
        agent_ref: &str,
        name: &str,
        caller_user_id: &str,
    ) -> Result<crate::agent::AgentInstance, DaemonError> {
        ExtensionGrant::app(name).validate_app_binding()?;
        self.owned.ensure_agent_extension_authority(
            agent_ref,
            caller_user_id,
            "revoke agent App",
        )?;
        let agent = {
            let mut agents = self.owned.agent_store.write();
            let agent = agents
                .get_agent(agent_ref)
                .or_else(|_| agents.get_agent_by_ref(agent_ref))?;
            self.owned.durable_state_store.append_event(
                "app.binding.owner_revoked",
                Some(agent.id().into()),
                serde_json::json!({"installation_id":name}),
            )?;
            agents.revoke_extension(agent.id(), ExtensionKind::App, name)?
        };
        self.owned.session_snapshot(agent.session_id())?;
        self.record_revoked_app_binding(&agent, name, caller_user_id)?;
        self.propagate_revoked_app_binding(agent, caller_user_id)
            .await
    }

    /// The durable record of a removed binding, written before anything async.
    pub(in crate::runtime::state) fn record_revoked_app_binding(
        &self,
        agent: &crate::agent::AgentInstance,
        name: &str,
        caller_user_id: &str,
    ) -> Result<(), DaemonError> {
        // The foreground App is not bound to this agent again on a focus change.
        self.app_control()
            .views()
            .set_revoked(agent.session_id(), agent.id(), name, true);
        self.app_control()
            .views()
            .forget_agent_approvals(agent.id(), name);
        #[cfg(any(target_os = "macos", all(target_os = "linux", target_env = "gnu")))]
        self.app_control()
            .forget_unlisted_binding(&self.app_reach(agent), name);
        self.record_agent_durable_event(
            "agent.extension_revoked",
            agent,
            Some(&format!("app:{name}")),
        )?;
        self.append_home_extension_named_grant_audit_event(
            "home_extension.grant.revoked",
            agent,
            caller_user_id,
            ExtensionKind::App,
            name,
        )
    }

    /// Manifest, workflow-copy and provider-catalog follow-up of a revoke.
    pub(in crate::runtime::state) async fn propagate_revoked_app_binding(
        &self,
        agent: crate::agent::AgentInstance,
        caller_user_id: &str,
    ) -> Result<crate::agent::AgentInstance, DaemonError> {
        self.sync_remote_extension_manifest_for_agent(&agent, Some(caller_user_id), Some(true))
            .await?;
        self.invalidate_workflow_copies_after_source_agent_change(agent.session_id(), agent.id())?;
        self.refresh_agent_runtime_tool_catalog(agent.session_id(), agent.id())
            .await?;
        Ok(agent)
    }
}

/// Why the App binding path saved no binding.
enum NotBound {
    /// No App admission slot was free.
    Busy,
    /// The installation is not an active, verified App the agent's Room reaches.
    Refused,
}

/// How long a fork waits for an App admission slot to copy one binding.
const COPY_ADMISSION_WAIT: std::time::Duration = std::time::Duration::from_secs(10);

fn app_binding_error(message: &str) -> DaemonError {
    DaemonError::LocalTransport {
        operation: "agent.extension.app",
        message: message.into(),
    }
}
