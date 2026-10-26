//! The existing Project exporter/importer owns file selection and publication.
use super::*;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;

use crate::error::DaemonError;
use crate::managed_context::development::{
    DevelopmentContextExportRequest, DevelopmentContextImportRequest,
    DevelopmentRepositorySelection,
};
use crate::provider::ProviderNativeCheckpoint;
use sha2::{Digest, Sha256};

pub const MOVE_CHUNK_BYTES: usize = 128 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentMoveArtifactKind {
    Workspace,
    Native,
}

impl AgentMoveArtifactKind {
    fn filename(self) -> &'static str {
        match self {
            Self::Workspace => "workspace.archive",
            Self::Native => "native.checkpoint",
        }
    }
    fn descriptor(self, capture: &AgentMoveCapture) -> Result<&AgentMoveArtifact, DaemonError> {
        match self {
            Self::Workspace => Ok(&capture.workspace),
            Self::Native => capture
                .native
                .as_ref()
                .ok_or_else(|| error("Move has no native checkpoint")),
        }
    }
}

impl AgentMoveStore {
    pub(crate) fn artifact_path(&self, operation: &str, kind: AgentMoveArtifactKind) -> PathBuf {
        self.artifact_root(operation).join(kind.filename())
    }

    pub(crate) fn capture_source(
        &self,
        source: &MoveSource,
        project_id: String,
        repositories: Vec<DevelopmentRepositorySelection>,
        checkpoint: Option<&ProviderNativeCheckpoint>,
        config: &crate::config::DaemonConfig,
        environment: crate::managed_context::development::DevelopmentProjectEnvironment,
    ) -> Result<AgentMoveCapture, DaemonError> {
        if !source.stopped {
            return Err(error(
                "Confirm source provider teardown before capturing the Project",
            ));
        }
        if let Some(capture) = &source.capture {
            return Ok(capture.clone());
        }
        let root = self.artifact_root(&source.identity.operation_id);
        fs::create_dir_all(&root).map_err(|_| {
            error("Cannot stage move artifacts; check free space and state permissions")
        })?;
        let workspace = root.join(AgentMoveArtifactKind::Workspace.filename());
        // No capture receipt exists yet. Only this operation's unpublished archive
        // can be discarded after a crash between export and journal publication.
        if workspace
            .try_exists()
            .map_err(|_| error("Cannot inspect move staging"))?
        {
            fs::remove_file(&workspace)
                .map_err(|_| error("Cannot remove unfinished move archive"))?;
        }
        let source_repositories = repositories
            .iter()
            .map(|selection| DevelopmentSourceRepositoryBinding {
                role: selection.role,
                workspace_id: selection.workspace_id.clone(),
                worktree_id: selection.worktree_id.clone(),
            })
            .collect();
        let archive_project_id = environment.sealed.manifest.project_id.clone();
        let vault = crate::managed_context::development::vault_artifacts::ExportVault::new(
            config,
            &source.identity.operation_id,
            &source.identity.target_kernel_id,
            &source.identity.target_public_key,
            &archive_project_id,
        );
        let exported = crate::managed_context::development::export_development_context_with_vault(
            DevelopmentContextExportRequest {
                project_id: archive_project_id.clone(),
                repositories,
                archive_path: workspace,
            },
            Some(environment),
            Some(&vault),
        )?;
        let native = if let Some(checkpoint) = checkpoint {
            let path = root.join(AgentMoveArtifactKind::Native.filename());
            super::store::atomic_save(&path, checkpoint)?;
            Some(hash_file(&path)?)
        } else {
            None
        };
        let capture = AgentMoveCapture {
            identity: source.identity.clone(),
            source_project_id: project_id,
            archive_project_id,
            source_provider_run_id: source.provider_run_id.clone(),
            source_repositories,
            workspace: AgentMoveArtifact {
                sha256: exported.archive_sha256,
                size_bytes: exported.archive_size_bytes,
            },
            native,
            checkpoint_binding: checkpoint.map(|checkpoint| checkpoint.binding().clone()),
        };
        let mut source = source.clone();
        source.capture = Some(capture.clone());
        self.save_source(source)?;
        Ok(capture)
    }

    pub(crate) fn read_chunk(
        &self,
        capture: &AgentMoveCapture,
        kind: AgentMoveArtifactKind,
        offset: u64,
    ) -> Result<Vec<u8>, DaemonError> {
        let descriptor = kind.descriptor(capture)?;
        if offset > descriptor.size_bytes {
            return Err(error("Move artifact offset exceeds its declared size"));
        }
        let mut file = File::open(self.artifact_path(&capture.identity.operation_id, kind))
            .map_err(|_| {
                error("Move source artifact is unavailable; restore it before retrying")
            })?;
        file.seek(SeekFrom::Start(offset))
            .map_err(|_| error("Cannot read move source artifact"))?;
        let count = (descriptor.size_bytes - offset).min(MOVE_CHUNK_BYTES as u64) as usize;
        let mut bytes = vec![0; count];
        file.read_exact(&mut bytes)
            .map_err(|_| error("Move source artifact changed; restore the original capture"))?;
        Ok(bytes)
    }

