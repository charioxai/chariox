//! MP-08 / MP-09 / MP-10 / MP-11: filesystem and Git preparation in the owned metadata worker.
use crate::session::RuntimeProject;
use serde::{Deserialize, Serialize};
use std::cell::OnceCell;
use std::collections::HashMap;
use std::path::PathBuf;
use std::time::{Duration, Instant};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Candidate {
    pub id: String,
    pub workspaces: Vec<String>,
}
#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct Input {
    pub workspace: String,
    /// The session's worktree, which can move the effective workspace to another repository.
    pub worktree: Option<String>,
    pub create: bool,
    pub selected: Option<Candidate>,
    pub defaults: Vec<Candidate>,
}
#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct Metadata {
    pub workspace: String,
    /// The folder the request resolves to after Git normalization, before the saved spelling.
    pub identity: String,
    pub name: Option<String>,
    pub project_id: Option<String>,
}
#[derive(Clone, Copy, Serialize, Deserialize)]
pub(crate) enum Failure {
    WorkspaceUnavailable,
    WorkspaceMismatch,
}
impl Failure {
    pub(crate) fn error(self) -> crate::error::DaemonError {
        crate::error::DaemonError::LocalTransport {
            operation: "project.resolve",
            message: match self {
                Self::WorkspaceUnavailable => "Project folder unavailable or protected; choose an existing absolute directory and retry",
                Self::WorkspaceMismatch => "caller does not own the selected Project and workspace",
            }.into(),
        }
    }
}
// A saved folder that does not answer within this bound cannot be proven to be the requested one.
const SAVED_FOLDER_PROBE: Duration = Duration::from_secs(1);

// Real paths of the saved folders, probed together on first use: an unresponsive mount costs one
// bound and never decides the outcome for the others. Stuck probes end with this worker process.
struct SavedFolders<'a> {
    input: &'a Input,
    resolved: OnceCell<HashMap<String, PathBuf>>,
}
impl<'a> SavedFolders<'a> {
    fn new(input: &'a Input) -> Self {
        Self {
            input,
            resolved: OnceCell::new(),
        }
    }
    fn resolved(&self) -> &HashMap<String, PathBuf> {
        self.resolved.get_or_init(|| {
            let (answers, received) = std::sync::mpsc::channel();
            let saved = self.input.selected.iter().chain(&self.input.defaults);
            for path in saved.flat_map(|candidate| candidate.workspaces.iter().cloned()) {
                let answers = answers.clone();
                std::thread::spawn(move || {
                    #[cfg(test)]
                    if std::env::var_os("CHARIOX_PROJECT_PREPARATION_STALL_PATH")
                        .is_some_and(|stalled| stalled == path.as_str())
                    {
                        std::thread::sleep(Duration::from_secs(30));
                    }
                    // A saved folder that is gone cannot be this folder.
                    if std::fs::symlink_metadata(&path).is_ok() {
                        if let Ok(real) = std::fs::canonicalize(&path) {
                            let _ = answers.send((path, real));
                        }
                    }
                });
            }
            drop(answers);
            let deadline = Instant::now() + SAVED_FOLDER_PROBE;
            let mut resolved = HashMap::new();
            while let Ok((path, real)) =
                received.recv_timeout(deadline.saturating_duration_since(Instant::now()))
            {
                resolved.insert(path, real);
            }
            resolved
        })
    }
}
fn matching_workspace<'a>(
    candidate: &'a Candidate,
    workspace: &str,
    folders: &SavedFolders,
) -> Option<&'a str> {
    candidate
        .workspaces
        .iter()
        .find(|path| path.as_str() == workspace)
        .or_else(|| {
            candidate
                .workspaces
                .iter()
                .find(|path| RuntimeProject::workspace_paths_match(path, workspace))
        })
        .or_else(|| {
            let canonical = std::fs::canonicalize(workspace).ok()?;
            candidate
                .workspaces
                .iter()
                .find(|path| folders.resolved().get(path.as_str()) == Some(&canonical))
        })
        .map(String::as_str)
}
pub(crate) fn measure(input: Input) -> Result<Metadata, Failure> {
    let folders = SavedFolders::new(&input);
    let selected_binding = input
        .selected
        .as_ref()
        .map(|candidate| {
            matching_workspace(candidate, &input.workspace, &folders)
                .ok_or(Failure::WorkspaceMismatch)
        })
        .transpose()?;
    // Detached, already saved Projects remain readable without a filesystem preflight.
    if !input.create {
        if let (Some(candidate), Some(binding)) = (&input.selected, selected_binding) {
            return Ok(Metadata {
                workspace: binding.into(),
                identity: binding.into(),
                name: None,
                project_id: Some(candidate.id.clone()),
            });
        }
    }
    let admitted = crate::git_worktree_placement::preflight_existing_absolute_directory(
        &input.workspace,
        "workspace_id",
        "project.resolve",
    )
    .map_err(|_| Failure::WorkspaceUnavailable)?;
    let path = admitted.canonical_path.to_string_lossy().into_owned();
    let worktree = input.worktree.as_deref().unwrap_or(&path);
    let canonical = crate::runtime::workspace_git_common::canonical_workspace_path(&path, worktree)
        .unwrap_or(path);
    crate::git_worktree_placement::preflight_existing_absolute_directory(
        &canonical,
        "workspace_id",
        "project.resolve",
    )
    .map_err(|_| Failure::WorkspaceUnavailable)?;
    if let (Some(candidate), Some(binding)) = (&input.selected, selected_binding) {
        return Ok(Metadata {
            workspace: binding.into(),
            identity: canonical,
            name: None,
            project_id: Some(candidate.id.clone()),
        });
    }
    let matched = input
        .defaults
        .iter()
        .find_map(|candidate| {
            candidate
                .workspaces
                .iter()
                .find(|path| path.as_str() == canonical)
                .map(|path| (candidate, path.as_str()))
        })
        .or_else(|| {
            input.defaults.iter().find_map(|candidate| {
                matching_workspace(candidate, &canonical, &folders).map(|path| (candidate, path))
            })
        });
    if let Some((candidate, binding)) = matched {
        return Ok(Metadata {
            workspace: binding.into(),
            identity: canonical,
            name: None,
            project_id: Some(candidate.id.clone()),
        });
    }
    let name = crate::runtime::workspace_git_common::workspace_display_label(&canonical);
    Ok(Metadata {
        workspace: canonical.clone(),
        identity: canonical,
        name,
        project_id: None,
    })
}
