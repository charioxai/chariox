//! MP-08/MP-11: a view call of an agent proceeds only while that agent still
//! holds the App in the Room it was admitted in.
use crate::{
    agent::{AgentInstance, AgentServiceStore},
    extension::ExtensionKind,
    runtime::browser_controller_app_view::BrowserAppViewError,
};

#[derive(Clone)]
pub(crate) struct AppViewAuthority {
    agent: Option<(AgentServiceStore, String, String, String)>,
}
impl AppViewAuthority {
    pub(crate) fn new(
        agents: AgentServiceStore,
        agent: Option<&AgentInstance>,
        installation: &str,
    ) -> Self {
        Self {
            agent: agent.map(|agent| {
                (
                    agents,
                    agent.id().to_owned(),
                    agent.session_id().to_owned(),
                    installation.to_owned(),
                )
            }),
        }
    }
    pub(crate) fn check(&self) -> Result<(), BrowserAppViewError> {
        self.commit(|| Ok(()))
    }
    pub(crate) fn enqueue<T>(
        &self,
        call: impl FnOnce() -> Result<T, BrowserAppViewError>,
    ) -> Result<T, BrowserAppViewError> {
        self.commit(call)
    }
    /// The agent store stays read-locked through the enqueue/reply commit, so
    /// an archive or an unbind cannot slip between the check and the commit.
    pub(crate) fn commit<T>(
        &self,
        call: impl FnOnce() -> Result<T, BrowserAppViewError>,
    ) -> Result<T, BrowserAppViewError> {
        let Some((store, id, session, installation)) = &self.agent else {
            return call();
        };
        let agents = store.read();
        if !agents.get_agent(id).is_ok_and(|current| {
            !current.archived()
                && current.session_id() == session
                && current.has_extension_grant(ExtensionKind::App, installation)
        }) {
            return Err(not_granted());
        }
        call()
    }
}
pub(crate) fn not_granted() -> BrowserAppViewError {
    BrowserAppViewError {
        code: "AGENT_APP_NOT_GRANTED".into(),
        message:
            "This agent has not asked for this App yet; request it with chariox.request_extension"
                .into(),
    }
}
