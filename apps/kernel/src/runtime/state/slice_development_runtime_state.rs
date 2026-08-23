use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::error::DaemonError;
use crate::managed_context::development::{
    cleanup_development_context_publication, cleanup_development_context_publication_staging,
    export_development_context, import_development_context_with_publication,
    recover_pruned_development_context_publication_for_cleanup,
    recover_pruned_mutable_development_context_publication, DevelopmentContextExportRequest,
    DevelopmentContextImportRequest, DevelopmentRepositoryRole, DevelopmentSourceRepositoryBinding,
};
use crate::managed_context::package::ManagedContextDevelopmentSelection;

use super::KernelRuntimeState;

const SLICE_DEVELOPMENT_PUBLICATION_ID: &str = "development";
const SLICE_DEVELOPMENT_EXPORT_SCRATCH: &str = ".slice-development-export";
const MANAGED_PUBLICATION_ACCESS_HELPER: &str = "/usr/lib/chariox/slice-build-context/apps/kernel/slice-linux-docker/managed-publication-access.sh";

impl KernelRuntimeState {
    pub(crate) fn prepare_slice_development_storage_parent(
        &self,
        config: &crate::config::DaemonConfig,
    ) -> Result<PathBuf, DaemonError> {
        let options = crate::slice::LocalDockerSliceOptions::from_config(config);
        let storage_parent = options.root.join("development");
        ensure_private_real_directory(&storage_parent)?;
        fs::canonicalize(&storage_parent)
            .map_err(|error| slice_development_io_error("resolve", &storage_parent, error))
    }

    pub(crate) fn slice_development_selection_for_session(
        &self,
        session: &crate::session::RuntimeSession,
        worktree_id: &str,
    ) -> Result<ManagedContextDevelopmentSelection, DaemonError> {
        let project = self.owned.session_store.get_project(session.project_id())?;
        let mut repositories = Vec::with_capacity(project.workspace_ids().len());
        repositories.push(DevelopmentSourceRepositoryBinding {
            role: DevelopmentRepositoryRole::Primary,
            workspace_id: session.workspace_id().to_string(),
            worktree_id: Some(worktree_id.to_string()),
        });
        repositories.extend(
            project
                .workspace_ids()
                .iter()
                .filter(|workspace_id| workspace_id.as_str() != session.workspace_id())
                .map(|workspace_id| DevelopmentSourceRepositoryBinding {
                    role: DevelopmentRepositoryRole::Supporting,
                    workspace_id: workspace_id.clone(),
                    worktree_id: None,
                }),
        );
        let selection = ManagedContextDevelopmentSelection::SourceProject {
            project_id: project.id().to_string(),
            repositories,
        };
        self.validate_slice_development_selection(
            Some(&selection),
            &crate::slice::SliceBackendKind::LocalDocker,
            Some(session.workspace_id()),
            Some(worktree_id),
        )?;
        Ok(selection)
    }

    pub(crate) fn validate_slice_development_selection(
        &self,
        development: Option<&ManagedContextDevelopmentSelection>,
        backend: &crate::slice::SliceBackendKind,
        workspace_id: Option<&str>,
        worktree_id: Option<&str>,
    ) -> Result<(), DaemonError> {
        let Some(ManagedContextDevelopmentSelection::SourceProject {
            project_id,
            repositories,
        }) = development
        else {
            return Ok(());
        };
        if *backend != crate::slice::SliceBackendKind::LocalDocker {
            return Err(slice_development_error(
                "source-project development materialization requires a local Docker slice",
            ));
        }
        crate::managed_context::package::validate_development_selection(
            development.expect("source project selection"),
        )?;
        let primary = repositories
            .iter()
            .find(|repository| repository.role == DevelopmentRepositoryRole::Primary)
            .expect("validated development selection has one primary repository");
        if workspace_id != Some(primary.workspace_id.as_str())
            || worktree_id != primary.worktree_id.as_deref()
        {
            return Err(slice_development_error(
                "slice workspace/worktree must match the primary development repository",
            ));
        }
        let project = self.owned.session_store.get_project(project_id)?;
        for repository in repositories {
            if !project.contains_workspace(&repository.workspace_id) {
                return Err(slice_development_error(format!(
                    "project `{project_id}` does not include Workspace `{}`",
                    repository.workspace_id
                )));
            }
            crate::managed_context::outbound_service::resolve_repository_selection(repository)?;
        }
        Ok(())
    }

