//! MP-08 / MP-10: Values install only within the authenticated lease's mounted repositories.
use super::RemoteLeaseRuntime;
use crate::{
    error::DaemonError, project_environment::*, transport::relay_peer::RemoteSkillSyncContext,
};
use std::{
    collections::BTreeMap,
    path::{Component, Path},
};

impl RemoteLeaseRuntime<'_> {
    pub(crate) fn install_project_environment(
        &mut self,
        context: RemoteSkillSyncContext,
        layer: crate::managed_context::development::DevelopmentProjectEnvironment,
        directories: BTreeMap<String, String>,
    ) -> Result<String, DaemonError> {
        self.consume_leased_agent_authorization(&context.leased_agent_id)?;
        let agent = self
            .app
            .leased_agents
            .get(&context.leased_agent_id)
            .ok_or_else(|| failure("leased agent missing"))?;
        let lease = self
            .app
            .execution_leases
            .get(&agent.lease_id)
            .ok_or_else(|| failure("execution lease missing"))?;
        if lease.home_kernel_id != context.home_kernel_id
            || lease.home_session_id != context.home_session_id
            || lease.home_agent_id != context.home_agent_id
            || layer.sealed.binding.context_id != lease.id
        {
            return Err(failure(
                "Project environment context does not match the lease",
            ));
        }
        let session = self.app.sessions.get_session(&agent.backing_session_id)?;
        let root = std::fs::canonicalize(session.worktree_id())
            .map_err(|_| failure("leased repository unavailable"))?;
        let parent = root
            .parent()
            .ok_or_else(|| failure("leased repository parent unavailable"))?;
        let mut roots = BTreeMap::new();
        let mut mapping = BTreeMap::new();
        for (workspace, basename) in directories {
            if basename.is_empty()
                || Path::new(&basename).components().count() != 1
                || !Path::new(&basename)
                    .components()
                    .all(|c| matches!(c, Component::Normal(_)))
            {
                return Err(failure("Project environment repository basename invalid"));
            }
            let path = parent.join(basename);
            if path
                .symlink_metadata()
                .map_err(|_| failure("mounted Project repository unavailable"))?
                .file_type()
                .is_symlink()
                || std::fs::canonicalize(&path)
                    .map_err(|_| failure("mounted Project repository unavailable"))?
                    != path
            {
                return Err(failure(
                    "Project environment repository escapes mounted parent",
                ));
            }
            mapping.insert(workspace.clone(), path.to_string_lossy().into_owned());
            roots.insert(workspace, path);
        }
        if !roots.values().any(|path| path == &root)
            || layer
                .sealed
                .manifest
                .entries
                .iter()
                .any(|entry| !roots.contains_key(&entry.workspace_id))
        {
            return Err(failure(
                "Project environment does not include the leased repository",
            ));
        }
        let project_id = layer.sealed.manifest.project_id.clone();
        let mut workspace_ids = vec![session.workspace_id().to_string()];
        workspace_ids.extend(
            mapping
                .values()
                .filter(|path| *path != session.workspace_id())
                .cloned(),
        );
        let project = self.app.sessions.read().prepare_leased_project(
            session.id(),
            &project_id,
            workspace_ids,
        )?;
        let project_exists = self.app.sessions.get_project(&project_id).is_ok();
        let store = ProjectEnvironmentStore::new(&self.app.config.private_runtime_state_root());
        // Independent target state is never refreshed from home after its initial import.
        if store.load(&project_id)?.is_none() {
            let authority = ProjectEnvironmentImportAuthority {
                config: self.app.config.clone(),
                context_id: lease.id.clone(),
                source_kernel_id: context.home_kernel_id,
                source_key_thumbprint: crate::runtime::terminal_pairings::public_key_thumbprint(
                    &layer.sealed.values.sender_public_key,
                ),
                target_kernel_id: self.app.config.daemon_id.clone(),
            };
            PreparedProjectEnvironmentImport::prepare_for_project(
                &authority,
                &layer,
                &roots,
                &mapping,
                &project_id,
            )?
            .commit()?;
        }
        if !project_exists {
            self.app.durable_state.append_event(
                "project.created",
                Some(project_id.clone()),
                serde_json::json!({"project": &project}),
            )?;
        }
        let bound = self
            .app
            .sessions
            .write()
            .bind_leased_project(session.id(), project)?;
        self.app.update_session_projection(bound);
        Ok(project_id)
    }
}
fn failure(message: &'static str) -> DaemonError {
    DaemonError::LocalTransport {
        operation: "install leased Project environment",
        message: message.into(),
    }
}
