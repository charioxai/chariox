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
