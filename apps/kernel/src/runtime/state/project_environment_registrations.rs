//! MCP servers and skills registered with Chariox as rows of a Project's Environment: the MCPs granted to an agent of
//! the Project and the Project-scope skills are its registrations, and a room created in the Project is granted the
//! saved MCPs. Skills need no grant: every agent of the Project sees its Project-scope skills.
use super::*;
use crate::extension::{ExtensionGrant, ExtensionKind};
use crate::local::EnvironmentEntryKind;
use crate::project_environment::{
    mcp_key_names, ExtensionEntry, ProjectEnvironmentStore, ProjectRegistrations, RegistrationHost,
    UserRegistrations,
};

impl KernelRuntimeState {
    /// The user's registrations for `project_id`, with the home skill catalog when this kernel has one.
    pub(super) fn user_registrations(&self, project_id: &str) -> UserRegistrations {
        UserRegistrations {
            skills: self.skills_catalog_service().ok(),
            project_id: project_id.to_string(),
        }
    }

    pub(crate) fn project_registrations(
        &self,
        project: &crate::session::RuntimeProject,
    ) -> Result<ProjectRegistrations, DaemonError> {
        let registrations = self.user_registrations(project.id());
        let mut mcps = Vec::new();
        for session in self.owned.session_store.sessions_in_project(project.id())? {
            for agent in self.owned.agent_store.get_session_agents(session.id()) {
                for grant in agent.extension_grants() {
                    if grant.kind == ExtensionKind::Mcp
                        && registrations.has(EnvironmentEntryKind::Mcp, &grant.name)
                        && !mcps
                            .iter()
                            .any(|entry: &ExtensionEntry| entry.name == grant.name)
                    {
                        mcps.push(ExtensionEntry::registration(grant.name.clone()));
                    }
                }
            }
        }
        let mut key_names = Vec::new();
        for entry in &mcps {
            if let Ok(Some(crate::project_environment::Registration::Mcp(config))) =
                registrations.read(EnvironmentEntryKind::Mcp, &entry.name)
            {
                key_names.extend(mcp_key_names(&config));
            }
        }
        key_names.sort();
        key_names.dedup();
        let skills = match &registrations.skills {
            Some(catalog) => catalog
                .project_skill_names(project.id())?
                .into_iter()
                .map(ExtensionEntry::registration)
                .collect(),
            None => Vec::new(),
        };
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
    /// A room created in a Project holds the grants of the Project's saved MCP registrations this kernel has.
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
        // Only MCPs are granted; skills reach every agent of the Project through the catalog.
        let registrations = UserRegistrations {
            skills: None,
            project_id: project_id.to_string(),
        };
        for entry in record
            .mcps
            .iter()
            .filter(|entry| entry.file_path().is_none())
        {
            if registrations.has(EnvironmentEntryKind::Mcp, &entry.name) {
                if let Err(error) = self.agent_store.grant_extension(
                    agent_id,
                    ExtensionGrant::new(ExtensionKind::Mcp, entry.name.clone()),
                ) {
                    tracing::warn!(%error, name = %entry.name, "a Project registration was not granted to the new room");
                }
            }
        }
    }
}