    pub(crate) fn materialize_slice_development_context(
        &self,
        slice: &crate::slice::SliceRecord,
    ) -> Result<crate::slice::SliceRecord, DaemonError> {
        let Some(ManagedContextDevelopmentSelection::SourceProject {
            project_id,
            repositories,
        }) = slice.development.as_ref()
        else {
            return Ok(slice.clone());
        };
        let publication_parent = slice_development_storage_root(slice)?;
        let publication = materialize_slice_development_publication(
            &publication_parent,
            project_id,
            repositories,
            slice.development_publication.as_ref(),
        )?;
        if slice
            .development_publication
            .as_ref()
            .is_some_and(|expected| expected != &publication)
        {
            return Err(slice_development_error(
                "recovered slice development publication does not match durable state",
            ));
        }
        let updated = self.owned.slice_store.set_development_publication(
            &slice.id,
            publication,
            crate::session::unix_epoch_ms(),
        )?;
        self.append_slice_durable_event("slice.updated", &updated)?;
        Ok(updated)
    }

    pub(crate) fn cleanup_slice_development_context(
        &self,
        slice: &crate::slice::SliceRecord,
    ) -> Result<(), DaemonError> {
        let Some(ManagedContextDevelopmentSelection::SourceProject {
            project_id,
            repositories,
        }) = slice.development.as_ref()
        else {
            return Ok(());
        };
        let publication_parent = slice_development_storage_root(slice)?;
        cleanup_slice_development_publication(&publication_parent, project_id, repositories)
    }
}

fn materialize_slice_development_publication(
    publication_parent: &Path,
    project_id: &str,
    repositories: &[DevelopmentSourceRepositoryBinding],
    expected_publication: Option<&crate::slice::SliceDevelopmentPublication>,
) -> Result<crate::slice::SliceDevelopmentPublication, DaemonError> {
    ensure_private_real_directory(publication_parent)?;
    let canonical_publication_parent = fs::canonicalize(publication_parent)
        .map_err(|error| slice_development_io_error("resolve", publication_parent, error))?;
    if canonical_publication_parent != publication_parent {
        return Err(slice_development_error(
            "slice development storage root changed after it was recorded",
        ));
    }
    let publication_parent = canonical_publication_parent;
    cleanup_slice_development_export_scratch(&publication_parent)?;
    let receipt = match recover_pruned_mutable_development_context_publication(
        &publication_parent,
        project_id,
        repositories,
    )? {
        Some(receipt) => receipt,
        None if expected_publication.is_some() => {
            return Err(slice_development_error(
                "durable slice development publication is missing",
            ))
        }
        None => {
            let scratch_root = publication_parent.join(SLICE_DEVELOPMENT_EXPORT_SCRATCH);
            ensure_private_real_directory(&scratch_root)?;
            let scratch_cleanup = SliceDevelopmentScratchCleanup(scratch_root.clone());
            let archive_path = scratch_root.join("development.tar.gz");
            let resolved = repositories
                .iter()
                .map(crate::managed_context::outbound_service::resolve_repository_selection)
                .collect::<Result<Vec<_>, _>>()?;
            let exported = export_development_context(DevelopmentContextExportRequest {
                project_id: project_id.to_string(),
                repositories: resolved,
                archive_path: archive_path.clone(),
            })?;
            let publication_id = SLICE_DEVELOPMENT_PUBLICATION_ID.to_string();
            let destination_root = publication_parent.join(&publication_id);
            let request = DevelopmentContextImportRequest {
                archive_path,
                expected_archive_sha256: exported.archive_sha256,
                expected_project_id: project_id.to_string(),
                expected_source_repositories: Some(repositories.to_vec()),
                destination_root: destination_root.clone(),
            };
            let imported =
                import_development_context_with_publication(request, publication_id.clone());
            drop(scratch_cleanup);
            match imported {
                Ok(receipt) => receipt,
                Err(error) => {
                    let _ = cleanup_development_context_publication_staging(
                        &destination_root,
                        &publication_id,
                    );
                    return Err(error);
                }
            }
        }
    };
    let primary = receipt
        .repositories
        .iter()
        .find(|repository| repository.role == DevelopmentRepositoryRole::Primary)
        .ok_or_else(|| slice_development_error("slice publication has no primary repository"))?;
    let repository_paths = receipt
        .repositories
        .iter()
        .map(|repository| path_to_string(&repository.destination_path))
        .collect::<Result<Vec<_>, _>>()?;
    let publication = crate::slice::SliceDevelopmentPublication {
        publication_id: receipt.publication_id,
        destination_root: path_to_string(&receipt.destination_root)?,
        primary_repository_path: path_to_string(&primary.destination_path)?,
        repository_paths,
    };
    update_managed_publication_access("grant", &publication_parent, &publication)?;
    Ok(publication)
}

