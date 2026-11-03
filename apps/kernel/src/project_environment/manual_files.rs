//! MP-08 / MP-10 / MP-11: owner-bound manual Files, streamed digest and protected verdict.
use super::detect_index::{credential_content, safe_metadata, EvidenceRoot};
use super::*;
use crate::error::DaemonError;
use sha2::{Digest, Sha256};
use std::io::Read;
use std::path::Path;
use zeroize::Zeroizing;

const MAX_ENTRIES: usize = 20_000;
const MAX_BYTES: u64 = 8 * 1024 * 1024 * 1024;
struct Selection {
    digest: Sha256,
    bytes: u64,
    count: usize,
    protected: bool,
    deadline: Option<std::time::Instant>,
}
impl Selection {
    fn check_budget(&self) -> Result<(), DaemonError> {
        if self
            .deadline
            .is_some_and(|deadline| std::time::Instant::now() >= deadline)
        {
            return Err(environment_error("Check file measurement budget exceeded"));
        }
        Ok(())
    }
}
fn protected_path(path: &str) -> bool {
    !safe_metadata(path)
        || super::detect_index::credential_metadata(path)
        || secret_looking_project_path(path)
        || path.split('/').any(|p| {
            matches!(
                p,
                ".git" | ".ssh" | ".codex" | ".claude" | ".chariox" | "auth.json"
            )
        })
}
fn stream(
    root: &EvidenceRoot,
    path: &str,
    selection: &mut Selection,
) -> Result<Option<String>, DaemonError> {
    selection.check_budget()?;
    if protected_path(path) {
        selection.protected = true;
        return Ok(None);
    }
    let mut file = root.file(path)?;
    let before = file
        .metadata()
        .map_err(|_| environment_error("File metadata unavailable"))?;
    if selection.bytes.saturating_add(before.len()) > MAX_BYTES {
        return Err(environment_error(
            "selected Files exceed the 8 GiB review capacity",
        ));
    }
    selection.count += 1;
    if selection.count > MAX_ENTRIES {
        return Err(environment_error(
            "selected folder exceeds File count capacity",
        ));
    }
    let mut digest = Sha256::new();
    let mut buffer = Zeroizing::new(vec![0u8; 64 * 1024]);
    let mut overlap = Zeroizing::new(Vec::<u8>::new());
    let mut bytes = 0u64;
    loop {
        selection.check_budget()?;
        #[cfg(test)]
        if let Some(marker) = std::env::var_os("CHARIOX_FILES_CHECK_IO_STALL_MARKER") {
            // MP-08 / MP-10 / MP-11: owned process supervision bounds an admitted I/O stall.
            #[cfg(target_os = "linux")]
            assert_eq!(unsafe { libc::prctl(libc::PR_GET_DUMPABLE, 0, 0, 0, 0) }, 0);
            std::fs::write(marker, std::process::id().to_string()).unwrap();
            std::thread::sleep(std::time::Duration::from_secs(2));
        }
        let n = file
            .read(&mut buffer)
            .map_err(|_| environment_error("selected File read failed"))?;
        if n == 0 {
            break;
        }
        bytes += n as u64;
        if selection.bytes.saturating_add(bytes) > MAX_BYTES {
            return Err(environment_error(
                "selected Files exceed the 8 GiB review capacity",
            ));
        }
        overlap.extend_from_slice(&buffer[..n]);
        // Inspect every byte, including non-UTF8 assets and patterns across chunk boundaries.
        let text = Zeroizing::new(String::from_utf8_lossy(&overlap).into_owned());
        if credential_content(&text) {
            selection.protected = true;
        }
        let retain = overlap.len().min(8192);
        let discard = overlap.len() - retain;
        overlap.drain(..discard);
        digest.update(&buffer[..n]);
    }
    let after = file
        .metadata()
        .map_err(|_| environment_error("File metadata unavailable"))?;
    if bytes != before.len()
        || after.len() != before.len()
        || after.modified().ok() != before.modified().ok()
    {
        return Err(environment_error(
            "selected File changed during review; retry Save",
        ));
    }
    selection.bytes += bytes;
    selection.digest.update((path.len() as u64).to_be_bytes());
    selection.digest.update(path.as_bytes());
    selection.digest.update(bytes.to_be_bytes());
    let hash = digest.finalize();
    selection.digest.update(hash);
    Ok(Some(format!("{hash:x}")))
}
fn walk(
    root: &EvidenceRoot,
    path: &str,
    depth: usize,
    selection: &mut Selection,
) -> Result<(), DaemonError> {
    if depth > 32 {
        return Err(environment_error("selected folder exceeds depth capacity"));
    }
    selection.check_budget()?;
    if protected_path(path) {
        selection.protected = true;
        return Ok(());
    }
    let (_anchor, directory) = root.directory(path)?;
    let mut names = Vec::new();
    for entry in std::fs::read_dir(directory)
        .map_err(|_| environment_error("selected folder unavailable"))?
        .take(MAX_ENTRIES + 1)
    {
        selection.check_budget()?;
        let entry = entry.map_err(|_| environment_error("selected folder entry unavailable"))?;
        let name = entry
            .file_name()
            .into_string()
            .map_err(|_| environment_error("selected File path encoding unsupported"))?;
        if !safe_metadata(&name) || super::detect_index::credential_metadata(&name) {
            selection.protected = true;
            continue;
        }
        let kind = entry
            .file_type()
            .map_err(|_| environment_error("selected File type unavailable"))?;
        if kind.is_symlink() || (!kind.is_file() && !kind.is_dir()) {
            return Err(environment_error(
                "selected folder contains symlink or non-regular File",
            ));
        }
        names.push((name, kind.is_dir()));
        if names.len() + selection.count > MAX_ENTRIES {
            return Err(environment_error(
                "selected folder exceeds File count capacity",
            ));
        }
    }
    names.sort();
    selection.count += 1;
    selection.digest.update(b"folder\0");
    selection.digest.update(path.as_bytes());
    selection.digest.update(b"\0");
    for (name, is_dir) in names {
        let child = format!("{path}/{name}");
        if is_dir {
            walk(root, &child, depth + 1, selection)?;
        } else {
            let _ = stream(root, &child, selection)?;
        }
    }
    Ok(())
}
fn ignored(root: &Path, path: &str, roots: &[std::path::PathBuf]) -> Option<bool> {
    let executable = super::check_inventory::executable("git")?;
    let mut command = super::check_executable::command(&executable, roots).ok()?;
    command
        .arg("-C")
        .arg(root)
        .args(["check-ignore", "--quiet", "--no-index", "--", path])
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null");
    let output = crate::local::provider_requests::capture_inventory_probe(
        command,
        // Optional metadata must leave time for file validation and owned cleanup.
        std::time::Duration::from_millis(100),
    )
    .ok()?;
    let _stdout = Zeroizing::new(output.stdout);
    let _stderr = Zeroizing::new(output.stderr);
    match output.status.code() {
        Some(0) => Some(true),
        Some(1) => Some(false),
        _ => None,
    }
}
pub(crate) fn verify_manual_files(
    current: &ProjectEnvironment,
    draft: &mut EnvironmentRevisionDraft,
) -> Result<(), DaemonError> {
    verify_files(current, draft, true, None)
}
pub(super) fn verify_manual_files_until(
    current: &ProjectEnvironment,
    draft: &mut EnvironmentRevisionDraft,
    deadline: std::time::Instant,
) -> Result<(), DaemonError> {
    verify_files(current, draft, true, Some(deadline))
}
pub(super) fn has_included_files(draft: &EnvironmentRevisionDraft) -> bool {
    draft.project_requirements.iter().chain(draft.folders.iter().flat_map(|f| &f.requirements)).any(|r|
        matches!(&r.spec, RequirementSpec::Files { entries, .. } if entries.iter().any(|e| e.transfer_inclusion != EnvironmentTransferInclusion::Exclude)))
}
// MP-08 / MP-10 / MP-11: Check measures protected content without Save-only Git metadata.
pub(super) fn verify_check_files(
    current: &ProjectEnvironment,
    draft: &mut EnvironmentRevisionDraft,
    deadline: std::time::Instant,
) -> Result<(), DaemonError> {
    verify_files(current, draft, false, Some(deadline))
}
fn verify_files(
    current: &ProjectEnvironment,
    draft: &mut EnvironmentRevisionDraft,
    git_metadata: bool,
    deadline: Option<std::time::Instant>,
) -> Result<(), DaemonError> {
    let mut roots: Option<Vec<std::path::PathBuf>> = None;
    let mut total_bytes = 0;
    let mut total_count = 0;
    for requirement in draft
        .project_requirements
        .iter_mut()
        .chain(draft.folders.iter_mut().flat_map(|f| &mut f.requirements))
    {
        let RequirementSpec::Files { folder_id, entries } = &mut requirement.spec else {
            continue;
        };
        // MP-08 / MP-10 / MP-11: clients may exclude absent paths, but
        // cannot manufacture or edit kernel measurements while doing so.
        for entry in entries
            .iter_mut()
            .filter(|entry| entry.transfer_inclusion == EnvironmentTransferInclusion::Exclude)
        {
            let measured = current
                .project_requirements
                .iter()
                .chain(
                    current
                        .folders
                        .iter()
                        .flat_map(|folder| &folder.requirements),
                )
                .chain(
                    current
                        .proposals
                        .iter()
                        .map(|proposal| &proposal.requirement),
                )
                .filter(|saved| saved.requirement_id == requirement.requirement_id)
                .find_map(|saved| match &saved.spec {
                    RequirementSpec::Files {
                        folder_id: saved_folder,
                        entries: saved_entries,
                    } if saved_folder == folder_id => saved_entries.iter().find(|saved| {
                        saved.relative_path == entry.relative_path && saved.kind == entry.kind
                    }),
                    _ => None,
                });
            entry.content_digest = measured.and_then(|saved| saved.content_digest.clone());
            entry.byte_count = measured.and_then(|saved| saved.byte_count);
            entry.git_ignored = measured.and_then(|saved| saved.git_ignored);
            entry.secret_looking = measured.is_some_and(|saved| saved.secret_looking);
            entry.credential_filter_verdict = measured
                .map(|saved| saved.credential_filter_verdict.clone())
                .unwrap_or(EnvironmentCredentialFilterVerdict::NotChecked);
        }
        // MP-08 / MP-10 / MP-11: exclusion is saved metadata, including
        // imported selections deliberately absent from this machine.
        if entries
            .iter()
            .all(|entry| entry.transfer_inclusion == EnvironmentTransferInclusion::Exclude)
        {
            continue;
        }
        // MP-08 / MP-10 / MP-11: a detached folder cannot be rescanned, but
        // an unchanged saved Files selection remains valid revision metadata.
        let attached = current
            .folders
            .iter()
            .any(|f| &f.folder_id == folder_id && !f.local_workspace_binding.is_empty());
        if !attached && current.project_requirements.iter()
            .chain(current.folders.iter().flat_map(|f| &f.requirements))
            .any(|r| r.requirement_id == requirement.requirement_id
                && r.scope == requirement.scope
                && matches!(&r.spec, RequirementSpec::Files { folder_id: old_folder, entries: old_entries } if old_folder == folder_id && old_entries == entries)) {
            continue;
        }
        let folder = current
            .folders
            .iter()
            .find(|f| &f.folder_id == folder_id && !f.local_workspace_binding.is_empty())
            .ok_or_else(|| environment_error("selected File folder is no longer attached"))?;
        let path = Path::new(&folder.local_workspace_binding);
        let root = EvidenceRoot::open(path)?;
        for entry in entries.iter_mut() {
            if entry.transfer_inclusion == EnvironmentTransferInclusion::Exclude {
                continue;
            }
            let unchanged_detected = !entry.user_selected && current.project_requirements.iter().chain(current.folders.iter().flat_map(|f| &f.requirements)).chain(current.proposals.iter().map(|p| &p.requirement)).any(|r| r.requirement_id == requirement.requirement_id && matches!(&r.spec,RequirementSpec::Files{entries:old,..} if old.contains(entry)));
            if unchanged_detected {
                continue;
            }
            entry.user_selected = true;
            let mut selection = Selection {
                digest: Sha256::new(),
                bytes: total_bytes,
                count: total_count,
                protected: false,
                deadline,
            };
            let file_digest = match entry.kind {
                EnvironmentFileKind::File => stream(&root, &entry.relative_path, &mut selection)?,
                EnvironmentFileKind::Folder => {
                    walk(&root, &entry.relative_path, 0, &mut selection)?;
                    None
                }
            };
            let bytes = selection.bytes - total_bytes;
            total_bytes = selection.bytes;
            total_count = selection.count;
            if git_metadata {
                let roots = roots.get_or_insert_with(|| {
                    current
                        .folders
                        .iter()
                        .filter(|folder| !folder.local_workspace_binding.is_empty())
                        .filter_map(|folder| {
                            #[cfg(test)]
                            if let Some(marker) =
                                std::env::var_os("CHARIOX_MANUAL_ROOT_STALL_MARKER")
                            {
                                std::fs::write(marker, std::process::id().to_string()).unwrap();
                                std::thread::sleep(std::time::Duration::from_secs(2));
                            }
                            Path::new(&folder.local_workspace_binding)
                                .canonicalize()
                                .ok()
                        })
                        .collect()
                });
                entry.git_ignored = ignored(path, &entry.relative_path, roots);
            }
            entry.secret_looking = selection.protected;
            if selection.protected {
                entry.content_digest = None;
                entry.byte_count = None;
                entry.credential_filter_verdict = EnvironmentCredentialFilterVerdict::NeedsVault;
                entry.transfer_inclusion = EnvironmentTransferInclusion::Exclude;
                entry.reason = Some(
                    "Plaintext blocked · choose a Vault Secret or exclude this selection".into(),
                );
            } else {
                entry.content_digest = Some(
                    file_digest.unwrap_or_else(|| format!("{:x}", selection.digest.finalize())),
                );
                entry.byte_count = Some(bytes);
                entry.credential_filter_verdict = EnvironmentCredentialFilterVerdict::Clear;
                entry.reason=Some("Manual selection · credentials scanned by the owning kernel; instruction activation is separate".into());
            }
        }
    }
    Ok(())
}

