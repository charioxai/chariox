//! MP-08/MP-11: a view call retains the agent's installation and AppView grants.
use crate::{
    agent::{AgentInstance, AgentService, AgentServiceStore},
    extension::ExtensionKind,
    runtime::{
        browser_controller_app_view::BrowserAppViewError,
        kernel_browser_host::KernelBrowserResourceGrant,
    },
};
use std::sync::Arc;

type AgentGrantCheck = Arc<dyn Fn(&AgentService) -> bool + Send + Sync>;
#[derive(Clone)]
pub(crate) struct AppViewAuthority {
    agent: Option<(AgentServiceStore, AgentGrantCheck)>,
    browser: Option<KernelBrowserResourceGrant>,
}
impl AppViewAuthority {
    pub(crate) fn new(
        agents: AgentServiceStore,
        agent: Option<&AgentInstance>,
        installation: &str,
        browser: Option<KernelBrowserResourceGrant>,
    ) -> Self {
        let agent = agent.map(|agent| {
            let (id, session, owner, installation) = (
                agent.id().to_owned(),
                agent.session_id().to_owned(),
                agent.owner_user_id().to_owned(),
                installation.to_owned(),
            );
            let check: AgentGrantCheck = Arc::new(move |agents| {
                agents.get_agent(&id).is_ok_and(|current| {
                    current.session_id() == session
                        && current.owner_user_id() == owner
                        && current.has_extension_grant(ExtensionKind::App, &installation)
                })
            });
            (agents, check)
        });
        Self { agent, browser }
    }
    pub(crate) fn check(&self) -> Result<(), BrowserAppViewError> {
        self.commit(|| Ok(()))
    }
    /// Both authority locks remain held through the enqueue/reply commit. The
    /// durable queue budget reads only the retained atomic epoch and deadline.
    pub(crate) fn commit<T>(
        &self,
        call: impl FnOnce() -> Result<T, BrowserAppViewError>,
    ) -> Result<T, BrowserAppViewError> {
        if self.browser.as_ref().is_some_and(|grant| grant.cancelled()) {
            return Err(not_granted());
        }
        self.with_agent(|| match &self.browser {
            Some(grant) => grant.commit(call).map_err(|_| not_granted())?,
            None => call(),
        })
    }
    /// Installation authority spans the writer wait. Browser authority commits
    /// inside the retained operation budget at its actual enqueue boundary.
    pub(crate) fn enqueue<T>(
        &self,
        call: impl FnOnce() -> Result<T, BrowserAppViewError>,
    ) -> Result<T, BrowserAppViewError> {
        if self.browser.as_ref().is_some_and(|grant| grant.cancelled()) {
            return Err(not_granted());
        }
        self.with_agent(call)
    }
    fn with_agent<T>(
        &self,
        call: impl FnOnce() -> Result<T, BrowserAppViewError>,
    ) -> Result<T, BrowserAppViewError> {
        let agents = self.agent.as_ref().map(|(store, _)| store.read());
        if let (Some(agents), Some((_, check))) = (&agents, &self.agent) {
            if !check(agents) {
                return Err(not_granted());
            }
        }
        call()
    }

    /// Browser revocation wakes cold acquisition and a pending worker reply.
    /// Poll only the lock-free epoch: an enqueue may hold the authority locks.
    pub(crate) async fn cancelled(&self) {
        let Some(grant) = &self.browser else {
            std::future::pending::<()>().await;
            return;
        };
        while !grant.cancelled() {
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        }
    }
}
pub(crate) fn not_granted() -> BrowserAppViewError {
    BrowserAppViewError {
        code: "AGENT_APP_NOT_GRANTED".into(),
        message: "This agent has no grant for this App; the owner can grant it the App".into(),
    }
}
