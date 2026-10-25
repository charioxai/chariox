//! MP-08 / MP-10 / MP-11: one source fence spans every workspace in a Project.
use super::*;

#[derive(Debug, PartialEq, Eq)]
pub(super) struct SourceSnapshot {
    #[cfg(unix)]
    identity: (u64, u64),
    git: Option<export::RepositorySourceState>,
    directories: Vec<String>,
    overlay: Vec<DevelopmentOverlayEntry>,
}

impl SourceSnapshot {
    pub(super) fn capture(
        worktree: &Path,
        budget: &mut ManifestMemoryBudget,
        vault: Option<&vault_artifacts::ExportVault<'_>>,
    ) -> Result<Self, DaemonError> {
        let metadata = fs::symlink_metadata(worktree)
            .map_err(|error| context_io_error("inspect source snapshot root", error))?;
        if !metadata.is_dir() || metadata.file_type().is_symlink() {
            return Err(source_changed());
        }
        #[cfg(unix)]
        let identity = {
            use std::os::unix::fs::MetadataExt;
            (metadata.dev(), metadata.ino())
        };
        let (git, directories, overlay) = match fs::symlink_metadata(worktree.join(".git")) {
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                let root = directory::open_root(worktree)?;
                let (entries, directories, _, _) =
                    directory::snapshot(worktree, &root, "snapshot", worktree, budget, vault)?;
                (None, directories, entries)
            }
            Err(error) => return Err(context_io_error("inspect source Git metadata", error)),
            Ok(_) => {
                ensure_worktree_root(worktree)?;
                let before = export::repository_source_state(worktree)?;
                budget.consume(
                    before.head_sha.len()
                        + before.branch.as_ref().map_or(0, String::len)
                        + before.upstream.as_ref().map_or(0, String::len)
                        + before.origin_url.as_ref().map_or(0, String::len)
                        + 256,
                )?;
                let (entries, _) = overlay::verify_overlay(
                    worktree,
                    "snapshot",
                    worktree,
                    budget,
                    vault.is_some(),
                )?;
                if before != export::repository_source_state(worktree)? {
                    return Err(source_changed());
                }
                (Some(before), Vec::new(), entries)
            }
        };
        Ok(Self {
            #[cfg(unix)]
            identity,
            git,
            directories,
            overlay,
        })
    }

    pub(super) fn verify_exported(
        &self,
        repository: &DevelopmentRepositoryManifest,
    ) -> Result<(), DaemonError> {
        let git_matches = match &self.git {
            Some(git) => {
                repository.workspace_kind.is_git()
                    && git.head_sha == repository.head_sha
                    && git.branch == repository.branch
                    && git.upstream.as_ref().filter(|upstream| {
                        git.origin_url.is_some()
                            && upstream
                                .strip_prefix("origin/")
                                .is_some_and(|branch| !branch.is_empty())
                    }) == repository.upstream.as_ref()
                    && git.origin_url == repository.origin_url
            }
            None => !repository.workspace_kind.is_git(),
        };
        if !git_matches
            || self.directories != repository.directories
            || self.overlay.len() != repository.overlay.len()
            || !self.overlay.iter().zip(&repository.overlay).all(|(a, b)| {
                a.path == b.path
                    && same_file(&a.index, &b.index)
                    && same_file(&a.worktree, &b.worktree)
            })
        {
            return Err(source_changed());
        }
        Ok(())
    }
}

// Object storage paths include an export-local repository ID; source identity
// depends on the bytes, sizes, modes and workspace paths rather than that ID.
fn same_file(a: &DevelopmentFileState, b: &DevelopmentFileState) -> bool {
    match (a, b) {
        (DevelopmentFileState::Absent, DevelopmentFileState::Absent) => true,
        (
            DevelopmentFileState::File {
                sha256: a,
                size_bytes: a_size,
                executable: a_mode,
                ..
            },
            DevelopmentFileState::File {
                sha256: b,
                size_bytes: b_size,
                executable: b_mode,
                ..
            },
        ) => a == b && a_size == b_size && a_mode == b_mode,
        _ => false,
    }
}

pub(super) fn source_changed() -> DaemonError {
    context_error("source Project changed while exporting; stop workspace writes and retry")
}