fn cleanup_slice_development_publication(
    publication_parent: &Path,
    project_id: &str,
    repositories: &[DevelopmentSourceRepositoryBinding],
) -> Result<(), DaemonError> {
    let parent_metadata = match fs::symlink_metadata(publication_parent) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => {
            return Err(slice_development_io_error(
                "inspect",
                publication_parent,
                error,
            ))
        }
    };
    if !parent_metadata.is_dir() || parent_metadata.file_type().is_symlink() {
        return Err(slice_development_error(
            "slice development publication parent is unsafe",
        ));
    }
    let expected_publication_parent = publication_parent.to_path_buf();
    let publication_parent = fs::canonicalize(publication_parent)
        .map_err(|error| slice_development_io_error("resolve", publication_parent, error))?;
    if publication_parent != expected_publication_parent {
        return Err(slice_development_error(
            "slice development storage root changed after it was recorded",
        ));
    }
    cleanup_slice_development_export_scratch(&publication_parent)?;
    let recovered = recover_pruned_development_context_publication_for_cleanup(
        &publication_parent,
        project_id,
        repositories,
    )?;
    let destination_root = publication_parent.join(SLICE_DEVELOPMENT_PUBLICATION_ID);
    cleanup_development_context_publication_staging(
        &destination_root,
        SLICE_DEVELOPMENT_PUBLICATION_ID,
    )?;
    if let Some(receipt) = recovered {
        let publication = crate::slice::SliceDevelopmentPublication {
            publication_id: receipt.publication_id.clone(),
            destination_root: path_to_string(&receipt.destination_root)?,
            primary_repository_path: receipt
                .repositories
                .iter()
                .find(|repository| repository.role == DevelopmentRepositoryRole::Primary)
                .map(|repository| path_to_string(&repository.destination_path))
                .transpose()?
                .ok_or_else(|| {
                    slice_development_error("slice publication has no primary repository")
                })?,
            repository_paths: receipt
                .repositories
                .iter()
                .map(|repository| path_to_string(&repository.destination_path))
                .collect::<Result<Vec<_>, _>>()?,
        };
        update_managed_publication_access("revoke", &publication_parent, &publication)?;
        cleanup_development_context_publication(
            &receipt.destination_root,
            &receipt.publication_id,
        )?;
    }
    fs::remove_dir(&publication_parent)
        .map_err(|error| slice_development_io_error("remove", &publication_parent, error))
}

fn update_managed_publication_access(
    action: &str,
    storage_root: &Path,
    publication: &crate::slice::SliceDevelopmentPublication,
) -> Result<(), DaemonError> {
    if !crate::slice::managed_docker_broker_configured() {
        return Ok(());
    }
    let status = Command::new(MANAGED_PUBLICATION_ACCESS_HELPER)
        .arg(action)
        .arg(storage_root)
        .arg(&publication.destination_root)
        .args(&publication.repository_paths)
        .status()
        .map_err(|error| {
            slice_development_error(format!("run managed publication access helper: {error}"))
        })?;
    if status.success() {
        Ok(())
    } else {
        Err(slice_development_error(format!(
            "managed publication access helper failed with {status}"
        )))
    }
}

struct SliceDevelopmentScratchCleanup(PathBuf);

impl Drop for SliceDevelopmentScratchCleanup {
    fn drop(&mut self) {
        let _ = cleanup_slice_development_export_scratch_root(&self.0);
    }
}

fn slice_development_storage_root(
    slice: &crate::slice::SliceRecord,
) -> Result<PathBuf, DaemonError> {
    let storage_root = slice
        .development_storage_root
        .as_deref()
        .map(PathBuf::from)
        .or_else(|| {
            slice
                .development_publication
                .as_ref()
                .and_then(|publication| Path::new(&publication.destination_root).parent())
                .map(Path::to_path_buf)
        })
        .ok_or_else(|| {
            slice_development_error("slice development storage root is missing from durable state")
        })?;
    if !storage_root.is_absolute() {
        return Err(slice_development_error(
            "slice development storage root is not absolute",
        ));
    }
    if let Some(publication) = slice.development_publication.as_ref() {
        let destination = Path::new(&publication.destination_root);
        if destination.parent() != Some(storage_root.as_path())
            || destination.file_name().and_then(|name| name.to_str())
                != Some(publication.publication_id.as_str())
        {
            return Err(slice_development_error(
                "slice development publication escaped its durable storage root",
            ));
        }
    }
    Ok(storage_root)
}