// Worker metadata carries only these safe validation outcomes, never arbitrary error text.
#[derive(serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum ManualFilesFailure {
    ByteCapacity,
    FileCountCapacity,
    DepthCapacity,
    ChangedDuringReview,
    FolderDetached,
    SymlinkOrNonRegular,
    FolderUnavailable,
    FileUnavailable,
    UnsupportedEncoding,
    BudgetExceeded,
    InvalidSelection,
}
impl ManualFilesFailure {
    pub(super) fn from_error(error: &DaemonError) -> Self {
        let DaemonError::LocalTransport {
            operation: "Project environment",
            message,
        } = error
        else {
            return Self::InvalidSelection;
        };
        match message.as_str() {
            "selected Files exceed the 8 GiB review capacity" => Self::ByteCapacity,
            "selected folder exceeds File count capacity" => Self::FileCountCapacity,
            "selected folder exceeds depth capacity" => Self::DepthCapacity,
            "selected File changed during review; retry Save" => Self::ChangedDuringReview,
            "selected File folder is no longer attached" => Self::FolderDetached,
            "selected folder contains symlink or non-regular File" => Self::SymlinkOrNonRegular,
            "selected folder unavailable" | "selected folder entry unavailable" => {
                Self::FolderUnavailable
            }
            "File metadata unavailable"
            | "selected File read failed"
            | "selected File type unavailable" => Self::FileUnavailable,
            "selected File path encoding unsupported" => Self::UnsupportedEncoding,
            "Check file measurement budget exceeded" => Self::BudgetExceeded,
            _ => Self::InvalidSelection,
        }
    }
    pub(super) fn error(self) -> DaemonError {
        environment_error(match self {
            Self::ByteCapacity => "selected Files exceed the 8 GiB review capacity",
            Self::FileCountCapacity => "selected folder exceeds File count capacity",
            Self::DepthCapacity => "selected folder exceeds depth capacity",
            Self::ChangedDuringReview => "selected File changed during review; retry Save",
            Self::FolderDetached => "selected File folder is no longer attached",
            Self::SymlinkOrNonRegular => "selected folder contains symlink or non-regular File",
            Self::FolderUnavailable => "selected folder unavailable; check the selected path and retry Save",
            Self::FileUnavailable => "selected File unavailable; check the selected path and permissions, then retry Save",
            Self::UnsupportedEncoding => "selected File path encoding unsupported",
            Self::BudgetExceeded => "Check file measurement budget exceeded; select fewer Files and retry Save",
            Self::InvalidSelection => "selected Files could not be validated; check paths and permissions, then retry Save",
        })
    }
}
