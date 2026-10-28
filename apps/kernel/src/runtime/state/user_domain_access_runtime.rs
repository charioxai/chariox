//! MP-08/MP-11: wiring to existing agent lifecycle and wake ownership.
use super::kernel_browser_runtime::host_error;
use super::*;

impl KernelRuntimeState {
    pub(crate) fn refresh_user_domain_grants(&self) {
        let turns = self.owned.active_turns.snapshot();
        for (owner, id) in self.owned.kernel_browser_host.grant_holders() {
            let Ok(agent) = self.owned.agent_store.get_agent(&id) else {
                self.owned.kernel_browser_host.revoke_agent(&id);
                continue;
            };
            let Ok(session) = self.owned.session_snapshot(agent.session_id()) else {
                self.owned.kernel_browser_host.revoke_agent(&id);
                continue;
            };
            // Remote agents use their Room route; only an explicit typed
            // leased desktop grant keeps a HOME grant (MP-08/MP-11).
            let leased_desktop = self
                .owned
                .leased_computer_grants
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .contains_key(&id);
            if (agent.remote_execution().is_some() && !leased_desktop)
                || session.status() == crate::session::SessionStatus::Ended
                || self.provider_account_authority_owner_user_id(agent.owner_user_id()) != owner
            {
                self.owned.kernel_browser_host.revoke_agent(&id);
                continue;
            }
            // An admitted provider turn includes harness-owned subprocess/subagent/tool
            // waits. Scheduled prompts and kernel continuations remain wake ownership
            // after a provider yields; a live idle provider process is not work.
            let busy = turns
                .values()
                .any(|turn| turn.agent_id == id && turn.session_id == session.id())
                || self
                    .owned
                    .prompt_state_owner
                    .active_prompt_for_agent_snapshot(&session, &id)
                    .is_some()
                || session
                    .queued_prompts()
                    .iter()
                    .any(|prompt| prompt.target_agent_id() == id)
                || session
                    .agent_prompt_schedules()
                    .iter()
                    .any(|schedule| schedule.agent_id() == id)
                || session
                    .active_interactions()
                    .iter()
                    .any(|interaction| interaction.agent_id() == Some(id.as_str()))
                || self
                    .owned
                    .pending_mcp_continuations
                    .write()
                    .values()
                    .any(|pending| pending.agent_id == id && pending.session_id == session.id())
                || self
                    .owned
                    .pending_provider_reloads
                    .write()
                    .values()
                    .any(|pending| pending.agent_id == id && pending.session_id == session.id());
            self.owned
                .kernel_browser_host
                .bind_activity(&owner, &id, session.id(), busy);
        }
    }
    pub(super) fn user_domain_agent(
        &self,
        run: &crate::provider::RuntimeProviderRun,
    ) -> Result<crate::agent::AgentInstance, DaemonError> {
        self.refresh_user_domain_grants();
        self.user_domain_agent_authority(run)
    }
    // Cancellation checks can run while a browser actor is locked. They read
    // authority only; lifecycle reconciliation must not acquire that actor again.
    pub(super) fn user_domain_agent_authority(
        &self,
        run: &crate::provider::RuntimeProviderRun,
    ) -> Result<crate::agent::AgentInstance, DaemonError> {
        let agent = self.user_domain_agent_placement(run)?;
        if agent.archived() {
            return Err(host_error(
                "Unarchive this agent before using the browser".into(),
            ));
        }
        let owner = self.provider_account_authority_owner_user_id(agent.owner_user_id());
        if !self.owned.kernel_browser_host.has_grant(&owner, agent.id()) {
            return Err(host_error(
                "MP-08: not_granted: user-domain access expired or revoked; ask the user to focus this agent"
                    .into(),
            ));
        }
        Ok(agent)
    }
    /// MP-08/MP-11: the run's agent executes here, in its live room. Grant
    /// presence is checked separately so an owner-requested turn can acquire one.
    pub(super) fn user_domain_agent_placement(
        &self,
        run: &crate::provider::RuntimeProviderRun,
    ) -> Result<crate::agent::AgentInstance, DaemonError> {
        if run.state() == crate::provider::ProviderRunState::Ended {
            return Err(host_error(
                "MP-11: not_granted: provider run ended; user-domain authority revoked".into(),
            ));
        }
        let id = run
            .agent_instance_id()
            .ok_or_else(|| host_error("MP-08: admitted provider agent required".into()))?;
        let agent = self.owned.agent_store.get_agent(id)?;
        if self.owned.provider_store.get_run(run.id())?.state()
            == crate::provider::ProviderRunState::Ended
            || self
                .owned
                .provider_store
                .get_run_for_agent(run.session_id(), id)
                .is_none_or(|current| current.id() != run.id())
        {
            return Err(host_error(
                "MP-11: not_granted: provider run was replaced or ended".into(),
            ));
        }

        if agent.remote_execution().is_some()
            || self.slice_kernel_id().is_some()
            || self
                .owned
                .provider_run_projection
                .is_leased_provider_run(run.id())
        {
            return Err(host_error("MP-08: leased or remote agents use their Room Browser/Computer route; user-domain control is unavailable".into()));
        }
        let session = self.owned.session_snapshot(run.session_id())?;
        if agent.session_id() != run.session_id()
            || !session.has_member(agent.owner_user_id())
            || session.status() == crate::session::SessionStatus::Ended
        {
            return Err(host_error(
                "MP-08: not_granted: user-domain access expired or revoked; ask the user to focus this agent"
                    .into(),
            ));
        }
        Ok(agent)
    }
    pub(super) async fn user_domain_tool_agent(
        &self,
        run: &crate::provider::RuntimeProviderRun,
    ) -> Result<crate::agent::AgentInstance, DaemonError> {
        self.refuse_leased_user_domain_run(run).await?;
        self.user_domain_agent(run)
    }
    /// MP-08: leased agents use only their Room Browser/Computer route.
    pub(super) async fn refuse_leased_user_domain_run(
        &self,
        run: &crate::provider::RuntimeProviderRun,
    ) -> Result<(), DaemonError> {
        if self
            .owned
            .provider_run_projection
            .is_leased_provider_run(run.id())
        {
            let context = self
                .with_app_side_effect(|app| {
                    crate::app::RemoteLeaseRuntime::new(app)
                        .leased_extension_invocation_context_for_runtime_provider_run(run)
                })
                .await;
            if let Some(context) = context {
                let execution = self.owned.config_projection.snapshot().daemon_id;
                return Err(host_error(format!("MP-08: this agent executes on kernel {execution}; the user-domain window is on kernel {}. Ask the user to focus an agent on the window's kernel {}; cross-kernel control is unavailable.", context.home_kernel_id, context.home_kernel_id)));
            }
        }
        Ok(())
    }
    pub(super) fn user_domain_owner_aliases(&self, owner: &str) -> Vec<String> {
        let canonical = self.provider_account_authority_owner_user_id(owner);
        let mut aliases = vec![canonical.clone()];
        if canonical == crate::session::DEFAULT_LOCAL_USER_ID {
            if let Some(profile) = self.owned.config_projection.snapshot().cloud_relay {
                if !profile.user_id.is_empty() && profile.user_id != canonical {
                    aliases.push(profile.user_id);
                }
            }
        }
        aliases
    }
    pub(super) fn user_domain_window_projection(&self, owner: &str) -> serde_json::Value {
        let owner = self.provider_account_authority_owner_user_id(owner);
        let kernel = self.owned.config_projection.snapshot().daemon_id;
        let focused = self
            .owned
            .kernel_browser_host
            .focused_agent(&owner)
            .and_then(|id| self.owned.agent_store.get_agent(&id).ok());
        let focused_kernel = focused.as_ref().map(|agent| {
            agent
                .remote_execution()
                .map(|binding| binding.worker_kernel_id.clone())
                .unwrap_or_else(|| kernel.clone())
        });
        serde_json::json!({"kernel_id":kernel,"kernel_name":kernel,"focused_agent_kernel_id":focused_kernel,"reachable_by_focused_agent":focused_kernel.as_deref()==Some(kernel.as_str())})
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::Instant;

    use tokio::sync::Mutex;

    use crate::runtime::router::CommandRouter;
    use crate::session::CreateSessionRequest;
    use crate::{DaemonApp, DaemonConfig};

    // Detecting a catalog change visits every running provider. With one
    // focused agent per attached session, each visit must not reconcile every
    // grant holder again: on a busy kernel that made an idle monitor spend one
    // full session snapshot per (provider, holder) pair and saturate a core.
    // Listing the catalogs of 24 providers while 24 agents hold grants must cost
    // about the same as listing them with no holder.
    #[tokio::test]
    async fn catalog_signatures_do_not_reconcile_every_grant_holder_per_provider() {
        crate::test_support::isolated_env_test!();
        const SESSIONS: usize = 24;
        let root = std::env::temp_dir().join(format!(
            "catalog-signature-grants-{}-{}",
            std::process::id(),
            crate::session::unix_epoch_ms()
        ));
        std::fs::create_dir_all(&root).expect("workspace");
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon bootstrap");
        let mut agents = Vec::new();
        for _ in 0..SESSIONS {
            let (session, agent) = crate::app::KernelSessionService::new(&mut app)
                .create_session(CreateSessionRequest::new(
                    root.to_string_lossy(),
                    root.to_string_lossy(),
                ))
                .expect("session");
            app.launch_provider(
                crate::provider::LaunchProviderRequest::new(
                    session.id(),
                    "dev-stub",
                    "dev-stub",
                    "default",
                    "default",
                )
                .with_agent_id(agent.id()),
            )
            .expect("provider run");
            for _ in 0..6 {
                crate::app::KernelSessionService::new(&mut app)
                    .spawn_agent(crate::agent::CreateAgentRequest::new(
                        session.id(),
                        "dev-stub",
                    ))
                    .expect("extra agent");
            }
            agents.push(agent);
        }
        let runtime =
            CommandRouter::with_interactive_capacity(Arc::new(Mutex::new(app)), 8).runtime_state();
        for agent in &agents {
            let owner = runtime.provider_account_authority_owner_user_id(agent.owner_user_id());
            runtime
                .owned
                .kernel_browser_host
                .set_focus(&owner, Some(agent.id()));
        }
        assert_eq!(
            runtime.owned.kernel_browser_host.grant_holders().len(),
            SESSIONS
        );
        let tokens = runtime.runtime_tool_catalog_auth_tokens();
        assert_eq!(tokens.len(), SESSIONS);

        let time_signatures = || {
            (0..3)
                .map(|_| {
                    let started = Instant::now();
                    for token in &tokens {
                        runtime.runtime_catalog_signature(token);
                    }
                    started.elapsed()
                })
                .min()
                .expect("timed signatures")
        };
        let with_holders = time_signatures();
        for agent in &agents {
            runtime.owned.kernel_browser_host.revoke_agent(agent.id());
        }
        assert!(runtime.owned.kernel_browser_host.grant_holders().is_empty());
        let without_holders = time_signatures();

        assert!(
            with_holders < without_holders * 2,
            "signatures for {SESSIONS} providers took {with_holders:?} with {SESSIONS} grant holders and {without_holders:?} with none"
        );
        let _ = std::fs::remove_dir_all(&root);
    }
}
