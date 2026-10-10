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
}
fn protected_path(path: &str) -> bool {
    !safe_metadata(path)
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
        let entry = entry.map_err(|_| environment_error("selected folder entry unavailable"))?;
        let name = entry
            .file_name()
            .into_string()
            .map_err(|_| environment_error("selected File path encoding unsupported"))?;
        if !safe_metadata(&name) {
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
fn ignored(root: &Path, path: &str) -> Option<bool> {
    let status = std::process::Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["check-ignore", "--quiet", "--no-index", "--", path])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .ok()?;
    match status.code() {
        Some(0) => Some(true),
        Some(1) => Some(false),
        _ => None,
    }
}
pub(crate) fn verify_manual_files(
    current: &ProjectEnvironment,
    draft: &mut EnvironmentRevisionDraft,
) -> Result<(), DaemonError> {
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
        let folder = current
            .folders
            .iter()
            .find(|f| &f.folder_id == folder_id && !f.local_workspace_binding.is_empty())
            .ok_or_else(|| environment_error("selected File folder is no longer attached"))?;
        let path = Path::new(&folder.local_workspace_binding);
        let root = EvidenceRoot::open(path)?;
        for entry in entries.iter_mut() {
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
            entry.git_ignored = ignored(path, &entry.relative_path);
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