    pub(crate) fn begin_target(
        &self,
        capture: AgentMoveCapture,
        workspace_parent: &Path,
    ) -> Result<MoveTarget, DaemonError> {
        if let Some(target) = self.target(&capture.identity.operation_id) {
            if target.retired {
                return Err(error(
                    "This Move target was cancelled; start a new generation",
                ));
            }
            if target.capture != capture {
                return Err(error(
                    "Move generation already has a different target capture",
                ));
            }
            return Ok(target);
        }
        if capture.native.is_some() != capture.checkpoint_binding.is_some()
            || capture.workspace.sha256.len() != 64
            || capture.native.as_ref().is_some_and(|native| {
                native.sha256.len() != 64
                    || native.size_bytes
                        > (crate::provider::MAX_NATIVE_CHECKPOINT_BYTES as u64)
                            .saturating_mul(5)
                            .saturating_add(65536)
            })
        {
            return Err(error("Move capture descriptor is invalid"));
        }
        let required = capture
            .workspace
            .size_bytes
            .checked_add(
                capture
                    .native
                    .as_ref()
                    .map_or(0, |native| native.size_bytes),
            )
            .ok_or_else(|| {
                error("Move artifact sizes overflow available space; recapture the Project")
            })?;
        let available = crate::managed_context::limits::disk_budget(
            &self.artifact_root(&capture.identity.operation_id),
        );
        if required > available {
            return Err(error(format!("Move artifact staging requires {required} bytes but only {available} bytes of free space are available; free disk space and retry")));
        }
        fs::create_dir_all(workspace_parent).map_err(|_| {
            error("Cannot create the move workspace parent; choose a writable destination")
        })?;
        let parent = fs::canonicalize(workspace_parent)
            .map_err(|_| error("Cannot resolve the move workspace parent"))?;
        let destination_root = parent.join(format!(
            "move-{:x}",
            Sha256::digest(capture.identity.operation_id.as_bytes())
        ));
        if fs::symlink_metadata(&destination_root).is_ok() {
            return Err(error(
                "Move destination is occupied. Preserve that checkout and choose an unused root",
            ));
        }
        let target = MoveTarget {
            capture,
            destination_root,
            receipt: None,
            execution_lease_id: None,
            backing_session_id: None,
            backing_agent_id: None,
            active: false,
            ready: false,
            target_account_profile: None,
            backing: None,
            retired: false,
            project: None,
        };
        fs::create_dir_all(self.artifact_root(&target.capture.identity.operation_id))
            .map_err(|_| error("Cannot create move target staging; check free space"))?;
        self.save_target(target.clone())?;
        Ok(target)
    }

