//! MCP servers and skills registered with Chariox as rows of a Project's Environment: the ones granted to an agent
//! of the Project are its registrations, and a room created in the Project is granted the saved ones.
use super::*;
use crate::extension::{ExtensionGrant, ExtensionKind};
use crate::local::EnvironmentEntryKind;
use crate::project_environment::{
    mcp_key_names, ExtensionEntry, ProjectEnvironmentStore, ProjectRegistrations, RegistrationHost,
    UserRegistrations,
};

impl KernelRuntimeState {
    pub(crate) fn project_registrations(
        &self,
        project: &crate::session::RuntimeProject,
    ) -> Result<ProjectRegistrations, DaemonError> {
        let mut mcps = Vec::new();
        let mut skills = Vec::new();
        for session in self.owned.session_store.sessions_in_project(project.id())? {
            for agent in self.owned.agent_store.get_session_agents(session.id()) {
                for grant in agent.extension_grants() {
                    let (entries, kind) = match grant.kind {
                        ExtensionKind::Mcp => (&mut mcps, EnvironmentEntryKind::Mcp),
                        ExtensionKind::Skill => (&mut skills, EnvironmentEntryKind::Skill),
                        _ => continue,
                    };
                    if UserRegistrations.has(kind, &grant.name)
                        && !entries
                            .iter()
                            .any(|entry: &ExtensionEntry| entry.name == grant.name)
                    {
                        entries.push(ExtensionEntry::registration(grant.name.clone()));
                    }
                }
            }
        }
        let mut key_names = Vec::new();
        for entry in &mcps {
            if let Ok(Some(crate::project_environment::Registration::Mcp(config))) =
                UserRegistrations.read(EnvironmentEntryKind::Mcp, &entry.name)
            {
                key_names.extend(mcp_key_names(&config));
            }
        }
        key_names.sort();
        key_names.dedup();
        Ok(ProjectRegistrations {
            mcps,
            skills,
            key_names,
        })
    }
}

impl KernelRuntimeState {
    /// The Project's Variables and Secret values requested by the MCP server `name` of this run.
    pub(super) fn project_mcp_environment(
        &self,
        run: &crate::provider::RuntimeProviderRun,
        name: &str,
    ) -> std::collections::BTreeMap<String, String> {
        let Some(server) = run.mcp_servers().iter().find(|server| server.name == name) else {
            return Default::default();
        };
        let Ok(session) = self.owned.session_store.get_session(run.session_id()) else {
            return Default::default();
        };
        let Some(project) = self.owned.session_store.environment_project(&session) else {
            return Default::default();
        };
        crate::project_environment::project_mcp_environment(
            &self.owned.config_projection.snapshot(),
            project.id(),
            &mcp_key_names(server),
        )
    }
}

impl KernelRuntimeOwnedState {
    /// A room created in a Project holds the grants of the Project's saved registrations this kernel has.
    pub(super) fn grant_project_registrations(&self, project_id: &str, agent_id: &str) {
        let store = ProjectEnvironmentStore::new(
            &self
                .config_projection
                .snapshot()
                .private_runtime_state_root(),
        );
        let Ok(Some(record)) = store.load(project_id) else {
            return;
        };
        for (kind, extension, entries) in [
            (EnvironmentEntryKind::Mcp, ExtensionKind::Mcp, &record.mcps),
            (
                EnvironmentEntryKind::Skill,
                ExtensionKind::Skill,
                &record.skills,
            ),
        ] {
            for entry in entries.iter().filter(|entry| entry.file_path().is_none()) {
                if UserRegistrations.has(kind, &entry.name) {
                    if let Err(error) = self.agent_store.grant_extension(
                        agent_id,
                        ExtensionGrant::new(extension.clone(), entry.name.clone()),
                    ) {
                        tracing::warn!(%error, name = %entry.name, "a Project registration was not granted to the new room");
                    }
                }
            }
        }
    }
}
