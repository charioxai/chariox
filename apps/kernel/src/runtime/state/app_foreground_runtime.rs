//! Foreground Apps: the session's foreground App is the App whose view is
//! the Room's focused Tab. Opening an App's view binds it to the session's
//! focus agent through the same grant as an explicit selection or an agent's
//! self-grant, so the person can use it through that agent at once. A later
//! focus agent (a focus change or a person's spawn) is bound to the App of
//! the Tab the Room focuses then, unless the user revoked that binding (no
//! automatic regrant loop). With no App Tab in front, nothing is bound.
use super::KernelRuntimeState;
use crate::extension::{ExtensionGrant, ExtensionKind};

impl KernelRuntimeState {
    /// Binds the App of the view just opened on `target` to the focus agent.
    /// A direct user action, like an explicit grant: no separate approval,
    /// and it binds again a pair the user revoked.
    pub(crate) async fn foreground_app(&self, session_id: &str, target: &str) -> Option<String> {
        let view = self.app_control().views().binding_state(session_id, target)?.0;
        self.bind_app_to_focus_agent(session_id, &view.owner, &view.installation, true)
            .await
    }

    /// Binds the foreground App to the session's current focus agent, after a
    /// focus change or a person's spawn. A revoked pair is not bound again.
    /// Returns the bound agent.
    pub(crate) async fn bind_foreground_app(&self, session_id: &str) -> Option<String> {
        let (owner, installation) = self.room_foreground_app(session_id)?;
        self.bind_app_to_focus_agent(session_id, &owner, &installation, false)
            .await
    }

    /// The App whose open view is the Room's focused Tab, as (owner,
    /// installation). None when the Room focuses another page, the view was
    /// closed or unbound (uninstall), or the session has no Room.
    fn room_foreground_app(&self, session_id: &str) -> Option<(String, String)> {
        let focused = self
            .room_environment_snapshot(session_id)
            .ok()?
            .focused_tab_id?;
        let target = self
            .room_environment_controller_tab_binding(session_id, &focused)
            .ok()?
            .runtime_target_id;
        let view = self.app_control().views().binding_state(session_id, &target)?.0;
        Some((view.owner, view.installation))
    }

    /// Binds the App to the session's focus agent if the App's owner may bind
    /// it to that agent. `opened` (the App was just opened) binds even a pair
    /// the user revoked; a focus change does not.
    async fn bind_app_to_focus_agent(
        &self,
        session_id: &str,
        owner: &str,
        installation: &str,
        opened: bool,
    ) -> Option<String> {
        let views = self.app_control().views();
        let agent_id = self.focused_agent_id(session_id).await.ok().flatten()?;
        let agent = self.owned.agent_store.get_agent(&agent_id).ok()?;
        if agent.has_extension_grant(ExtensionKind::App, installation) {
            return Some(agent_id);
        }
        if !opened && views.is_revoked(session_id, &agent_id, installation) {
            return None;
        }
        match self
            .grant_agent_extension(&agent_id, ExtensionGrant::app(installation), owner)
            .await
        {
            Ok(_) => {
                views.set_revoked(session_id, &agent_id, installation, false);
                Some(agent_id)
            }
            Err(error) => {
                tracing::debug!(%error, "foreground App binding refused");
                None
            }
        }
    }

    /// A reinstall starts unbound: a revocation its uninstall could not finish
    /// is repeated before the owner is asked to approve it.
    pub(crate) async fn unbind_if_uninstalled(&self, owner: &str, installation: &str) {
        let store = self.owned.durable_state_store.clone();
        let (read_owner, read_installation) = (owner.to_owned(), installation.to_owned());
        let inactive = tokio::task::spawn_blocking(move || {
            store
                .get_app_installation(&read_owner, &read_installation)
                .map(|value| value.active.is_none())
        })
        .await;
        if matches!(inactive, Ok(Ok(true))) {
            self.unbind_uninstalled_app(owner, installation).await;
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