    pub(crate) fn write_chunk(
        &self,
        operation: &str,
        kind: AgentMoveArtifactKind,
        offset: u64,
        bytes: &[u8],
    ) -> Result<(), DaemonError> {
        let target = self
            .target(operation)
            .ok_or_else(|| error("Prepare the move target before uploading artifacts"))?;
        if target.retired {
            return Err(error(
                "This Move target was cancelled; start a new generation",
            ));
        }
        let descriptor = kind.descriptor(&target.capture)?;
        let end = offset
            .checked_add(bytes.len() as u64)
            .ok_or_else(|| error("Move artifact offset overflow"))?;
        if bytes.len() > MOVE_CHUNK_BYTES || end > descriptor.size_bytes {
            return Err(error("Move artifact chunk exceeds its declared bounds"));
        }
        let path = self.artifact_path(operation, kind);
        let mut options = OpenOptions::new();
        options.read(true).write(true).create(true).truncate(false);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
        }
        let mut file = options.open(path).map_err(|_| {
            error("Cannot stage move chunk; check free space and state permissions")
        })?;
        let mut length = file
            .metadata()
            .map_err(|_| error("Cannot inspect move staging"))?
            .len();
        if offset < length {
            if end <= length {
                let mut existing = vec![0; bytes.len()];
                file.seek(SeekFrom::Start(offset))
                    .and_then(|_| file.read_exact(&mut existing))
                    .map_err(|_| error("Cannot verify a retried move chunk"))?;
                if existing == bytes {
                    file.sync_all()
                        .map_err(|_| error("Cannot sync move artifact; check free space"))?;
                    return Ok(());
                }
            }
            if target.receipt.is_some() {
                return Err(error("A published Move artifact differs from its capture; restore the original generation"));
            }
            // A short write or crash may leave an unacknowledged suffix, including
            // zero-filled bytes. Rewind it; publication still verifies the full digest.
            file.set_len(offset)
                .map_err(|_| error("Cannot repair interrupted move staging; check free space"))?;
            length = offset;
        }
        if target.receipt.is_some() || offset != length {
            return Err(error(
                "Move chunk is out of order or its target is already published",
            ));
        }
        let available = crate::managed_context::limits::disk_budget(&self.artifact_root(operation));
        if bytes.len() as u64 > available {
            return Err(error(
                "Move chunk exceeds available staging space; free disk space and retry this chunk",
            ));
        }
        file.seek(SeekFrom::Start(offset))
            .and_then(|_| file.write_all(bytes))
            .map_err(|_| error("Cannot write move chunk; check free space"))?;
        file.sync_all()
            .map_err(|_| error("Cannot sync move artifact; check free space"))?;
        Ok(())
    }

    pub(crate) fn native_checkpoint(
        &self,
        capture: &AgentMoveCapture,
    ) -> Result<Option<ProviderNativeCheckpoint>, DaemonError> {
        let Some(expected) = &capture.native else {
            return Ok(None);
        };
        let path = self.artifact_path(
            &capture.identity.operation_id,
            AgentMoveArtifactKind::Native,
        );
        if hash_file(&path)? != *expected {
            return Err(error(
                "Move native artifact failed size or digest verification",
            ));
        }
        let checkpoint: ProviderNativeCheckpoint = serde_json::from_reader(
            File::open(path).map_err(|_| error("Move native artifact is unavailable"))?,
        )
        .map_err(|_| error("Move native artifact is invalid; restore the original capture"))?;
        if Some(checkpoint.binding()) != capture.checkpoint_binding.as_ref() {
            return Err(error(
                "Move native checkpoint binding differs from its capture",
            ));
        }
        checkpoint
            .validate()
            .map_err(crate::provider::NativeCheckpointFailure::into_daemon_error)?;
        Ok(Some(checkpoint))
    }

    pub(crate) fn publish_target(
        &self,
        operation: &str,
        config: &crate::config::DaemonConfig,
    ) -> Result<AgentMoveMaterialization, DaemonError> {
        let mut target = self
            .target(operation)
            .ok_or_else(|| error("Move target is not prepared"))?;
        if target.retired {
            return Err(error(
                "This Move target was cancelled; start a new generation",
            ));
        }
        if let Some(receipt) = target.receipt {
            return Ok(receipt);
        }
        self.native_checkpoint(&target.capture)?;
        let archive_path = self.artifact_path(operation, AgentMoveArtifactKind::Workspace);
        if hash_file(&archive_path)? != target.capture.workspace {
            return Err(error(
                "Move Project artifact failed size or digest verification",
            ));
        }
        let request = DevelopmentContextImportRequest {
            archive_path,
            expected_archive_sha256: target.capture.workspace.sha256.clone(),
            expected_project_id: target.capture.archive_project_id.clone(),
            expected_source_repositories: Some(target.capture.source_repositories.clone()),
            destination_root: target.destination_root.clone(),
        };
        let publication = format!("move-{:x}", Sha256::digest(operation.as_bytes()));
        let authority = crate::project_environment::ProjectEnvironmentImportAuthority {
            config: config.clone(),
            context_id: operation.into(),
            source_kernel_id: target.capture.identity.source_kernel_id.clone(),
            source_key_thumbprint: target.capture.identity.source_key_thumbprint.clone(),
            target_kernel_id: target.capture.identity.target_kernel_id.clone(),
        };
        let receipt = match crate::managed_context::development::recover_development_context_publication_with_environment(&request, &publication, Some(&authority))? {
            Some(receipt) => receipt,
            None => crate::managed_context::development::import_development_context_with_environment(request, publication, Some(&authority))?,
        };
        let materialized = AgentMoveMaterialization {
            identity: target.capture.identity.clone(),
            archive_sha256: receipt.archive_sha256,
            source_project_id: target.capture.source_project_id.clone(),
            target_project_id: receipt.project_id,
            destination_root: receipt.destination_root,
            primary_repository_id: receipt.primary_repository_id,
            repositories: receipt.repositories,
        };
        materialized.primary_path()?;
        target.receipt = Some(materialized.clone());
        self.save_target(target)?;
        Ok(materialized)
    }
}

pub(crate) fn hash_file(path: &Path) -> Result<AgentMoveArtifact, DaemonError> {
    let mut file = File::open(path)
        .map_err(|_| error("Move artifact is unavailable; restore the original capture"))?;
    let mut digest = Sha256::new();
    let mut size_bytes = 0_u64;
    let mut buffer = [0; 64 * 1024];
    loop {
        let count = file
            .read(&mut buffer)
            .map_err(|_| error("Cannot verify move artifact"))?;
        if count == 0 {
            break;
        }
        digest.update(&buffer[..count]);
        size_bytes = size_bytes
            .checked_add(count as u64)
            .ok_or_else(|| error("Move artifact size overflow"))?;
    }
    Ok(AgentMoveArtifact {
        sha256: format!("{:x}", digest.finalize()),
        size_bytes,
    })
}
