//! MP-08/MP-11: wiring to existing agent lifecycle and wake ownership.
use super::kernel_browser_runtime::host_error;
use super::*;

impl KernelRuntimeState {
    /// Reconciling a holder only reads its session. The projection already holds the
    /// published state, so rebuilding and republishing it per holder on every
    /// transport wake would cost a full snapshot per holder.
    fn grant_holder_session(
        &self,
        session_id: &str,
    ) -> Result<std::sync::Arc<crate::session::RuntimeSession>, DaemonError> {
        self.owned.durable_state_store.require_writer_healthy()?;
        match self.owned.session_projection.get_shared(session_id) {
            Some(session) => Ok(session),
            None => self
                .owned
                .session_snapshot(session_id)
                .map(std::sync::Arc::new),
        }
    }

    /// Forgets the agents whose agent, Room or owner is gone, or that now execute
    /// elsewhere: their in-flight browser calls end with them.
    pub(crate) fn refresh_user_domain_grants(&self) {
        for (owner, id) in self.owned.kernel_browser_host.running_agents() {
            let Ok(agent) = self.owned.agent_store.get_agent(&id) else {
                self.owned.kernel_browser_host.revoke_agent(&id);
                continue;
            };
            let Ok(session) = self.grant_holder_session(agent.session_id()) else {
                self.owned.kernel_browser_host.revoke_agent(&id);
                continue;
            };
            // Remote agents use their Room route; only an explicit typed
            // leased desktop route keeps a HOME presence (MP-08/MP-11).
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
            }
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
        Ok(agent)
    }
    /// MP-08/MP-11: the run's agent executes here, in its live room.
    pub(super) fn user_domain_agent_placement(
        &self,
        run: &crate::provider::RuntimeProviderRun,
    ) -> Result<crate::agent::AgentInstance, DaemonError> {
        if run.state() == crate::provider::ProviderRunState::Ended {
            return Err(host_error("MP-11: the provider run ended".into()));
        }
        let id = run
            .agent_instance_id()
            .ok_or_else(|| host_error("MP-08: admitted provider agent required".into()))?;
        let agent = self.owned.agent_store.get_agent(id)?;
        // The agent's current run is never an ended one; a run a hot reload handed over counts
        // until a recovered prompt claims it.
        if self
            .owned
            .provider_store
            .get_tool_run_for_agent(run.session_id(), id)
            .is_none_or(|current| current.id() != run.id())
        {
            return Err(host_error(
                "MP-11: the provider run was replaced or ended".into(),
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
        // Membership and status are stored on the Room; a full snapshot would also read its
        // tasks, wakes and queued messages and republish it, for every listed run.
        self.owned.durable_state_store.require_writer_healthy()?;
        let session = self.owned.session_store.get_session(run.session_id())?;
        if agent.session_id() != run.session_id()
            || !self.is_room_member(&session, agent.owner_user_id())
            || session.status() == crate::session::SessionStatus::Ended
        {
            return Err(host_error(
                "MP-08: the agent is not in this live Room".into(),
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
                return Err(host_error(format!("MP-08: this agent executes on kernel {execution}; the user-domain window is on kernel {}. Cross-kernel control is unavailable.", context.home_kernel_id)));
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
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::Instant;

    use tokio::sync::Mutex;

    use crate::runtime::router::CommandRouter;
    use crate::runtime::state::session::call_counts;
    use crate::session::CreateSessionRequest;
    use crate::{DaemonApp, DaemonConfig};

    struct GrantHolders {
        runtime: crate::runtime::state::KernelRuntimeState,
        agents: Vec<crate::agent::AgentInstance>,
        session_ids: Vec<String>,
        root: std::path::PathBuf,
    }

    impl Drop for GrantHolders {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }

    /// `sessions` sessions, each with a running provider, `extra_agents` more agents
    /// and its first agent holding the browser focus grant.
    fn grant_holders(sessions: usize, extra_agents: usize) -> GrantHolders {
        let root = std::env::temp_dir().join(format!(
            "user-domain-grants-{}-{}",
            std::process::id(),
            crate::session::unix_epoch_ms()
        ));
        std::fs::create_dir_all(&root).expect("workspace");
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon bootstrap");
        let mut agents = Vec::new();
        let mut session_ids = Vec::new();
        for _ in 0..sessions {
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
            for _ in 0..extra_agents {
                crate::app::KernelSessionService::new(&mut app)
                    .spawn_agent(crate::agent::CreateAgentRequest::new(
                        session.id(),
                        "dev-stub",
                    ))
                    .expect("extra agent");
            }
            session_ids.push(session.id().to_string());
            agents.push(agent);
        }
        let runtime =
            CommandRouter::with_interactive_capacity(Arc::new(Mutex::new(app)), 8).runtime_state();
        for agent in &agents {
            let owner = runtime.provider_account_authority_owner_user_id(agent.owner_user_id());
            runtime
                .owned
                .kernel_browser_host
                .admit(&owner, agent.id())
                .expect("presence");
        }
        assert_eq!(
            runtime.owned.kernel_browser_host.running_agents().len(),
            sessions
        );
        GrantHolders {
            runtime,
            agents,
            session_ids,
            root,
        }
    }

    fn count(counter: &'static std::thread::LocalKey<std::cell::Cell<usize>>) -> usize {
        counter.with(std::cell::Cell::get)
    }

    fn reset_counts() {
        call_counts::SNAPSHOT_BUILDS.with(|count| count.set(0));
        call_counts::RUNTIME_VIEW_PROJECTIONS.with(|count| count.set(0));
    }

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
        let holders = grant_holders(SESSIONS, 6);
        let runtime = &holders.runtime;
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
        for agent in &holders.agents {
            runtime.owned.kernel_browser_host.revoke_agent(agent.id());
        }
        assert!(runtime
            .owned
            .kernel_browser_host
            .running_agents()
            .is_empty());
        let without_holders = time_signatures();

        assert!(
            with_holders < without_holders * 2,
            "signatures for {SESSIONS} providers took {with_holders:?} with {SESSIONS} grant holders and {without_holders:?} with none"
        );
    }

    // The pump reconciles every holder on each change wake. A holder only needs the
    // session state the kernel already published, not a rebuilt and republished one.
    #[tokio::test]
    async fn grant_reconciliation_reads_published_sessions_without_rebuilding_them() {
        crate::test_support::isolated_env_test!();
        const SESSIONS: usize = 12;
        let holders = grant_holders(SESSIONS, 2);
        let runtime = &holders.runtime;
        for session_id in &holders.session_ids {
            runtime
                .owned
                .session_snapshot(session_id)
                .expect("publish session");
        }

        reset_counts();
        runtime.refresh_user_domain_grants();

        assert_eq!(
            count(&call_counts::SNAPSHOT_BUILDS),
            0,
            "reconciling {SESSIONS} holders rebuilt published sessions"
        );
        assert_eq!(
            runtime.owned.kernel_browser_host.running_agents().len(),
            SESSIONS
        );
    }

    // A holder whose session was never published is still reconciled from the
    // stored session, and one whose session is gone loses its grant.
    #[tokio::test]
    async fn grant_reconciliation_revokes_holders_of_removed_sessions() {
        crate::test_support::isolated_env_test!();
        let holders = grant_holders(2, 0);
        let runtime = &holders.runtime;
        runtime.refresh_user_domain_grants();
        assert_eq!(runtime.owned.kernel_browser_host.running_agents().len(), 2);

        runtime
            .owned
            .session_projection
            .remove(&holders.session_ids[0]);
        runtime
            .owned
            .session_store
            .delete_session(&holders.session_ids[0])
            .expect("remove session");
        runtime.refresh_user_domain_grants();

        let remaining = runtime.owned.kernel_browser_host.running_agents();
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0].1, holders.agents[1].id());
    }

    // A snapshot is built from the stores and projected once; publishing it must
    // not project it again.
    #[tokio::test]
    async fn session_snapshot_projects_the_runtime_view_once() {
        crate::test_support::isolated_env_test!();
        let holders = grant_holders(1, 4);
        let runtime = &holders.runtime;

        reset_counts();
        runtime
            .owned
            .session_snapshot(&holders.session_ids[0])
            .expect("snapshot");

        assert_eq!(count(&call_counts::SNAPSHOT_BUILDS), 1);
        assert_eq!(count(&call_counts::RUNTIME_VIEW_PROJECTIONS), 1);
    }
}
