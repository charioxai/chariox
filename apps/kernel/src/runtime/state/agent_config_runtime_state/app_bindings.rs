use super::*;
use crate::extension::{ExtensionGrant, ExtensionKind};

impl KernelRuntimeState {
    pub(super) async fn grant_agent_app(
        &self,
        agent_ref: &str,
        grant: ExtensionGrant,
        caller_user_id: &str,
    ) -> Result<crate::agent::AgentInstance, DaemonError> {
        let agent = self
            .grant_agent_app_for_tool(agent_ref, grant, caller_user_id)
            .await?;
        #[cfg(any(target_os = "macos", all(target_os = "linux", target_env = "gnu")))]
        if self.app_control().has_active_apps_for_agent(&agent) {
            self.refresh_agent_runtime_tool_catalog(agent.session_id(), agent.id())
                .await?;
        }
        Ok(agent)
    }

    /// The authenticated agent tool schedules its own continuation after the
    /// same grant path; it must not also schedule a separate provider reload.
    pub(in crate::runtime::state) async fn grant_agent_app_for_tool(
        &self,
        agent_ref: &str,
        grant: ExtensionGrant,
        caller_user_id: &str,
    ) -> Result<crate::agent::AgentInstance, DaemonError> {
        match self
            .checked_app_grant(agent_ref, grant, caller_user_id, false)
            .await?
        {
            Ok(agent) => Ok(agent),
            Err(NotBound::Busy) => Err(app_binding_error("App control is busy")),
            Err(NotBound::Refused) => Err(app_binding_error(
                "App installation is unavailable or its publisher verification is no longer valid",
            )),
        }
    }

