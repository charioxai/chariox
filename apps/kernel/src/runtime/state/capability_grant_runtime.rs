//! App binding revisions: a binding change and a pending acquisition serialize on them.
use super::*;

/// MP-11: serialize binding changes with pending acquisition.
/// Revisions are process-local; persisted grants carry their own generation.
#[derive(Default)]
pub(super) struct AppGrantEpochs {
    pub(super) revisions:
        std::sync::Mutex<BTreeMap<(String, String), Arc<std::sync::atomic::AtomicU64>>>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::extension::{ExtensionGrant, ExtensionKind};

    // MP-08/MP-10/MP-11 A06: a still-live binding answers at once in Room mode.
    #[tokio::test]
    async fn room_self_binding_of_a_live_app_binding_asks_nothing_and_keeps_it() {
        let worktree = crate::test_support::TestWorktree::new("room-live-binding");
        let mut config = crate::config::DaemonConfig::for_tests();
        config.room_agent_tools = true;
        let mut app = DaemonApp::bootstrap(config).unwrap();
        crate::durable_state::app_state::fixture_catalog(&app.durable_state_store());
        let (session, agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(worktree.session_request().with_owner_user_id("alice"))
            .unwrap();
        let router = crate::runtime::router::CommandRouter::with_interactive_capacity(
            Arc::new(Mutex::new(app)),
            4,
        );
        let state = router.runtime_state();
        let bound = state
            .grant_agent_extension(agent.id(), ExtensionGrant::app("installed"), "alice")
            .await
            .unwrap();
        let permit = tokio::time::timeout(
            Duration::from_secs(2),
            state.authorize_agent_app_binding(session.id(), agent.id(), agent.id(), "installed"),
        )
        .await
        .expect("a live binding must not wait for a decision")
        .unwrap();
        assert!(permit.is_some());
        let kept = state
            .grant_agent_app_for_tool(
                agent.id(),
                ExtensionGrant::app("installed"),
                "alice",
                permit,
            )
            .await
            .unwrap();
        assert_eq!(kept.extension_grants(), bound.extension_grants());
    }

    /// An agent binds the owner's trusted App to itself with no owner prompt, no
    /// sudo and no card, as it uses the kernel browser.
    #[tokio::test]
    async fn room_self_binding_of_a_trusted_app_asks_nothing() {
        let worktree = crate::test_support::TestWorktree::new("room-app-no-card");
        let mut config = crate::config::DaemonConfig::for_tests();
        config.room_agent_tools = true;
        let mut app = DaemonApp::bootstrap(config).unwrap();
        crate::durable_state::app_state::fixture_catalog(&app.durable_state_store());
        let (session, agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(worktree.session_request().with_owner_user_id("alice"))
            .unwrap();
        let router = crate::runtime::router::CommandRouter::with_interactive_capacity(
            Arc::new(Mutex::new(app)),
            4,
        );
        let state = router.runtime_state();
        let permit = tokio::time::timeout(
            Duration::from_secs(2),
            state.authorize_agent_app_binding(session.id(), agent.id(), agent.id(), "installed"),
        )
        .await
        .expect("binding a trusted App must not wait for a decision")
        .expect("an agent may bind the owner's trusted App");
        let bound = state
            .grant_agent_app_for_tool(
                agent.id(),
                ExtensionGrant::app("installed"),
                "alice",
                permit,
            )
            .await
            .unwrap();
        assert!(bound.has_extension_grant(ExtensionKind::App, "installed"));
        assert!(state
            .owned
            .session_store
            .get_session(session.id())
            .unwrap()
            .active_interactions()
            .is_empty());
    }
}

pub(crate) struct AppBindingPermit {
    pub(super) actor: crate::agent::AgentInstance,
    pub(super) target_id: String,
    pub(super) target_binding: Option<crate::agent::RemoteAgentBinding>,
    pub(super) installation: String,
    pub(super) target_revision: u64,
    pub(super) provider_run_id: Option<String>,
}

impl KernelRuntimeState {
    pub(super) fn app_binding_epoch(
        &self,
        agent: &str,
        installation: &str,
    ) -> Arc<std::sync::atomic::AtomicU64> {
        self.owned
            .app_grant_epochs
            .revisions
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .entry((agent.into(), installation.into()))
            .or_default()
            .clone()
    }

    pub(super) fn app_binding_revision(&self, agent: &str, installation: &str) -> u64 {
        self.app_binding_epoch(agent, installation)
            .load(Ordering::Acquire)
    }
}
