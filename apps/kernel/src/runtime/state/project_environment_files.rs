//! MP-08 / MP-10 / MP-11: Explicit imported-file retrieval on the bound source lease.
use super::project_environment_export::environment_failure;
use super::*;
use crate::project_environment::*;
use crate::transport::relay_peer::{RelayManagedContextChunk, RemoteNativeInteractionContext};
use std::io::Read;
impl KernelRuntimeState {
    pub(crate) async fn fetch_project_private_file(
        &self,
        worker_id: &str,
        context: RemoteNativeInteractionContext,
        workspace_id: String,
        path: String,
    ) -> Result<RelayManagedContextChunk, DaemonError> {
        let agent = self.owned.agent_store.get_agent(&context.home_agent_id)?;
        let binding = agent
            .remote_execution()
            .ok_or_else(|| environment_failure("source file caller has no worker lease"))?;
        if agent.session_id() != context.home_session_id
            || binding.worker_kernel_id != worker_id
            || binding.leased_agent_id != context.leased_agent_id
        {
            return Err(environment_failure(
                "source file caller does not match selected worker",
            ));
        }
        let session = self
            .owned
            .session_store
            .get_session(&context.home_session_id)?;
        if let Some(slice) = self
            .owned
            .slice_store
            .list()
            .into_iter()
            .find(|slice| slice.worker_kernel_id.as_deref() == Some(worker_id))
        {
            if let Some(source_ref) = &slice.source_slice_ref {
                let source_agent = self.source_slice_agent(source_ref)?;
                if self
                    .owned
                    .session_store
                    .get_session(source_agent.session_id())?
                    .project_id()
                    != session.project_id()
                {
                    return Err(environment_failure("private file source Project mismatch"));
                }
                let remote = source_agent
                    .remote_execution()
                    .ok_or_else(|| environment_failure("source worker binding unavailable"))?;
                let mut config = self.owned.config_projection.snapshot();
                if let (Some(url), Some(token)) = (&remote.relay_url, &remote.relay_token) {
                    config.apply_remote_relay_override(url.clone(), token.clone());
                }
                let response =
                    crate::transport::relay_client::send_peer_request_via_temporary_connection(
                        &config,
                        ClientTarget {
                            daemon_id: Some(remote.worker_kernel_id.clone()),
                            daemon_alias: None,
                        },
                        RelayPeerRequest::ReadLeasedProjectPrivateFile {
                            context: crate::transport::relay_peer::RemoteSkillSyncContext {
                                home_kernel_id: config.daemon_id.clone(),
                                home_session_id: source_agent.session_id().into(),
                                home_agent_id: source_agent.id().into(),
                                leased_agent_id: remote.leased_agent_id.clone(),
                            },
                            workspace_id,
                            path,
                        },
                    )
                    .await?;
                return match response {
                    RelayPeerResponse::ProjectPrivateFile { bytes } => Ok(bytes),
                    _ => Err(environment_failure(
                        "unexpected source worker file response",
                    )),
                };
            }
        }
        self.read_project_private_file(
            session.project_id(),
            &session,
            worker_id,
            workspace_id,
            path,
        )
    }
    pub(crate) async fn read_leased_project_private_file(
        &self,
        context: crate::transport::relay_peer::RemoteSkillSyncContext,
        workspace_id: String,
        path: String,
    ) -> Result<RelayManagedContextChunk, DaemonError> {
        let target = self
            .with_app_side_effect(move |app| {
                let mut runtime = crate::app::RemoteLeaseRuntime::new(app);
                runtime.consume_leased_agent_authorization(&context.leased_agent_id)?;
                runtime.leased_project_target(
                    &context.leased_agent_id,
                    &context.home_session_id,
                    &context.home_agent_id,
                    None,
                )
            })
            .await?;
        let session = self
            .owned
            .session_store
            .get_session(&target.backing_session_id)?;
        self.read_project_private_file(
            session.project_id(),
            &session,
            "source-worker",
            workspace_id,
            path,
        )
    }
    fn read_project_private_file(
        &self,
        project_id: &str,
        session: &crate::session::RuntimeSession,
        worker_id: &str,
        workspace_id: String,
        path: String,
    ) -> Result<RelayManagedContextChunk, DaemonError> {
        let config = self.owned.config_projection.snapshot();
        let state = ProjectEnvironmentStore::new(&config.private_runtime_state_root())
            .load(project_id)?
            .ok_or_else(|| environment_failure("source environment unavailable"))?;
        let file = state
            .manifest
            .private_files
            .iter()
            .find(|file| file.workspace_id == workspace_id && file.path == path)
            .ok_or_else(|| {
                environment_failure("source private file is not in the exported selection")
            })?;
        if file.secret_looking
            || secret_looking_project_path(&path)
            || crate::workspace_live_sync_ignore::workspace_live_sync_force_excluded_path(&path)
            || state.manifest.entries.iter().any(|entry| {
                entry.workspace_id == workspace_id
                    && entry.kind == ProjectEnvironmentEntryKind::ConfigFile
                    && entry.name == path
            })
        {
            return Err(environment_failure(
                "source private file belongs to sealed configuration",
            ));
        }
        let project = self.owned.session_store.get_project(project_id)?;
        if !project.contains_workspace(&workspace_id) {
            return Err(environment_failure(
                "source file workspace is outside Project",
            ));
        }
        let root = if session.workspace_id() == workspace_id {
            PathBuf::from(session.worktree_id())
        } else {
            PathBuf::from(&workspace_id)
        };
        let mut bytes = zeroize::Zeroizing::new(Vec::new());
        open_workspace_file(&root, &path)?
            .take(16 * 1024 * 1024 + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| environment_failure("source private file read failed"))?;
        if bytes.len() > 16 * 1024 * 1024 {
            return Err(environment_failure(
                "source private file exceeds transfer bounds",
            ));
        }
        if std::str::from_utf8(&bytes).is_ok_and(contains_secret_configuration) {
            return Err(environment_failure(
                "source file now requires sealed configuration",
            ));
        }
        // Audit only identities, never file contents.
        tracing::info!(
            project_id = project.id(),
            worker_id,
            "MP-08 / MP-10 / MP-11 explicit Project file retrieval"
        );
        Ok(RelayManagedContextChunk::new(
            base64::engine::general_purpose::STANDARD.encode(&*bytes),
        ))
    }
    pub(super) async fn retrieve_project_private_files(
        &self,
        state: &StoredProjectEnvironment,
        roots: &BTreeMap<String, PathBuf>,
    ) -> Result<ProjectPrivateFileAdditions, DaemonError> {
        let mut additions = ProjectPrivateFileAdditions::default();
        for file in state
            .manifest
            .private_files
            .iter()
            .filter(|file| file.bring)
        {
            let root = roots
                .get(&file.workspace_id)
                .ok_or_else(|| environment_failure("private file workspace unavailable"))?;
            if open_workspace_file(root, &file.path).is_ok() {
                continue;
            }
            // Never overwrite an unsafe or user-created target entry.
            if root.join(&file.path).symlink_metadata().is_ok() {
                return Err(environment_failure("private file target is unsafe"));
            }
            let source = state
                .source
                .as_ref()
                .ok_or_else(|| environment_failure("private file source is unavailable"))?;
            let workspace_id = source
                .workspaces
                .get(&file.workspace_id)
                .ok_or_else(|| environment_failure("private file source mapping unavailable"))?
                .clone();
            let config = self.owned.config_projection.snapshot();
            let response =
                crate::transport::relay_client::send_peer_request_via_temporary_connection(
                    &config,
                    ClientTarget {
                        daemon_id: Some(source.kernel_id.clone()),
                        daemon_alias: None,
                    },
                    RelayPeerRequest::FetchProjectPrivateFile {
                        context: source.context.clone(),
                        workspace_id,
                        path: file.path.clone(),
                    },
                )
                .await?;
            let RelayPeerResponse::ProjectPrivateFile { bytes } = response else {
                return Err(environment_failure("unexpected private file response"));
            };
            let encoded = zeroize::Zeroizing::new(bytes.into_inner());
            if encoded.len() > 24 * 1024 * 1024 {
                return Err(environment_failure("private file response exceeds bounds"));
            }
            let bytes = zeroize::Zeroizing::new(
                base64::engine::general_purpose::STANDARD
                    .decode(encoded.as_bytes())
                    .map_err(|_| environment_failure("invalid private file response"))?,
            );
            additions.add(root, &file.path, &bytes)?;
        }
        Ok(additions)
    }
}