fn cleanup_slice_development_export_scratch(publication_parent: &Path) -> Result<(), DaemonError> {
    cleanup_slice_development_export_scratch_root(
        &publication_parent.join(SLICE_DEVELOPMENT_EXPORT_SCRATCH),
    )
}

fn cleanup_slice_development_export_scratch_root(path: &Path) -> Result<(), DaemonError> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(slice_development_io_error("inspect", path, error)),
    };
    if metadata.file_type().is_symlink() || metadata.is_file() {
        fs::remove_file(path).map_err(|error| slice_development_io_error("remove", path, error))?;
    } else if metadata.is_dir() {
        fs::remove_dir_all(path)
            .map_err(|error| slice_development_io_error("remove", path, error))?;
    } else {
        return Err(slice_development_error(
            "slice development export scratch has an unsupported file type",
        ));
    }
    Ok(())
}

fn ensure_private_real_directory(path: &Path) -> Result<(), DaemonError> {
    fs::create_dir_all(path).map_err(|error| slice_development_io_error("create", path, error))?;
    let metadata = fs::symlink_metadata(path)
        .map_err(|error| slice_development_io_error("inspect", path, error))?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err(slice_development_error(format!(
            "slice development root `{}` is not a real directory",
            path.display()
        )));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))
            .map_err(|error| slice_development_io_error("protect", path, error))?;
    }
    Ok(())
}

fn path_to_string(path: &Path) -> Result<String, DaemonError> {
    path.to_str()
        .map(str::to_string)
        .ok_or_else(|| slice_development_error("slice development path is not portable UTF-8"))
}

fn slice_development_io_error(operation: &str, path: &Path, error: std::io::Error) -> DaemonError {
    slice_development_error(format!(
        "failed to {operation} slice development root {}: {error}",
        path.display()
    ))
}

fn slice_development_error(message: impl Into<String>) -> DaemonError {
    DaemonError::LocalTransport {
        operation: "slice.development",
        message: message.into(),
    }
}

#[cfg(test)]
mod tests {
    use std::process::Command;
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::*;

    #[test]
    fn slice_development_materializes_recovers_and_cleans_two_repositories() {
        let root = test_root("slice-project-materialization");
        let primary_source = root.join("primary-source");
        let supporting_source = root.join("supporting-source");
        init_repository(&primary_source, "primary.txt", "primary source\n");
        init_repository(&supporting_source, "supporting.txt", "supporting source\n");
        let repositories = vec![
            DevelopmentSourceRepositoryBinding {
                role: DevelopmentRepositoryRole::Primary,
                workspace_id: path_to_string(&primary_source).expect("primary source path"),
                worktree_id: None,
            },
            DevelopmentSourceRepositoryBinding {
                role: DevelopmentRepositoryRole::Supporting,
                workspace_id: path_to_string(&supporting_source).expect("supporting source path"),
                worktree_id: None,
            },
        ];
        let publication_parent = fs::canonicalize(&root)
            .expect("canonical test root")
            .join("slice-root/development/slice-1");
        let stale_scratch = publication_parent.join(SLICE_DEVELOPMENT_EXPORT_SCRATCH);
        fs::create_dir_all(stale_scratch.join("staging")).expect("create stale export scratch");
        fs::write(stale_scratch.join("development.tar.gz"), b"stale plaintext")
            .expect("write stale export archive");

        let publication = materialize_slice_development_publication(
            &publication_parent,
            "project-1",
            &repositories,
            None,
        )
        .expect("materialize slice Project");
        assert!(!stale_scratch.exists());
        let receipt = recover_pruned_mutable_development_context_publication(
            &publication_parent,
            "project-1",
            &repositories,
        )
        .expect("recover publication")
        .expect("publication receipt");
        assert_eq!(receipt.repositories.len(), 2);
        let primary = receipt
            .repositories
            .iter()
            .find(|repository| repository.role == DevelopmentRepositoryRole::Primary)
            .expect("primary repository");
        let supporting = receipt
            .repositories
            .iter()
            .find(|repository| repository.role == DevelopmentRepositoryRole::Supporting)
            .expect("supporting repository");
        assert_eq!(
            fs::read_to_string(primary.destination_path.join("primary.txt"))
                .expect("read primary repository"),
            "primary source\n"
        );
        assert_eq!(
            fs::read_to_string(supporting.destination_path.join("supporting.txt"))
                .expect("read supporting repository"),
            "supporting source\n"
        );

        fs::write(
            primary.destination_path.join("primary.txt"),
            "preserved slice edit\n",
        )
        .expect("edit materialized primary repository");
        git(&primary.destination_path, &["add", "primary.txt"]);
        git(&primary.destination_path, &["commit", "-m", "slice edit"]);
        let recovered = materialize_slice_development_publication(
            &publication_parent,
            "project-1",
            &repositories,
            Some(&publication),
        )
        .expect("recover slice Project after restart");
        assert_eq!(recovered, publication);
        assert_eq!(
            fs::read_to_string(primary.destination_path.join("primary.txt"))
                .expect("read recovered primary repository"),
            "preserved slice edit\n"
        );

        fs::create_dir_all(stale_scratch.join("staging"))
            .expect("recreate stale export scratch before delete");
        fs::write(stale_scratch.join("development.tar.gz"), b"stale plaintext")
            .expect("rewrite stale export archive");
        fs::remove_dir_all(&supporting.destination_path)
            .expect("simulate agent removing a supporting repository");
        cleanup_slice_development_publication(&publication_parent, "project-1", &repositories)
            .expect("clean exact receipted publication");
        assert!(!publication_parent.exists());
        fs::remove_dir_all(root).expect("remove test root");
    }

