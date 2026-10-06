//! MP-08 / MP-10 / MP-11: home-owned Computer denial, independent of membership.
use super::*;
impl KernelRuntimeState {
    pub(super) fn require_room_computer_access(&self, agent_id: &str) -> Result<(), DaemonError> {
        if self
            .owned
            .room_computer_revoked
            .lock()
            .map_err(|_| access_error())?
            .contains(agent_id)
        {
            Err(access_error())
        } else {
            Ok(())
        }
    }
    pub(super) fn set_room_computer_access(
        &self,
        user: &str,
        reference: Option<&str>,
        allowed: bool,
    ) -> Result<(), DaemonError> {
        let aliases = self.user_domain_owner_aliases(user);
        let agents = self.owned.agent_store.list_agents();
        let mut revoked = self
            .owned
            .room_computer_revoked
            .lock()
            .map_err(|_| access_error())?;
        let mut found = reference.is_none();
        let mut changed = false;
        for agent in agents {
            if !aliases.iter().any(|owner| owner == agent.owner_user_id()) {
                continue;
            }
            if reference.is_some_and(|reference| {
                reference != agent.id() && Some(reference) != agent.alias()
            }) {
                continue;
            }
            found = true;
            if allowed {
                changed |= revoked.remove(agent.id());
            } else {
                changed |= revoked.insert(agent.id().into());
            }
        }
        if !found && allowed {
            return Err(access_error());
        }
        if changed {
            self.owned
                .kernel_browser_host
                .access_projection_changed(user);
        }
        Ok(())
    }
    // Same lock order as mutation: Room permissions, then the shared cursor.
    pub(super) fn room_computer_grant_snapshot(
        &self,
        user: &str,
        kernel: &str,
    ) -> Result<serde_json::Value, DaemonError> {
        let aliases = self.user_domain_owner_aliases(user);
        let agents = self.owned.agent_store.list_agents();
        let revoked = self
            .owned
            .room_computer_revoked
            .lock()
            .map_err(|_| access_error())?;
        let mut snapshot = self.owned.kernel_browser_host.grant_snapshot(user, kernel);
        snapshot["room_computer"] = serde_json::json!(agents.iter().filter(|agent| aliases.iter().any(|owner| owner == agent.owner_user_id())).map(|agent| serde_json::json!({"agent_id":agent.id(),"session_id":agent.session_id(),"allowed":!revoked.contains(agent.id())})).collect::<Vec<_>>());
        Ok(snapshot)
    }
    pub(super) fn require_room_computer_actor(&self, actor: &str) -> Result<(), DaemonError> {
        if let Some(agent_id) = actor.strip_prefix("agent:") {
            self.require_room_computer_access(agent_id)?;
        }
        Ok(())
    }
}
fn access_error() -> DaemonError {
    crate::error::HostFailure::Refused(crate::error::UserDomainRefusalReason::NotGranted)
        .into_daemon("room_computer")
}