    /// A fork copies its source's App bindings, the owner's explicit choice,
    /// through the same checked and audited grant as any binding. It waits
    /// briefly for an App admission slot. None when the binding is not
    /// copied: the check refuses the App now (uninstalled, publisher revoked)
    /// or App control stayed busy; the fork itself still completes.
    pub(in crate::runtime::state) async fn copy_agent_app_grant(
        &self,
        agent_id: &str,
        grant: ExtensionGrant,
        caller_user_id: &str,
    ) -> Result<Option<crate::agent::AgentInstance>, DaemonError> {
        let installation = grant.name.clone();
        Ok(
            match self
                .checked_app_grant(agent_id, grant, caller_user_id, true)
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

    /// The App binding path: owner authority, the installation and publisher
    /// check under an App admission slot (waiting for one when
    /// `wait_for_slot`), then the grant with its durable event, audit, leased
    /// manifest sync and workflow copy invalidation.
    async fn checked_app_grant(
        &self,
        agent_ref: &str,
        grant: ExtensionGrant,
        caller_user_id: &str,
        wait_for_slot: bool,
    ) -> Result<Result<crate::agent::AgentInstance, NotBound>, DaemonError> {
        self.authorize_current_forwarded_binding()?;
        grant.validate_app_binding()?;
        self.owned.ensure_agent_extension_authority(
            agent_ref,
            caller_user_id,
            "grant agent App",
        )?;
        let agent = self
            .owned
            .agent_store
            .get_agent(agent_ref)
            .or_else(|_| self.owned.agent_store.get_agent_by_ref(agent_ref))?;
        let store = self.owned.durable_state_store.clone();
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
        let owner = caller_user_id.to_string();
        let installation_id = grant.name.clone();
        #[cfg(any(target_os = "macos", all(target_os = "linux", target_env = "gnu")))]
        let control = self.app_control().clone();
        let state = self.clone();
        let checked = tokio::task::spawn_blocking(move || {
            state.authorize_current_forwarded_binding()?;
            let _permit = permit;
            let binding = store.check_app_binding(&owner, &installation_id);
            // Under the same slot: its tools are listed at once, even before
            // the App's first start. The dormant catalog is the owner's, and
            // only agents bound to the App list it.
            #[cfg(any(target_os = "macos", all(target_os = "linux", target_env = "gnu")))]
            if binding.is_ok() {
                control.seed_dormant(&owner, &installation_id);
            }
            Ok::<_, DaemonError>(binding)
        })
        .await
        .map_err(|_| app_binding_error("App binding check did not complete"))??;
        if checked.is_err() {
            return Ok(Err(NotBound::Refused));
        }
        // Ownership is checked again after the writer wait. A binding does not
        // retain invocation authority if the App is subsequently retired/revoked.
        self.authorize_current_forwarded_binding()?;
        let agent = self
            .owned
            .grant_agent_extension(agent.id(), grant.clone(), caller_user_id)?;
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
            .note_unlisted_binding(&agent, &grant.name);
        Ok(Ok(agent))
    }

    /// Agents whose App tool listing left out an App that has since started
    /// get the same refresh as after a grant: a leased agent's manifest, then
    /// the provider's catalog, which lists under the usual admission rules.
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
            if agent.owner_user_id() != owner
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
        let grant = ExtensionGrant::app(name);
        grant.validate_app_binding()?;
        // Revocation must work even after uninstall or publisher revocation.
        let agent = self.owned.revoke_agent_extension(
            agent_ref,
            ExtensionKind::App,
            name,
            caller_user_id,
        )?;
        // The foreground App is not bound to this agent again on a focus change.
        self.app_control()
            .views()
            .set_revoked(agent.session_id(), agent.id(), name, true);
        #[cfg(any(target_os = "macos", all(target_os = "linux", target_env = "gnu")))]
        self.app_control().forget_unlisted_binding(&agent, name);
        self.append_agent_durable_event(
            "agent.extension_revoked",
            &agent,
            Some(&format!("app:{name}")),
        )
        .await?;
        self.append_home_extension_named_grant_audit_event(
            "home_extension.grant.revoked",
            &agent,
            caller_user_id,
            ExtensionKind::App,
            name,
        )?;
        self.sync_remote_extension_manifest_for_agent(&agent, Some(caller_user_id), Some(true))
            .await?;
        self.invalidate_workflow_copies_after_source_agent_change(agent.session_id(), agent.id())?;
        self.refresh_agent_runtime_tool_catalog(agent.session_id(), agent.id())
            .await?;
        Ok(agent)
    }

    /// Agent-initiated binding uses the existing session/agent permission mode
    /// and RuntimeInteraction. Direct authenticated user grants skip this helper.
    pub(crate) async fn authorize_agent_app_binding(
        &self,
        session_id: &str,
        initiating_agent_id: &str,
        target_agent_id: &str,
        installation_id: &str,
    ) -> Result<bool, DaemonError> {
        ExtensionGrant::app(installation_id).validate_app_binding()?;
        let session = self.owned.session_store.get_session(session_id)?;
        let agent = self.owned.agent_store.get_agent(initiating_agent_id)?;
        let target = self.owned.agent_store.get_agent(target_agent_id)?;
        if agent.session_id() != session.id()
            || target.session_id() != session.id()
            || agent.owner_user_id() != target.owner_user_id()
        {
            return Err(app_binding_error(
                "App binding target is outside this agent's authority",
            ));
        }
        self.owned.ensure_agent_extension_authority(
            target.id(),
            agent.owner_user_id(),
            "request agent App binding",
        )?;
        if target.has_extension_grant(ExtensionKind::App, installation_id) {
            return Ok(true);
        }
        if !crate::session::effective_agent_user_authority(&session, Some(&agent))
            .requires_approval()
        {
            return Ok(true);
        }
        let interaction = crate::session::RuntimeInteraction::new(
            format!(
                "app-binding-permission-{}-{:016x}",
                agent.id(),
                rand::random::<u64>()
            ),
            agent.id(),
            crate::session::RuntimeInteractionKind::Permission,
            crate::session::RuntimeInteractionLevel::Warning,
            Some("Chariox App binding approval".into()),
            format!(
                "Allow agent `{}` to bind App installation `{installation_id}` to agent `{}`?",
                agent.agent_ref(),
                target.agent_ref()
            ),
            vec![
                crate::session::RuntimeInteractionChoice::new(
                    "allow",
                    "Allow",
                    "allow",
                    Some(crate::session::RuntimeInteractionChoiceStyle::Primary),
                ),
                crate::session::RuntimeInteractionChoice::new(
                    "deny",
                    "Deny",
                    "deny",
                    Some(crate::session::RuntimeInteractionChoiceStyle::Danger),
                ),
            ],
            None,
            None,
            None,
        );
        let resolution = self
            .create_runtime_interaction(session.id(), interaction)
            .await?
            .await
            .map_err(|_| app_binding_error("App binding approval closed without a decision"))?;
        // A deleted/reassigned initiating agent must not retain a pending grant.
        let current = self.owned.agent_store.get_agent(agent.id())?;
        if current.owner_user_id() != agent.owner_user_id() || current.session_id() != session.id()
        {
            return Err(app_binding_error(
                "App binding caller changed while approval was pending",
            ));
        }
        Ok(resolution.choice_id.as_deref() == Some("allow"))
    }
}

/// Why the App binding path saved no binding.
enum NotBound {
    /// No App admission slot was free.
    Busy,
    /// The installation is not the owner's active, verified App.
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