    #[test]
    fn slice_development_uses_its_durable_storage_root_after_config_changes() {
        let store = crate::slice::SliceStore::default();
        let slice = store
            .create(
                "kernel-1",
                "machine-1",
                crate::slice::CreateSliceInput {
                    name: "project-slice".to_string(),
                    backend: crate::slice::SliceBackendKind::SshDocker,
                    os: "linux".to_string(),
                    display_mode: crate::slice::SliceDisplayMode::Headless,
                    workspace_id: Some("/primary".to_string()),
                    worktree_id: Some("/primary-worktree".to_string()),
                    workspace_mount: Some("/primary-worktree".to_string()),
                    development: None,
                    worker_kernel_ref: None,
                    display_url: None,
                    provider_auth: Vec::new(),
                    from_saved_state: None,
                    now_ms: 1,
                },
            )
            .expect("create slice record");
        let slice = store
            .set_development_storage_root(
                &slice.id,
                "/old-config-root/development/slice-1".to_string(),
                2,
            )
            .expect("persist storage root");
        let slice = store
            .set_development_publication(
                &slice.id,
                crate::slice::SliceDevelopmentPublication {
                    publication_id: SLICE_DEVELOPMENT_PUBLICATION_ID.to_string(),
                    destination_root: "/old-config-root/development/slice-1/development"
                        .to_string(),
                    primary_repository_path:
                        "/old-config-root/development/slice-1/development/primary".to_string(),
                    repository_paths: vec![
                        "/old-config-root/development/slice-1/development/primary".to_string(),
                    ],
                },
                3,
            )
            .expect("persist publication");

        assert_eq!(
            slice_development_storage_root(&slice).expect("resolve durable root"),
            PathBuf::from("/old-config-root/development/slice-1")
        );
    }

    fn init_repository(path: &Path, file: &str, contents: &str) {
        fs::create_dir_all(path).expect("create repository");
        git(path, &["init", "-b", "main"]);
        git(path, &["config", "user.email", "tests@chariox.local"]);
        git(path, &["config", "user.name", "Chariox Tests"]);
        fs::write(path.join(file), contents).expect("write repository file");
        git(path, &["add", file]);
        git(path, &["commit", "-m", "initial"]);
    }

    fn git(path: &Path, args: &[&str]) {
        let output = Command::new("git")
            .args(args)
            .current_dir(path)
            .output()
            .expect("run git");
        assert!(
            output.status.success(),
            "git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn test_root(label: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        root_path(label, nonce)
    }

    fn root_path(label: &str, nonce: u128) -> PathBuf {
        std::env::temp_dir().join(format!("chariox-{label}-{}-{nonce}", std::process::id()))
    }
}
