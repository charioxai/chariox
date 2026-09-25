//! Foreground Apps: opening an App's view binds it to the session's focus
//! agent through the same grant as an explicit selection or an agent's
//! self-grant, so the person can use it through that agent at once. The
//! session remembers its foreground App; a later focus agent is bound too.
use super::KernelRuntimeState;
use crate::extension::{ExtensionGrant, ExtensionKind};

impl KernelRuntimeState {
    /// Records the session's foreground App and binds the focus agent. A
    /// direct user action, like an explicit grant: no separate approval.
    pub(crate) async fn foreground_app(
        &self,
        session_id: &str,
        owner: &str,
        installation: &str,
    ) -> Option<String> {
        self.app_control()
            .views()
            .set_foreground(session_id, owner, installation);
        self.bind_foreground_app(session_id).await
    }

    /// Binds the session's foreground App to its current focus agent, if the
    /// App's owner is the one acting. Returns the bound agent.
    pub(crate) async fn bind_foreground_app(&self, session_id: &str) -> Option<String> {
        let (owner, installation) = self.app_control().views().foreground(session_id)?;
        let agent_id = self.focused_agent_id(session_id).await.ok().flatten()?;
        let agent = self.owned.agent_store.get_agent(&agent_id).ok()?;
        if agent.has_extension_grant(ExtensionKind::App, &installation) {
            return Some(agent_id);
        }
        match self
            .grant_agent_extension(&agent_id, ExtensionGrant::app(installation), &owner)
            .await
        {
            Ok(_) => Some(agent_id),
            Err(error) => {
                tracing::debug!(%error, "foreground App binding refused");
                None
            }
        }
    }

    /// An uninstalled App is unbound from every agent, which refreshes their
    /// tool catalogs; workflow copies follow the existing grant path.
    pub(crate) async fn unbind_uninstalled_app(&self, owner: &str, installation: &str) {
        self.app_control()
            .views()
            .forget_installation(owner, installation);
        for agent in self.owned.agent_store.list_agents() {
            if agent.has_extension_grant(ExtensionKind::App, installation) {
                if let Err(error) = self
                    .revoke_agent_extension(agent.id(), ExtensionKind::App, installation, owner)
                    .await
                {
                    tracing::debug!(%error, "uninstalled App binding not revoked");
                }
            }
        }
    }
}
