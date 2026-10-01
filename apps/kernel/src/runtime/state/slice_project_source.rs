//! MP-08 / MP-10 / MP-11: Slice-to-slice export takes code from the owned mounted snapshot,
//! and environment decisions/values from its actual execution kernel.
use super::project_environment_export::environment_failure;
use super::*;
use crate::managed_context::{development::*, package::ManagedContextDevelopmentSelection};
use crate::project_environment::*;
impl KernelRuntimeState {
    pub(crate) fn validate_source_slice_export(
        &self,
        source_ref: &str,
        development: Option<&ManagedContextDevelopmentSelection>,
    ) -> Result<(), DaemonError> {
        let source = self.resolve_slice(source_ref)?;
        let config = self.owned.config_projection.snapshot();
        let Some(ManagedContextDevelopmentSelection::SourceProject {
            project_id,
            repositories,
        }) = development
        else {
            return Err(environment_failure(
                "source slice requires a selected Project",
            ));
        };
        let Some(ManagedContextDevelopmentSelection::SourceProject {
            project_id: source_project,
            repositories: source_repositories,
        }) = &source.development
        else {
            return Err(environment_failure("source slice has no Project snapshot"));
        };
        if source.owner_kernel_id != config.daemon_id
            || source.backend != crate::slice::SliceBackendKind::LocalDocker
            || source.status != crate::slice::SliceStatus::Running
            || source.development_publication.is_none()
            || project_id != source_project
            || repositories != source_repositories
        {
            return Err(environment_failure(
                "source slice is not this kernel's running selected Project snapshot",
            ));
        }
        Ok(())
    }
    pub(crate) fn slice_repository_selections(
        &self,
        slice: &crate::slice::SliceRecord,
    ) -> Result<Vec<DevelopmentRepositorySelection>, DaemonError> {
        let Some(ManagedContextDevelopmentSelection::SourceProject { repositories, .. }) =
            &slice.development
        else {
            return Ok(vec![]);
        };
        let Some(source_ref) = &slice.source_slice_ref else {
            return repositories
                .iter()
                .map(crate::managed_context::outbound_service::resolve_repository_selection)
                .collect();
        };
        self.validate_source_slice_export(source_ref, slice.development.as_ref())?;
        let source = self.resolve_slice(source_ref)?;
        let publication = source
            .development_publication
            .as_ref()
            .ok_or_else(|| environment_failure("source publication missing"))?;
        if publication.repository_paths.len() != repositories.len() {
            return Err(environment_failure(
                "source repository mapping is incomplete",
            ));
        }
        repositories
            .iter()
            .zip(&publication.repository_paths)
            .map(|(repository, path)| {
                let root = std::fs::canonicalize(path)
                    .map_err(|_| environment_failure("source repository unavailable"))?;
                if root != PathBuf::from(path) || !root.starts_with(&publication.destination_root) {
                    return Err(environment_failure(
                        "source repository escaped the owned publication",
                    ));
                }
                Ok(DevelopmentRepositorySelection {
                    workspace_id: repository.workspace_id.clone(),
                    worktree_id: repository.worktree_id.clone(),
                    worktree_path: root,
                    role: repository.role,
                })
            })
            .collect()
    }
    pub(super) fn source_slice_agent(
        &self,
        source_ref: &str,
    ) -> Result<crate::agent::AgentInstance, DaemonError> {
        let source = self.resolve_slice(source_ref)?;
        let Some(ManagedContextDevelopmentSelection::SourceProject { project_id, .. }) =
            &source.development
        else {
            return Err(environment_failure("source slice Project missing"));
        };
        self.owned
            .agent_store
            .list_agents()
            .into_iter()
            .find(|agent| {
                agent.remote_execution().is_some_and(|remote| {
                    Some(&remote.worker_kernel_id) == source.worker_kernel_id.as_ref()
                }) && self
                    .owned
                    .session_store
                    .get_session(agent.session_id())
                    .is_ok_and(|session| {
                        session.project_id() == project_id
                            && session.owner_user_id() == agent.owner_user_id()
                    })
                    && !agent.is_processing()
            })
            .ok_or_else(|| {
                environment_failure("source slice needs an existing leased Project agent")
            })
    }
    pub(crate) async fn refresh_slice_source_environment(
        &self,
        slice: &crate::slice::SliceRecord,
        interactive: bool,
    ) -> Result<(), DaemonError> {
        let source_ref = slice
            .source_slice_ref
            .as_deref()
            .ok_or_else(|| environment_failure("source slice missing"))?;
        let agent = self.source_slice_agent(source_ref)?;
        let response = self
            .export_remote_project_environment(&agent, interactive, &slice.name, None)
            .await?;
        let RelayPeerResponse::LeasedProjectEnvironmentExport { mut manifest, .. } = response
        else {
            return Err(environment_failure(
                "unexpected source environment response",
            ));
        };
        let selections = self.slice_repository_selections(slice)?;
        let roots: BTreeMap<_, _> = selections
            .iter()
            .map(|r| (r.workspace_id.clone(), r.worktree_path.clone()))
            .collect();
        let workspace_mapping: BTreeMap<_, _> = manifest
            .private_files
            .iter()
            .map(|file| file.workspace_id.clone())
            .chain(
                manifest
                    .entries
                    .iter()
                    .map(|entry| entry.workspace_id.clone()),
            )
            .map(|workspace| {
                let basename = Path::new(&workspace).file_name();
                let selected = selections
                    .iter()
                    .find(|r| r.worktree_path.file_name() == basename)
                    .ok_or_else(|| {
                        environment_failure("source environment repository mapping is incomplete")
                    })?;
                Ok((workspace, selected.workspace_id.clone()))
            })
            .collect::<Result<_, DaemonError>>()?;
        for entry in &mut manifest.entries {
            entry.workspace_id = workspace_mapping[&entry.workspace_id].clone();
        }
        for file in &mut manifest.private_files {
            file.workspace_id = workspace_mapping[&file.workspace_id].clone();
        }
        register_project_file_rules(&manifest, &roots);
        Ok(())
    }
}
