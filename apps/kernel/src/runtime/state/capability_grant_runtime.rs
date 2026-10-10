//! MP-08/MP-10/MP-11 A05: user-requested capability grants. Prompt causation is
//! typed (human attachment); agents never mint authority or widen a transfer.
use super::kernel_browser_runtime::host_error;
use super::*;
use crate::error::UserDomainRefusalReason;
use crate::local::UserDomainResource;
use crate::runtime::room_tool_admission::{denied, direct_child, resolve_agent};
use crate::transport::runtime_tools::RuntimeToolResult;

/// MP-08/MP-11: the typed refusal clients and providers classify by code.
pub(super) fn refused(reason: UserDomainRefusalReason) -> DaemonError {
    DaemonError::UserDomainRefused { reason }
}

/// MP-11: serialize binding changes with pending acquisition and delegation.
/// Revisions are process-local; persisted grants carry their own generation.
#[derive(Default)]
pub(super) struct AppGrantEpochs {
    pub(super) revisions: std::sync::Mutex<BTreeMap<(String, String), u64>>,
    stopped: std::sync::atomic::AtomicBool,
    changed: Notify,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::extension::{ExtensionGrant, ExtensionKind};

    #[tokio::test]
    async fn capability_app_expiry_wakes_and_revokes_only_its_delegated_bindings() {
        let worktree = crate::test_support::TestWorktree::new("capability-app-expiry");
        let mut config = crate::config::DaemonConfig::for_tests();
        config.room_agent_tools = true;
        let mut app = DaemonApp::bootstrap(config).unwrap();
        crate::durable_state::app_state::fixture_catalog(&app.durable_state_store());
        let (session, parent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(worktree.session_request().with_owner_user_id("alice"))
            .unwrap();
        // A02: spawning a child requires the creator's live turn.
        crate::test_support::admit_room_test_turn(&mut app, session.id(), parent.id());
        let child = crate::app::KernelSessionService::new(&mut app)
            .spawn_agent(
                crate::agent::CreateAgentRequest::new(session.id(), "dev-stub")
                    .with_owner_user_id("alice")
                    .with_spawned_by_agent_id(parent.id()),
            )
            .unwrap();
        let peer = crate::app::KernelSessionService::new(&mut app)
            .spawn_agent(
                crate::agent::CreateAgentRequest::new(session.id(), "dev-stub")
                    .with_owner_user_id("alice"),
            )
            .unwrap();
        let router = crate::runtime::router::CommandRouter::with_interactive_capacity(
            Arc::new(Mutex::new(app)),
            4,
        );
        let state = router.runtime_state();
        let parent = state
            .grant_agent_extension(parent.id(), ExtensionGrant::app("installed"), "alice")
            .await
            .unwrap();
        state
            .grant_agent_extension(peer.id(), ExtensionGrant::app("installed"), "alice")
            .await
            .unwrap();
        let mut grant = parent
            .extension_grants()
            .iter()
            .find(|g| g.kind == ExtensionKind::App)
            .unwrap()
            .clone();
        let cause = grant.app_grant.as_mut().unwrap();
        let original = cause.clone();
        assert!(original.expires_at_ms <= crate::session::unix_epoch_ms() + 8 * 3_600_000);
        cause.expires_at_ms = crate::session::unix_epoch_ms() + 500;
        let short = cause.clone();
        state
            .owned
            .agent_store
            .grant_extension(parent.id(), grant)
            .unwrap();
        state.arm_app_grant_expiry(parent.id(), "installed", &short);
        let permit = state
            .authorize_agent_app_binding(session.id(), parent.id(), child.id(), "installed")
            .await
            .unwrap()
            .unwrap();
        let child = state
            .grant_agent_app_for_tool(
                child.id(),
                ExtensionGrant::app("installed"),
                "alice",
                Some(permit),
            )
            .await
            .unwrap();
        assert_eq!(
            child.extension_grants()[0]
                .app_grant
                .as_ref()
                .unwrap()
                .expires_at_ms,
            short.expires_at_ms
        );
        let unchanged = state
            .grant_agent_extension(parent.id(), ExtensionGrant::app("installed"), "alice")
            .await
            .unwrap();
        assert_eq!(
            unchanged.extension_grants()[0].app_grant.as_ref(),
            Some(&short),
            "MP-11: repeated binding does not renew or replace delegated authority"
        );
        tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                let parent = state.owned.agent_store.get_agent(parent.id()).unwrap();
                let child = state.owned.agent_store.get_agent(child.id()).unwrap();
                if parent.extension_grants().is_empty() && child.extension_grants().is_empty() {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("MP-11: the expiry wake must remove bindings without another call");
        assert!(state
            .owned
            .agent_store
            .get_agent(peer.id())
            .unwrap()
            .has_extension_grant(ExtensionKind::App, "installed"));
        // An old timer cannot remove a fresh owner grant.
        let fresh = state
            .grant_agent_extension(parent.id(), ExtensionGrant::app("installed"), "alice")
            .await
            .unwrap();
        state
            .revoke_expired_agent_app(parent.id(), "installed", &original.grant_id)
            .await
            .unwrap();
        assert!(state
            .owned
            .agent_store
            .get_agent(fresh.id())
            .unwrap()
            .has_extension_grant(ExtensionKind::App, "installed"));
        let peer = state
            .owned
            .agent_store
            .bind_remote_execution(
                peer.id(),
                crate::agent::RemoteAgentBinding {
                    worker_kernel_id: "worker".into(),
                    worker_machine_id: "worker-machine".into(),
                    execution_lease_id: "lease".into(),
                    leased_agent_id: "worker-agent".into(),
                    active_worker_provider_run_id: None,
                    relay_url: None,
                    relay_token: None,
                    relay_peer_protocol_version: Some(
                        crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION,
                    ),
                },
            )
            .unwrap();
        assert!(
            state
                .remote_extension_manifest_for_agent(&peer)
                .unwrap()
                .tools
                .iter()
                .all(|tool| tool.kind != ExtensionKind::App),
            "MP-11: the home must not advertise App tools to a worker"
        );
        state.shutdown_cleanup().await.unwrap();
        assert!(
            state
                .grant_agent_extension(parent.id(), ExtensionGrant::app("installed"), "alice")
                .await
                .is_err(),
            "MP-11: shutdown cannot admit authority without a live expiry wake"
        );
    }

    /// MP-11 (#922 review 5): recovery that drops a legacy untimed binding
    /// records the normal revoke events and audit.
    #[tokio::test]
    async fn capability_recovery_revoke_records_the_normal_revoke_events() {
        let worktree = crate::test_support::TestWorktree::new("capability-app-recovery");
        let mut config = crate::config::DaemonConfig::for_tests();
        config.room_agent_tools = true;
        let mut app = DaemonApp::bootstrap(config).unwrap();
        crate::durable_state::app_state::fixture_catalog(&app.durable_state_store());
        let (_, agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(worktree.session_request().with_owner_user_id("alice"))
            .unwrap();
        app.agents_mut()
            .grant_extension(agent.id(), ExtensionGrant::app("installed"))
            .unwrap();
        let router = crate::runtime::router::CommandRouter::with_interactive_capacity(
            Arc::new(Mutex::new(app)),
            4,
        );
        let state = router.runtime_state();
        assert!(!state
            .owned
            .agent_store
            .get_agent(agent.id())
            .unwrap()
            .has_extension_grant(ExtensionKind::App, "installed"));
        let audit = state
            .list_home_extension_audit_events(agent.id(), "alice", 20)
            .unwrap();
        assert!(audit.iter().any(|event| {
            event.kind == "home_extension.grant.revoked"
                && event.payload.pointer("/grant/name") == Some(&"installed".into())
        }));
        state.shutdown_cleanup().await.unwrap();
    }
}

pub(crate) struct AppBindingPermit {
    pub(super) actor: crate::agent::AgentInstance,
    pub(super) target_id: String,
    pub(super) installation: String,
    pub(super) target_revision: u64,
    pub(super) prompt_id: Option<String>,
    pub(super) parent: Option<crate::extension::ExtensionGrant>,
    pub(super) provider_run_id: Option<String>,
    pub(super) prompt_delivery: Option<AppPromptDelivery>,
}

/// MP-11 SB-03: process-local admission witness, never client-supplied authority.
#[derive(Clone, PartialEq, Eq)]
pub(super) struct AppPromptDelivery {
    prompt_id: String,
    phase: Option<crate::session::DurablePromptDeliveryPhase>,
    provider_run_id: Option<String>,
    owner_request: bool,
    source_user_id: Option<String>,
}

impl KernelRuntimeState {
    pub(super) fn app_prompt_delivery(
        &self,
        agent: &crate::agent::AgentInstance,
    ) -> Option<AppPromptDelivery> {
        let session = self
            .owned
            .session_store
            .get_session(agent.session_id())
            .ok()?;
        let prompt = self
            .owned
            .prompt_state_owner
            .active_prompt_for_agent_snapshot(&session, agent.id())?;
        (prompt.status() == crate::session::PromptStatus::Running)
            .then(|| AppPromptDelivery::from_prompt(&prompt))
    }
}

impl AppPromptDelivery {
    pub(super) fn from_prompt(prompt: &crate::session::PromptQueueItem) -> Self {
        Self {
            prompt_id: prompt.id().into(),
            phase: prompt.durable_delivery_phase(),
            provider_run_id: prompt
                .durable_delivery_provider_run_id()
                .map(str::to_string),
            owner_request: prompt.owner_request(),
            source_user_id: prompt.source_user_id().map(str::to_string),
        }
    }
    pub(super) fn belongs_to(&self, run: &str) -> bool {
        use crate::session::DurablePromptDeliveryPhase::{Accepted, Delivered, Dispatching};
        // Native/legacy turns may lack a delivery run ID. Preserve their exact
        // witness while separately fencing the authenticated current run.
        match self.phase {
            Some(Delivered) => self.provider_run_id.as_deref().is_none_or(|id| id == run),
            Some(Dispatching) => false,
            Some(Accepted) | None => self.provider_run_id.is_none(),
        }
    }
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct ShareArguments {
    agent: String,
    resources: Vec<UserDomainResource>,
}

impl KernelRuntimeState {
    /// MP-08/MP-11: prompts establish causation, not arbitrary resource scope.
    /// Only the owner's resource-specific interaction reply authorizes acquisition.
    pub(super) async fn confirm_capability_request(
        &self,
        agent: &crate::agent::AgentInstance,
        resource: &str,
        scope_live: impl Fn() -> bool + Send + Sync,
    ) -> Result<String, DaemonError> {
        let prompt = self
            .owner_requested_prompt(agent)
            .ok_or_else(|| refused(UserDomainRefusalReason::NotRequested))?;
        let interaction_id = format!("capability-request-{:032x}", rand::random::<u128>());
        let interaction = crate::session::RuntimeInteraction::for_kernel_operation(
            &interaction_id,
            &interaction_id,
            "Chariox resource access",
            format!(
                "Allow agent `{}` to use {resource} for this request?",
                agent.agent_ref()
            ),
            vec![
                crate::session::RuntimeInteractionChoice::new("allow", "Allow", "allow", None),
                crate::session::RuntimeInteractionChoice::new("deny", "Deny", "deny", None),
            ],
        );
        let receiver = self
            .create_kernel_operation_interaction(
                agent.session_id(),
                agent.owner_user_id(),
                interaction,
            )
            .await?;
        // MP-11: the owner decision itself also has a live expiry wake. A turn
        // disappearing while the owner is deciding withdraws the stale popup.
        let resolution = tokio::select! {
            result = receiver => result.map_err(|_| refused(UserDomainRefusalReason::NotGranted))?,
            _ = tokio::time::sleep(Duration::from_secs(300)) => {
                let _ = self.owned.timeout_runtime_interaction(agent.session_id(), &interaction_id);
                return Err(refused(UserDomainRefusalReason::NotGranted));
            }
            _ = async {
                while self.owner_requested_prompt(agent).as_deref() == Some(prompt.as_str())
                    && self.authorize_current_external_command().is_ok() && scope_live()
                {
                    tokio::time::sleep(Duration::from_millis(50)).await;
                }
            } => {
                let _ = self.owned.timeout_runtime_interaction(agent.session_id(), &interaction_id);
                return Err(refused(UserDomainRefusalReason::NotGranted));
            }
        };
        self.authorize_current_external_command()?;
        let current = self.owned.agent_store.get_agent(agent.id())?;
        if current.owner_user_id() != agent.owner_user_id()
            || current.session_id() != agent.session_id()
            || current.remote_execution().is_some()
            || self.owner_requested_prompt(&current).as_deref() != Some(prompt.as_str())
            || !scope_live()
        {
            return Err(refused(UserDomainRefusalReason::NotGranted));
        }
        if resolution.choice_id.as_deref() != Some("allow") {
            return Err(refused(UserDomainRefusalReason::NotRequested));
        }
        Ok(prompt)
    }

    pub(super) fn app_binding_revision(&self, agent: &str, installation: &str) -> u64 {
        *self
            .owned
            .app_grant_epochs
            .revisions
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(&(agent.into(), installation.into()))
            .unwrap_or(&0)
    }

    pub(super) async fn revoke_agent_app_capability_grants(
        &self,
        agent: &crate::agent::AgentInstance,
    ) -> Result<(), DaemonError> {
        if self.room_agent_tools_enabled() {
            for grant in agent.extension_grants() {
                if let Some(cause) = &grant.app_grant {
                    self.revoke_expired_agent_app(agent.id(), &grant.name, &cause.grant_id)
                        .await?;
                }
            }
        }
        Ok(())
    }

    pub(super) fn require_app_grant_wake(&self) -> Result<(), DaemonError> {
        if self.owned.app_grant_epochs.stopped.load(Ordering::Acquire)
            || tokio::runtime::Handle::try_current().is_err()
        {
            return Err(denied("App grant expiry requires a live executor"));
        }
        Ok(())
    }

    pub(super) fn stop_app_grant_wakes(&self) {
        self.owned
            .app_grant_epochs
            .stopped
            .store(true, Ordering::Release);
        self.owned.app_grant_epochs.changed.notify_waiters();
    }

    pub(super) fn recover_app_grant_expiries(&self) {
        for agent in self.owned.agent_store.list_agents() {
            for grant in agent.extension_grants() {
                if self.room_agent_tools_enabled()
                    && grant.kind == crate::extension::ExtensionKind::App
                    && grant.app_grant.is_none()
                {
                    // MP-11: legacy untimed bindings need fresh owner approval
                    // before they can become A05 authority.
                    self.revoke_recovered_app_binding(agent.id(), &grant.name);
                    continue;
                }
                if let Some(cause) = &grant.app_grant {
                    if tokio::runtime::Handle::try_current().is_ok() {
                        self.arm_app_grant_expiry(agent.id(), &grant.name, cause);
                    } else {
                        // MP-11: recovery without a live executor must not retain
                        // authority whose mandatory expiry wake cannot be armed.
                        self.revoke_recovered_app_binding(agent.id(), &grant.name);
                        tracing::warn!("App grant recovery refused without a live expiry executor");
                    }
                }
            }
        }
    }

    /// MP-11: recovery removes the binding at once and records the normal
    /// revoke; the async follow-up runs when an executor exists.
    fn revoke_recovered_app_binding(&self, agent: &str, name: &str) {
        let Ok(agent) = self.owned.agent_store.revoke_extension(
            agent,
            crate::extension::ExtensionKind::App,
            name,
        ) else {
            return;
        };
        let owner = agent.owner_user_id().to_string();
        if let Err(error) = self.record_revoked_app_binding(&agent, name, &owner) {
            tracing::warn!(%error, "App grant recovery revoke was not recorded");
        }
        if let Ok(handle) = tokio::runtime::Handle::try_current() {
            let state = self.clone();
            handle.spawn(async move {
                let _ = state.propagate_revoked_app_binding(agent, &owner).await;
            });
        }
    }

    /// MP-08/MP-11: absolute expiry always has a real wake, including after
    /// recovery. Mutation uses the captured grant ID so a replacement survives.
    pub(super) fn arm_app_grant_expiry(
        &self,
        agent: &str,
        installation: &str,
        cause: &crate::extension::AppCapabilityGrant,
    ) {
        let Ok(handle) = tokio::runtime::Handle::try_current() else {
            return;
        };
        let state = self.clone();
        let agent = agent.to_string();
        let installation = installation.to_string();
        let id = cause.grant_id.clone();
        let deadline = tokio::time::Instant::now()
            + Duration::from_millis(
                cause
                    .expires_at_ms
                    .saturating_sub(crate::session::unix_epoch_ms()),
            )
            .min(crate::runtime::user_domain_access::MAX_GRANT_LIFETIME);
        handle.spawn(async move {
            loop {
                let changed = state.owned.app_grant_epochs.changed.notified();
                tokio::pin!(changed);
                changed.as_mut().enable();
                if state.owned.app_grant_epochs.stopped.load(Ordering::Acquire)
                    || !state
                        .owned
                        .agent_store
                        .get_agent(&agent)
                        .is_ok_and(|agent| {
                            agent.extension_grants().iter().any(|grant| {
                                grant.name == installation
                                    && grant
                                        .app_grant
                                        .as_ref()
                                        .is_some_and(|cause| cause.grant_id == id)
                            })
                        })
                {
                    return;
                }
                tokio::select! {
                    _ = tokio::time::sleep_until(deadline) => {
                        let _ = state.revoke_expired_agent_app(&agent, &installation, &id).await;
                        return;
                    }
                    _ = &mut changed => {}
                }
            }
        });
    }

    pub(super) fn notify_app_grant_change(&self) {
        self.owned.app_grant_epochs.changed.notify_waiters();
    }

    /// MP-08/MP-11: the owner's prompt that is running this agent's turn, if any.
    /// Agent messages, workflows, schedules, leases and wakes are never owner requests.
    pub(crate) fn owner_requested_prompt(
        &self,
        agent: &crate::agent::AgentInstance,
    ) -> Option<String> {
        if !self.room_agent_tools_enabled() {
            return None;
        }
        let session = self.owned.session_snapshot(agent.session_id()).ok()?;
        let prompt = self
            .owned
            .prompt_state_owner
            .active_prompt_for_agent_snapshot(&session, agent.id())?;
        let owner = self.provider_account_authority_owner_user_id(agent.owner_user_id());
        (prompt.status() == crate::session::PromptStatus::Running
            && prompt.owner_request()
            && prompt
                .source_user_id()
                .is_some_and(|user| self.provider_account_authority_owner_user_id(user) == owner))
        .then(|| prompt.id().to_string())
    }

    /// MP-08/MP-11: hand a direct local child an explicit subset of the caller's
    /// granted user-domain resources. The child cannot widen or outlive it.
    pub(super) fn kernel_browser_share(
        &self,
        actor: &crate::agent::AgentInstance,
        arguments: serde_json::Value,
        admission: &crate::runtime::kernel_browser_host::KernelBrowserAdmission,
    ) -> Result<RuntimeToolResult, DaemonError> {
        if !self.room_agent_tools_enabled() {
            return Err(denied("grant transfer requires room agent tools"));
        }
        let request: ShareArguments = serde_json::from_value(arguments)
            .map_err(|_| host_error("MP-08: invalid grant transfer".into()))?;
        let agents = self
            .owned
            .agent_store
            .get_session_agents(actor.session_id());
        let child = resolve_agent(&agents, actor.session_id(), &request.agent)?;
        direct_child(actor, child)?;
        if child.remote_execution().is_some() || child.owner_user_id() != actor.owner_user_id() {
            return Err(denied(
                "grant transfer requires a direct child executing on this kernel; leased agents use their Room Browser/Computer route",
            ));
        }
        self.owned
            .kernel_browser_host
            .transfer_grant(admission, child.id(), &request.resources)
            .map_err(host_error)?;
        Ok(RuntimeToolResult {
            ok: true,
            payload: serde_json::json!({"shared": request.resources.len(), "agent_id": child.id()}),
        })
    }
}
