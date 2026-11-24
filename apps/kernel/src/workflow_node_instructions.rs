//! A workflow node's instruction file. Its agent rereads it after losing
//! context, so it lives in the kernel home beside, not inside, the kernel's
//! hidden state.
use crate::config::DaemonConfig;
use crate::session::RuntimeSession;
use std::collections::BTreeSet;
use std::io;
use std::path::{Component, Path};

/// Writes the node's instructions (or a placeholder the first time) and
/// returns the file's path, or `None` when it cannot be written.
pub(crate) fn node_instruction_file(
    config: &DaemonConfig,
    session_id: &str,
    workflow_run_id: &str,
    node_id: &str,
    instructions: Option<&str>,
) -> Option<String> {
    let root = config
        .workflow_instruction_root()
        .join(session_id)
        .join(workflow_run_id);
    let path = root.join(format!("node-{node_id}.md"));
    if !path.exists() || instructions.is_some() {
        if let Err(error) = std::fs::create_dir_all(&root) {
            tracing::debug!(
                ?error,
                "Failed to create workflow instruction directory at {:?}",
                root
            );
            return None;
        }
        let content = instructions.map(str::to_string).unwrap_or_else(|| {
            format!(
                "# Workflow Node Instructions\n\nThis file is daemon-managed. Update node instructions through workflow configuration tooling.\n\nNode: {node_id}\n"
            )
        });
        if let Err(error) = std::fs::write(&path, content) {
            tracing::debug!(
                ?error,
                "Failed to write workflow instruction file at {:?}",
                path
            );
            return None;
        }
    }
    Some(path.to_string_lossy().to_string())
}

pub(crate) fn remove_run(root: &Path, session_id: &str, run_id: &str) {
    if let Err(error) = remove_run_directory(root, session_id, run_id) {
        if error.kind() != io::ErrorKind::NotFound {
            tracing::warn!(session_id, run_id, %error, "Could not remove workflow instruction files");
        }
    }
}

/// A crash can happen after the terminal commit but before file deletion.
/// Keep the files referenced by resumable turns and sweep the rest at startup.
pub(crate) fn remove_inactive_runs(root: &Path, sessions: &[RuntimeSession]) {
    let active = sessions
        .iter()
        .flat_map(|session| {
            session
                .workflow_runs()
                .iter()
                .filter(|run| !run.status().is_terminal())
                .map(move |run| (session.id(), run.id()))
        })
        .collect::<BTreeSet<_>>();
    if let Err(error) = sweep_inactive_runs(root, &active) {
        if error.kind() != io::ErrorKind::NotFound {
            tracing::warn!(%error, "Could not sweep inactive workflow instruction files");
        }
    }
}

fn sweep_inactive_runs(root: &Path, active: &BTreeSet<(&str, &str)>) -> io::Result<()> {
    for session in std::fs::read_dir(root)? {
        let session = session?;
        if !session.file_type()?.is_dir() {
            continue;
        }
        let session_id = session.file_name();
        let Some(session_id) = session_id.to_str() else {
            continue;
        };
        for run in std::fs::read_dir(session.path())? {
            let run = run?;
            let run_id = run.file_name();
            if let Some(run_id) = run_id.to_str() {
                if run.file_type()?.is_dir() && !active.contains(&(session_id, run_id)) {
                    remove_run(root, session_id, run_id);
                }
            }
        }
    }
    Ok(())
}

fn check_component(name: &str) -> io::Result<()> {
    let mut components = Path::new(name).components();
    if matches!(components.next(), Some(Component::Normal(component)) if component == name)
        && components.next().is_none()
    {
        return Ok(());
    }
    Err(io::ErrorKind::InvalidInput.into())
}

#[cfg(unix)]
fn remove_run_directory(root: &Path, session_id: &str, run_id: &str) -> io::Result<()> {
    use rustix::fs::{open, openat, unlinkat, AtFlags, Dir, Mode, OFlags};

    check_component(session_id)?;
    check_component(run_id)?;
    let flags = OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC;
    let mount = open(
        root.parent().ok_or(io::ErrorKind::InvalidInput)?,
        flags,
        Mode::empty(),
    )?;
    let daemon = openat(
        &mount,
        root.file_name().ok_or(io::ErrorKind::InvalidInput)?,
        flags,
        Mode::empty(),
    )?;
    let session = openat(&daemon, session_id, flags, Mode::empty())?;
    let run = openat(&session, run_id, flags, Mode::empty())?;
    for entry in Dir::read_from(&run)? {
        let entry = entry?;
        let name = entry.file_name();
        let bytes = name.to_bytes();
        if bytes.starts_with(b"node-") && bytes.ends_with(b".md") {
            unlinkat(&run, name, AtFlags::empty())?;
        }
    }
    unlinkat(&session, run_id, AtFlags::REMOVEDIR)?;
    Ok(())
}

#[cfg(not(unix))]
fn remove_run_directory(root: &Path, session_id: &str, run_id: &str) -> io::Result<()> {
    check_component(session_id)?;
    check_component(run_id)?;
    std::fs::remove_dir_all(root.join(session_id).join(run_id))
}
