use super::*;
use crate::extension::{AppCapabilityGrant, ExtensionGrant, ExtensionKind};
use crate::runtime::state::capability_grant_runtime::{AppBindingPermit, AppPromptDelivery};

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
            .checked_app_grant(agent_ref, grant, caller_user_id, false, permit)
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
        mut grant: ExtensionGrant,
        caller_user_id: &str,
    ) -> Result<Option<crate::agent::AgentInstance>, DaemonError> {
        if self.room_agent_tools_enabled() && self.agent_app_binding_origin() {
            // MP-11: an agent's provider fork does not inherit owner-only App authority.
            return Ok(None);
        }
        let installation = grant.name.clone();
        grant.app_grant = None; // An explicit owner fork is a new checked grant.
        Ok(
            match self
                .checked_app_grant(agent_id, grant, caller_user_id, true, None)
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
        mut grant: ExtensionGrant,
        caller_user_id: &str,
        wait_for_slot: bool,
        binding_permit: Option<AppBindingPermit>,
    ) -> Result<Result<crate::agent::AgentInstance, NotBound>, DaemonError> {
        if self.room_agent_tools_enabled() {
            self.require_app_grant_wake()?;
        }
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
        let checked = tokio::task::spawn_blocking(move || {
            state.authorize_current_external_command()?;
            let _permit = permit;
            let binding = store.check_app_binding(&owner, &installation_id);
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
            if let Some(prompt) = &permit.prompt_id {
                let current = self.owned.agent_store.get_agent(permit.actor.id())?;
                if self.owner_requested_prompt(&current).as_deref() != Some(prompt.as_str())
                    || self.app_prompt_delivery(&current) != permit.prompt_delivery
                {
                    return Err(app_binding_error("App request's owner turn changed"));
                }
            }
        }
        if self.room_agent_tools_enabled() {
            let parent_cause = binding_permit
                .as_ref()
                .and_then(|permit| permit.parent.as_ref())
                .and_then(|grant| grant.app_grant.as_ref());
            grant.app_grant = Some(AppCapabilityGrant {
                grant_id: format!("app-grant-{:032x}", rand::random::<u128>()),
                expires_at_ms: parent_cause
                    .map(|cause| cause.expires_at_ms)
                    .unwrap_or_else(|| {
                        let lifetime =
                            crate::runtime::kernel_browser_host::KernelBrowserHost::grant_lifetime(
                                None,
                            )
                            .expect("default lifetime is valid");
                        crate::session::unix_epoch_ms().saturating_add(lifetime.as_millis() as u64)
                    }),
                prompt_id: binding_permit
                    .as_ref()
                    .and_then(|permit| permit.prompt_id.clone()),
                delegated_by_agent_id: parent_cause
                    .and_then(|_| binding_permit.as_ref().map(|p| p.actor.id().to_string())),
                delegated_from_grant_id: parent_cause.map(|cause| cause.grant_id.clone()),
            });
        }
        // MP-11: revocation and child delegation serialize at the same boundary.
        let agent = {
            if self.room_agent_tools_enabled() {
                self.require_app_grant_wake()?;
            }
            let acquisition_session = binding_permit
                .as_ref()
                .filter(|permit| permit.prompt_id.is_some())
                .map(|permit| {
                    self.owned
                        .session_store
                        .get_session(permit.actor.session_id())
                })
                .transpose()?;
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
            if self.room_agent_tools_enabled() && current.remote_execution().is_some() {
                return Err(app_binding_error(
                    "leased agents use their Room Browser/Computer route",
                ));
            }
            if let Some(permit) = &binding_permit {
                let current_actor = agents.get_agent(permit.actor.id())?;
                if current_actor.owner_user_id() != permit.actor.owner_user_id()
                    || current_actor.session_id() != permit.actor.session_id()
                    || current_actor.remote_execution().is_some()
                {
                    return Err(app_binding_error("App acquisition caller changed"));
                }
                if let Some(run) = &permit.provider_run_id {
                    if !providers
                        .get_run_for_agent(current_actor.session_id(), current_actor.id())
                        .is_some_and(|current_run| {
                            current_run.id() == run
                                && current_run.owner_user_id() == current_actor.owner_user_id()
                        })
                    {
                        return Err(app_binding_error("App acquisition provider run changed"));
                    }
                }
                if *epochs.get(&key).unwrap_or(&0) != permit.target_revision {
                    return Err(app_binding_error(
                        "App binding changed while acquisition was pending",
                    ));
                }
                if let Some(parent) = &permit.parent {
                    let current_parent = agents.get_agent(permit.actor.id())?;
                    if !current_parent.has_extension_grant(ExtensionKind::App, &grant.name)
                        || !current_parent.extension_grants().contains(parent)
                        || current.remote_execution().is_some()
                    {
                        return Err(app_binding_error(
                            "App delegation source expired or was revoked",
                        ));
                    }
                }
            }
            if self.room_agent_tools_enabled()
                && current.has_extension_grant(ExtensionKind::App, &grant.name)
                && current.extension_grants().iter().any(|existing| {
                    existing.kind == ExtensionKind::App
                        && existing.name == grant.name
                        && existing.app_grant.is_some()
                })
            {
                // MP-11: an idempotent grant cannot renew its lifetime or orphan
                // descendants by silently replacing their source generation.
                return Ok(Ok(current));
            }
            let mut commit = || agents.grant_extension(agent.id(), grant.clone());
            let agent = if let Some(permit) = binding_permit
                .as_ref()
                .filter(|permit| permit.prompt_id.is_some())
            {
                let session = acquisition_session
                    .as_ref()
                    .expect("App prompt permit has a session");
                self.owned.prompt_state_owner.with_active_prompt(
                    session,
                    permit.actor.id(),
                    |prompt| {
                        if prompt.is_none_or(|prompt| {
                            prompt.status() != crate::session::PromptStatus::Running
                                || Some(AppPromptDelivery::from_prompt(prompt))
                                    != permit.prompt_delivery
                        }) {
                            return Err(app_binding_error("App request's prompt delivery changed"));
                        }
                        commit()
                    },
                )?
            } else {
                commit()?
            };
            *epochs.entry(key).or_default() += 1;
            agent
        };
        self.owned.session_snapshot(agent.session_id())?;
        self.notify_app_grant_change();
        if let Some(cause) = &grant.app_grant {
            self.arm_app_grant_expiry(agent.id(), &grant.name, cause);
        }
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
        self.revoke_app_binding_set(agent_ref, name, caller_user_id, None)
            .await
    }

    pub(in crate::runtime::state) async fn revoke_expired_agent_app(
        &self,
        agent_id: &str,
        name: &str,
        grant_id: &str,
    ) -> Result<crate::agent::AgentInstance, DaemonError> {
        let owner = self
            .owned
            .agent_store
            .get_agent(agent_id)?
            .owner_user_id()
            .to_string();
        self.revoke_app_binding_set(agent_id, name, &owner, Some(grant_id))
            .await
    }

    async fn revoke_app_binding_set(
        &self,
        agent_ref: &str,
        name: &str,
        caller_user_id: &str,
        expected: Option<&str>,
    ) -> Result<crate::agent::AgentInstance, DaemonError> {
        ExtensionGrant::app(name).validate_app_binding()?;
        self.owned.ensure_agent_extension_authority(
            agent_ref,
            caller_user_id,
            "revoke agent App",
        )?;
        let (root, revoked) = {
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
            let cause = root
                .extension_grants()
                .iter()
                .find(|g| g.kind == ExtensionKind::App && g.name == name)
                .and_then(|g| g.app_grant.as_ref());
            if expected.is_some_and(|id| cause.is_none_or(|cause| cause.grant_id != id)) {
                return Ok(root); // A newer grant replaced this timer's binding.
            }
            let mut ids = vec![root.id().to_string()];
            let mut source_ids: Vec<String> = cause
                .map(|cause| vec![cause.grant_id.clone()])
                .unwrap_or_default();
            let candidates = agents.get_session_agents(root.session_id());
            let mut index = 0;
            while index < source_ids.len() {
                for child in &candidates {
                    let cause = child
                        .extension_grants()
                        .iter()
                        .find(|g| g.kind == ExtensionKind::App && g.name == name)
                        .and_then(|g| g.app_grant.as_ref());
                    if let Some(cause) = cause.filter(|cause| {
                        cause.delegated_from_grant_id.as_deref() == Some(source_ids[index].as_str())
                    }) {
                        if !ids.iter().any(|id| id == child.id()) {
                            ids.push(child.id().to_string());
                            source_ids.push(cause.grant_id.clone());
                        }
                    }
                }
                index += 1;
            }
            let mut revoked = Vec::new();
            for id in ids {
                revoked.push(agents.revoke_extension(&id, ExtensionKind::App, name)?);
                *epochs.entry((id, name.to_string())).or_default() += 1;
            }
            (revoked[0].clone(), revoked)
        };
        self.notify_app_grant_change();
        for agent in revoked {
            self.owned.session_snapshot(agent.session_id())?;
            self.finish_revoked_app_binding(agent, name, caller_user_id)
                .await?;
        }
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
        let mut permit = AppBindingPermit {
            actor: agent.clone(),
            target_id: target.id().into(),
            installation: installation_id.into(),
            target_revision: self.app_binding_revision(target.id(), installation_id),
            prompt_id: None,
            parent: None,
            provider_run_id: self
                .room_provider_origin
                .as_ref()
                .map(|(_, run)| run.clone()),
            prompt_delivery: None,
        };
        self.authorize_current_external_command()?;
        if self.room_agent_tools_enabled() {
            if agent.remote_execution().is_some()
                || target.remote_execution().is_some()
                || self.slice_kernel_id().is_some()
            {
                return Err(app_binding_error(
                    "leased agents use their Room Browser/Computer route",
                ));
            }
            if agent.id() != target.id() {
                crate::runtime::room_tool_admission::direct_child(&agent, &target)?;
                let parent = agent
                    .extension_grants()
                    .iter()
                    .find(|grant| grant.kind == ExtensionKind::App && grant.name == installation_id)
                    .filter(|grant| {
                        grant.app_grant.is_some()
                            && agent.has_extension_grant(ExtensionKind::App, installation_id)
                    })
                    .ok_or_else(|| {
                        app_binding_error("App transfer must name a live App bound to the caller")
                    })?;
                permit.parent = Some(parent.clone());
                return Ok(Some(permit));
            }
            // Trust is checked before requesting any owner authorization; publication
            // and invocation recheck it through the existing App admission path.
            self.owned
                .durable_state_store
                .check_app_binding(agent.owner_user_id(), installation_id)
                .map_err(|_| app_binding_error("App installation is not trusted and available"))?;
            permit.prompt_delivery = self.app_prompt_delivery(&agent);
            if let Some(run) = &permit.provider_run_id {
                if permit
                    .prompt_delivery
                    .as_ref()
                    .is_none_or(|delivery| !delivery.belongs_to(run))
                {
                    return Err(app_binding_error(
                        "App request's prompt was not delivered to this provider",
                    ));
                }
            }
            permit.prompt_id = Some(
                self.confirm_capability_request(
                    &agent,
                    &format!("App installation `{installation_id}`"),
                    || {
                        self.app_binding_revision(target.id(), installation_id)
                            == permit.target_revision
                            && self.app_prompt_delivery(&agent) == permit.prompt_delivery
                    },
                )
                .await?,
            );
            return Ok(Some(permit));
        }
        if target.has_extension_grant(ExtensionKind::App, installation_id)
            || !crate::session::effective_agent_user_authority(&session, Some(&agent))
                .requires_approval()
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
