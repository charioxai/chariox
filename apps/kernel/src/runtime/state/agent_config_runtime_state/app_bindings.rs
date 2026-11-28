use super::*;
use crate::extension::{ExtensionGrant, ExtensionKind};
use crate::runtime::state::capability_grant_runtime::AppBindingPermit;

#[derive(PartialEq, Eq)]
enum AppGrantOrigin {
    Requested,
    DevAutomatic,
}

impl KernelRuntimeState {
    pub(crate) async fn grant_agent_app(
        &self,
        agent_ref: &str,
        grant: ExtensionGrant,
        caller_user_id: &str,
        permit: Option<AppBindingPermit>,
    ) -> Result<crate::agent::AgentInstance, DaemonError> {
        let agent = self
            .grant_agent_app_for_tool(agent_ref, grant, caller_user_id, permit)
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
    pub(crate) async fn grant_agent_app_for_tool(
        &self,
        agent_ref: &str,
        grant: ExtensionGrant,
        caller_user_id: &str,
        permit: Option<AppBindingPermit>,
    ) -> Result<crate::agent::AgentInstance, DaemonError> {
        match self
            .checked_app_grant(
                agent_ref,
                grant,
                caller_user_id,
                false,
                permit,
                AppGrantOrigin::Requested,
            )
            .await?
        {
            Ok(agent) => Ok(agent),
            Err(NotBound::Busy) => Err(app_binding_error("App control is busy")),
            Err(NotBound::Refused) => Err(app_binding_error(
                "App installation is unavailable or its publisher verification is no longer valid",
            )),
        }
    }

    /// The durable dev installation authorizes automatic bindings in its own
    /// Room. Reuse the normal revocation epoch so an owner revoke wins a race.
    pub(in crate::runtime::state) async fn grant_room_dev_app(
        &self,
        agent: &crate::agent::AgentInstance,
        installation: &str,
    ) -> Result<Option<crate::agent::AgentInstance>, DaemonError> {
        let record = self
            .owned
            .durable_state_store
            .get_app_installation(agent.owner_user_id(), installation)
            .map_err(|_| app_binding_error("Dev installation is unavailable"))?;
        if record.channel != chariox_app_runtime::installation::channel::InstallationChannel::Dev
            || !record.visible_in_room(Some(agent.session_id()))
        {
            return Err(app_binding_error(
                "Dev installation belongs to another Room",
            ));
        }
        let permit = AppBindingPermit {
            actor: agent.clone(),
            target_id: agent.id().into(),
            target_binding: agent.remote_execution().cloned(),
            installation: installation.into(),
            target_revision: self.app_binding_revision(agent.id(), installation),
            provider_run_id: None,
        };
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
                Some(permit),
                AppGrantOrigin::DevAutomatic,
            )
            .await?
        {
            Ok(agent) => Ok(Some(agent)),
            Err(NotBound::Busy) => Err(app_binding_error("App control is busy")),
            Err(NotBound::Refused) => Err(app_binding_error("Dev App is no longer available")),
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
        if self.room_agent_tools_enabled() && self.agent_app_binding_origin() {
            // MP-11: an agent's provider fork does not inherit owner-only App authority.
            return Ok(None);
        }
        let installation = grant.name.clone();
        Ok(
            match self
                .checked_app_grant(
                    agent_id,
                    grant,
                    caller_user_id,
                    true,
                    None,
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

    fn agent_app_binding_origin(&self) -> bool {
        self.room_provider_origin.is_some()
            || self.room_request_origin.is_some()
            || self.external_command_authority.is_some()
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
        binding_permit: Option<AppBindingPermit>,
        origin: AppGrantOrigin,
    ) -> Result<Result<crate::agent::AgentInstance, NotBound>, DaemonError> {
        if self.room_agent_tools_enabled()
            && binding_permit.is_none()
            && self.agent_app_binding_origin()
        {
            return Err(app_binding_error(
                "agent App acquisition needs owner approval or an explicit held subset",
            ));
        }
        self.authorize_current_external_command()?;
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
        let room = agent.session_id().to_owned();
        let checked = tokio::task::spawn_blocking(move || {
            state.authorize_current_external_command()?;
            let _permit = permit;
            let binding = store.check_app_binding_in_room(&owner, &installation_id, Some(&room));
            // MP-11 SB-03: writer completion cannot revive a stale provider.
            state.authorize_current_external_command()?;
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
        self.authorize_current_external_command()?;
        if let Some(permit) = &binding_permit {
            if permit.target_id != agent.id() || permit.installation != grant.name {
                return Err(app_binding_error("App grant scope changed"));
            }
        }
        // MP-11: revocation and binding serialize at the same boundary.
        let agent = {
            // MP-11 SB-03: hold run replacement/termination through mutation.
            let providers = self.owned.provider_store.read();
            let mut epochs = self
                .owned
                .app_grant_epochs
                .revisions
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            let key = (agent.id().to_string(), grant.name.clone());
            let mut agents = self.owned.agent_store.write();
            let current = agents.get_agent(agent.id())?;
            if current.owner_user_id() != agent.owner_user_id()
                || current.session_id() != agent.session_id()
            {
                return Err(app_binding_error("App target changed"));
            }
            if current.remote_execution() != agent.remote_execution() {
                return Err(app_binding_error("App target placement changed"));
            }
            if let Some(permit) = &binding_permit {
                let current_actor = agents.get_agent(permit.actor.id())?;
                if current_actor.owner_user_id() != permit.actor.owner_user_id()
                    || current_actor.session_id() != permit.actor.session_id()
                    || current_actor.remote_execution() != permit.actor.remote_execution()
                    || current.remote_execution() != permit.target_binding.as_ref()
                {
                    return Err(app_binding_error("App acquisition caller changed"));
                }
                if let Some(run) = &permit.provider_run_id {
                    // A home lease has the projected worker epoch, not a local run.
                    let current_run_matches = match current_actor.remote_execution() {
                        Some(binding) => binding
                            .active_worker_provider_run_id
                            .as_deref()
                            .is_some_and(|worker| {
                                crate::provider::projected_leased_provider_run_id(
                                    &binding.leased_agent_id,
                                    worker,
                                ) == *run
                            }),
                        None => providers
                            .get_run_for_agent(current_actor.session_id(), current_actor.id())
                            .is_some_and(|current_run| {
                                current_run.id() == run
                                    && current_run.owner_user_id() == current_actor.owner_user_id()
                            }),
                    };
                    if !current_run_matches {
                        return Err(app_binding_error("App acquisition provider run changed"));
                    }
                }
                if epochs
                    .get(&key)
                    .map_or(0, |epoch| epoch.load(std::sync::atomic::Ordering::Acquire))
                    != permit.target_revision
                {
                    return Err(app_binding_error(
                        "App binding changed while acquisition was pending",
                    ));
                }
            }
            let record_owner_choice = || {
                if origin == AppGrantOrigin::Requested {
                    // Serialize the owner's choice with revoke. The later grant
                    // audit may arrive after a revoke and cannot re-enable dev.
                    self.owned.durable_state_store.append_event(
                        "app.binding.owner_granted",
                        Some(agent.id().into()),
                        serde_json::json!({"installation_id":grant.name}),
                    )?;
                }
                Ok::<_, DaemonError>(())
            };
            if current.has_extension_grant(ExtensionKind::App, &grant.name) {
                record_owner_choice()?;
                return Ok(Ok(current));
            }
            let mut commit = || {
                record_owner_choice()?;
                agents.grant_extension(agent.id(), grant.clone())
            };
            let agent = commit()?;
            epochs
                .entry(key)
                .or_default()
                .fetch_add(1, std::sync::atomic::Ordering::AcqRel);
            agent
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

    /// Preserve the binding while the inactive App has no authority or tools:
    /// inspectors see it as broken, and reinstalling the kept installation
    /// makes it live again.
    pub(crate) async fn refresh_uninstalled_app_bindings(&self, owner: &str, installation: &str) {
        self.app_control()
            .views()
            .forget_installation(owner, installation);
        for agent in self.owned.agent_store.list_agents() {
            if !self.acts_for_owner(agent.session_id(), owner, agent.owner_user_id())
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
        self.revoke_app_binding_set(agent_ref, name, caller_user_id)
            .await
    }

    async fn revoke_app_binding_set(
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
        let root = {
            let mut epochs = self
                .owned
                .app_grant_epochs
                .revisions
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            let mut agents = self.owned.agent_store.write();
            let root = agents
                .get_agent(agent_ref)
                .or_else(|_| agents.get_agent_by_ref(agent_ref))?;
            self.owned.durable_state_store.append_event(
                "app.binding.owner_revoked",
                Some(root.id().to_string()),
                serde_json::json!({"installation_id":name}),
            )?;
            let revoked = agents.revoke_extension(root.id(), ExtensionKind::App, name)?;
            epochs
                .entry((root.id().to_string(), name.to_string()))
                .or_default()
                .fetch_add(1, std::sync::atomic::Ordering::AcqRel);
            revoked
        };
        self.owned.session_snapshot(root.session_id())?;
        self.finish_revoked_app_binding(root.clone(), name, caller_user_id)
            .await?;
        Ok(root)
    }

    async fn finish_revoked_app_binding(
        &self,
        agent: crate::agent::AgentInstance,
        name: &str,
        caller_user_id: &str,
    ) -> Result<crate::agent::AgentInstance, DaemonError> {
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
        #[cfg(any(target_os = "macos", all(target_os = "linux", target_env = "gnu")))]
        self.app_control().forget_unlisted_binding(agent, name);
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

    /// Agent-initiated binding uses the existing session/agent permission mode
    /// and RuntimeInteraction. Direct authenticated user grants skip this helper.
    pub(crate) async fn authorize_agent_app_binding(
        &self,
        session_id: &str,
        initiating_agent_id: &str,
        target_agent_id: &str,
        installation_id: &str,
    ) -> Result<Option<AppBindingPermit>, DaemonError> {
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
        let permit = AppBindingPermit {
            actor: agent.clone(),
            target_id: target.id().into(),
            target_binding: target.remote_execution().cloned(),
            installation: installation_id.into(),
            target_revision: self.app_binding_revision(target.id(), installation_id),
            provider_run_id: self
                .room_provider_origin
                .as_ref()
                .map(|(_, run)| run.clone()),
        };
        self.authorize_current_external_command()?;
        if self.room_agent_tools_enabled() {
            if self.slice_kernel_id().is_some() {
                return Err(app_binding_error(
                    "leased agents use their Room Browser/Computer route",
                ));
            }
            if agent.id() != target.id() {
                let agents = self
                    .owned
                    .agent_store
                    .get_session_agents(agent.session_id());
                crate::runtime::room_tool_admission::in_subtree(&agent, &target, &agents)?;
            }
            // Only the owner's trusted, available App binds; publication and
            // invocation recheck it through the existing App admission path.
            self.owned
                .durable_state_store
                .check_app_binding_in_room(
                    agent.owner_user_id(),
                    installation_id,
                    Some(agent.session_id()),
                )
                .map_err(|_| app_binding_error("App installation is not trusted and available"))?;
            // Every agent may use the owner's trusted Apps, as it has the kernel browser.
            return Ok(Some(permit));
        }
        if target.has_extension_grant(ExtensionKind::App, installation_id)
            || self
                .owned
                .agent_store
                .effective_execution_config(&session, &agent, &self.owned.session_store)
                .permission_level
                .is_yolo()
            || self
                .sudo_task_authorizes_agent(session.id(), agent.id())
                .await
        {
            return Ok(Some(permit));
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
            .create_runtime_interaction(
                crate::runtime::state::InteractionSurface::named("app_binding_permission"),
                session.id(),
                interaction,
            )
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
        Ok((resolution.choice_id.as_deref() == Some("allow")).then_some(permit))
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
