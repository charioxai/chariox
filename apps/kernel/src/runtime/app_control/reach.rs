//! Which installed Apps an agent may ask for. The agent's own request binds
//! one (no person is asked); installing an App exposes nothing by itself. A
//! guest's agent reaches the Room owner's Apps inside the invited Room only.

use super::AppControlService;
use chariox_app_runtime::installation::Installation;
use std::collections::BTreeSet;

/// The Apps one agent may ask for: the Room owner's Release installations, the
/// Dev installation of the agent's own Room, and, in a deployment's copy
/// session, only that deployment's copies.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct AppReach {
    pub(crate) agent: String,
    /// The Room owner, whose installations the agent can use.
    pub(crate) owner: String,
    pub(crate) room: String,
    /// The deployment whose copy session the agent runs in.
    pub(crate) deployment: Option<String>,
    /// The installations the agent has asked for: their tools are exposed.
    pub(crate) granted: BTreeSet<String>,
}

impl AppReach {
    /// The Room owner's installed Apps are reachable by every agent in the
    /// Room, including a guest's. A Room that no longer exists reaches the
    /// agent owner's Apps only.
    pub(crate) fn for_agent(
        agent: &crate::agent::AgentInstance,
        session: Option<&crate::session::RuntimeSession>,
    ) -> Self {
        Self {
            agent: agent.id().to_owned(),
            owner: session
                .map_or_else(|| agent.owner_user_id(), |session| session.owner_user_id())
                .to_owned(),
            room: agent.session_id().to_owned(),
            granted: agent
                .granted_extension_names(crate::extension::ExtensionKind::App)
                .into_iter()
                .collect(),
            deployment: session.and_then(|session| {
                session.workflow_publications().iter().find_map(|publication| {
                    let key = &publication.runtime_materialization()?.key;
                    let (deployment, _release) = key
                        .strip_prefix(
                            crate::runtime::state::workflow_publication_runtime_lifecycle::DEPLOYMENT_COPY_KEY_PREFIX,
                        )?
                        .rsplit_once(':')?;
                    Some(deployment.to_owned())
                })
            }),
        }
    }

    pub(crate) fn admits(&self, installation: &Installation, deployment: Option<&String>) -> bool {
        installation.visible_in_room(Some(&self.room)) && deployment == self.deployment.as_ref()
    }
}

impl AppControlService {
    /// Whether `reach` admits the one installation. The caller holds an App
    /// admission slot.
    pub(crate) fn can_reach(&self, reach: &AppReach, installation: &str) -> bool {
        let Ok(record) = self.store.get_app_installation(&reach.owner, installation) else {
            return false;
        };
        let Ok(deployments) = self.store.app_installation_deployments(&reach.owner) else {
            return false;
        };
        reach.admits(&record, deployments.get(installation))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chariox_app_runtime::installation::channel::InstallationChannel;

    fn installation(channel: InstallationChannel, dev_room: Option<&str>) -> Installation {
        Installation {
            installation_id: "app".into(),
            app_id: "com.example.app".into(),
            owner_id: "host".into(),
            channel,
            dev_room_id: dev_room.map(str::to_owned),
            generation: 1,
            active: None,
            pending_generation: None,
            admission_paused: false,
            retained: None,
        }
    }

    fn agent(session: &str, owner: &str) -> crate::agent::AgentInstance {
        let mut agent = crate::agent::AgentInstance::new(
            "agent-1",
            "ref-1",
            session,
            None,
            "dev-stub",
            None,
            None,
            None,
            crate::agent::GridPosition::new(0, 0, 1, 1),
        );
        agent.set_owner_user_id(owner);
        agent
    }

    fn room(id: &str, owner: &str) -> crate::session::RuntimeSession {
        let mut session = crate::session::RuntimeSession::new(
            id,
            None,
            "workspace",
            "/repo",
            "machine",
            "kernel",
        );
        session.set_owner_user_id(owner);
        session
    }

    #[test]
    fn a_guests_agent_reaches_the_room_owners_apps_in_that_room_only() {
        let guest = agent("invited-room", "guest");
        let reach = AppReach::for_agent(&guest, Some(&room("invited-room", "host")));
        assert_eq!(
            reach.owner, "host",
            "the Room owner's installations, not the guest's"
        );
        assert_eq!(reach.room, "invited-room");
        let release = installation(InstallationChannel::Release, None);
        assert!(reach.admits(&release, None));
        // The same guest agent in a Room it was not invited to has no such reach.
        let elsewhere = AppReach::for_agent(&guest, Some(&room("other-room", "stranger")));
        assert_eq!(elsewhere.owner, "stranger");
    }

    #[test]
    fn a_dev_installation_is_reachable_from_its_own_room_only() {
        let dev = installation(InstallationChannel::Dev, Some("dev-room"));
        let own = AppReach::for_agent(&agent("dev-room", "host"), Some(&room("dev-room", "host")));
        let other = AppReach::for_agent(&agent("other", "host"), Some(&room("other", "host")));
        assert!(own.admits(&dev, None));
        assert!(!other.admits(&dev, None));
    }

    #[test]
    fn a_deployment_copy_is_reachable_from_its_deployment_only() {
        let release = installation(InstallationChannel::Release, None);
        let plain = AppReach::for_agent(&agent("room", "host"), Some(&room("room", "host")));
        let mut deployed = plain.clone();
        deployed.deployment = Some("deployment-1".into());
        let tagged = "deployment-1".to_owned();
        assert!(plain.admits(&release, None));
        assert!(
            !plain.admits(&release, Some(&tagged)),
            "an ordinary agent never reaches a copy"
        );
        assert!(deployed.admits(&release, Some(&tagged)));
        assert!(
            !deployed.admits(&release, None),
            "a deployed agent reaches its copies only"
        );
    }
}
