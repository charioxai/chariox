//! MP-08 / MP-09 / MP-10 / MP-11: filesystem and Git preparation in the owned metadata worker.
use crate::session::RuntimeProject;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Candidate {
    pub id: String,
    pub workspaces: Vec<String>,
}
#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct Input {
    pub workspace: String,
    pub create: bool,
    pub selected: Option<Candidate>,
    pub defaults: Vec<Candidate>,
}
#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct Metadata {
    pub workspace: String,
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
fn matching_workspace<'a>(candidate: &'a Candidate, workspace: &str) -> Option<&'a str> {
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
                .find(|path| std::fs::canonicalize(path).ok().as_ref() == Some(&canonical))
        })
        .map(String::as_str)
}
pub(crate) fn measure(input: Input) -> Result<Metadata, Failure> {
    let selected_binding = input
        .selected
        .as_ref()
        .map(|candidate| {
            matching_workspace(candidate, &input.workspace).ok_or(Failure::WorkspaceMismatch)
        })
        .transpose()?;
    // Detached, already saved Projects remain readable without a filesystem preflight.
    if !input.create {
        if let (Some(candidate), Some(binding)) = (&input.selected, selected_binding) {
            return Ok(Metadata {
                workspace: binding.into(),
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
    let canonical = crate::runtime::workspace_git_common::canonical_workspace_path(&path, &path)
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
                matching_workspace(candidate, &canonical).map(|path| (candidate, path))
            })
        });
    if let Some((candidate, binding)) = matched {
        return Ok(Metadata {
            workspace: binding.into(),
            name: None,
            project_id: Some(candidate.id.clone()),
        });
    }
    let name = crate::runtime::workspace_git_common::workspace_display_label(&canonical);
    Ok(Metadata {
        workspace: canonical,
        name,
        project_id: None,
    })
}
