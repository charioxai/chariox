use std::collections::HashSet;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::error::DaemonError;
use crate::local::{
    WorkspaceGitChangeTotals, WorkspaceGitCompareRef, WorkspaceGitFileChange, WorkspaceGitOverview,
};
use crate::runtime::workspace_git_changes::workspace_git_file_changes;
use crate::runtime::workspace_git_common::{
    detect_git_branch, git_command_output, git_reference_resolves, resolve_repo_root,
    workspace_default_compare_ref, workspace_display_label,
};

pub(crate) fn inspect_workspace_git_overview(
    workspace_id: &str,
    worktree_id: &str,
    requested_compare_ref: Option<&str>,
) -> Result<WorkspaceGitOverview, DaemonError> {
    let worktree_path = worktree_id.trim();
    if worktree_path.is_empty() {
        return Err(DaemonError::LocalTransport {
            operation: "inspect workspace git overview",
            message: "worktree_id is required".to_string(),
        });
    }
    let repo_root = resolve_repo_root(worktree_path)?;
    let repo_root_string = repo_root.display().to_string();
    let branch = detect_git_branch(worktree_path).ok();
    let compare_refs = workspace_git_compare_refs(&repo_root_string, branch.as_deref());
    let compare_ref = requested_compare_ref
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .or_else(|| {
            compare_refs
                .iter()
                .find(|candidate| candidate.selected)
                .map(|candidate| candidate.name.clone())
        })
        .unwrap_or_else(|| "HEAD".to_string());
    let files = workspace_git_file_changes(worktree_path, &compare_ref)?;
    let compare_refs = compare_refs
        .into_iter()
        .map(|candidate| WorkspaceGitCompareRef {
            selected: candidate.name == compare_ref,
            ..candidate
        })
        .collect();
    Ok(WorkspaceGitOverview {
        workspace_id: workspace_id.to_string(),
        worktree_id: worktree_id.to_string(),
        repo_root: Some(repo_root_string.clone()),
        repo_label: workspace_display_label(&repo_root_string),
        branch,
        compare_ref,
        compare_refs,
        totals: workspace_git_change_totals(&files),
        files,
        generated_at_ms: current_unix_ms(),
    })
}

fn workspace_git_change_totals(files: &[WorkspaceGitFileChange]) -> WorkspaceGitChangeTotals {
    WorkspaceGitChangeTotals {
        files: files.len().min(u32::MAX as usize) as u32,
        additions: files
            .iter()
            .map(|file| file.additions)
            .fold(0u32, u32::saturating_add),
        deletions: files
            .iter()
            .map(|file| file.deletions)
            .fold(0u32, u32::saturating_add),
    }
}

fn workspace_git_compare_refs(
    repo_root: &str,
    branch: Option<&str>,
) -> Vec<WorkspaceGitCompareRef> {
    let mut refs = Vec::new();
    let mut seen = HashSet::new();
    let default_ref = workspace_default_compare_ref(repo_root, branch);
    for (name, detail) in [
        ("main".to_string(), Some("default".to_string())),
        ("master".to_string(), None),
        ("origin/main".to_string(), Some("remote".to_string())),
        ("HEAD".to_string(), Some("uncommitted".to_string())),
    ] {
        if name != "HEAD" && !git_reference_resolves(repo_root, &name) {
            continue;
        }
        push_workspace_git_compare_ref(&mut refs, &mut seen, name, detail, &default_ref);
    }
    for name in git_command_output(
        repo_root,
        &[
            "for-each-ref",
            "--format=%(refname:short)",
            "refs/heads",
            "refs/remotes",
        ],
    )
    .unwrap_or_default()
    .lines()
    .map(str::trim)
    .filter(|value| !value.is_empty())
    .take(80)
    {
        let detail = if name.starts_with("origin/") {
            Some("remote".to_string())
        } else {
            None
        };
        push_workspace_git_compare_ref(
            &mut refs,
            &mut seen,
            name.to_string(),
            detail,
            &default_ref,
        );
    }
    if refs.is_empty() {
        push_workspace_git_compare_ref(
            &mut refs,
            &mut seen,
            "HEAD".to_string(),
            Some("uncommitted".to_string()),
            &default_ref,
        );
    }
    refs
}

fn push_workspace_git_compare_ref(
    refs: &mut Vec<WorkspaceGitCompareRef>,
    seen: &mut HashSet<String>,
    name: String,
    detail: Option<String>,
    default_ref: &str,
) {
    if !seen.insert(name.clone()) {
        return;
    }
    refs.push(WorkspaceGitCompareRef {
        selected: name == default_ref,
        name,
        detail,
    });
}

fn current_unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::{push_workspace_git_compare_ref, workspace_git_change_totals};
    use crate::local::{WorkspaceGitCompareRef, WorkspaceGitFileChange};

    #[test]
    fn workspace_git_change_totals_sum_saturating_counts() {
        let totals = workspace_git_change_totals(&[
            WorkspaceGitFileChange {
                path: "src/lib.rs".to_string(),
                status: "modified".to_string(),
                additions: 3,
                deletions: 1,
            },
            WorkspaceGitFileChange {
                path: "README.md".to_string(),
                status: "added".to_string(),
                additions: u32::MAX,
                deletions: 2,
            },
        ]);

        assert_eq!(totals.files, 2);
        assert_eq!(totals.additions, u32::MAX);
        assert_eq!(totals.deletions, 3);
    }

    #[test]
    fn compare_ref_projection_dedupes_and_marks_default() {
        let mut refs = Vec::<WorkspaceGitCompareRef>::new();
        let mut seen = HashSet::new();
        push_workspace_git_compare_ref(
            &mut refs,
            &mut seen,
            "origin/main".to_string(),
            Some("remote".to_string()),
            "origin/main",
        );
        push_workspace_git_compare_ref(
            &mut refs,
            &mut seen,
            "origin/main".to_string(),
            None,
            "origin/main",
        );

        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0].name, "origin/main");
        assert_eq!(refs[0].detail.as_deref(), Some("remote"));
        assert!(refs[0].selected);
    }
}
