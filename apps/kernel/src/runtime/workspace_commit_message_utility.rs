use crate::error::DaemonError;
use crate::local::{WorkspaceCommitMessageUtilityInput, WorkspaceGitOverview};
use crate::runtime::workspace_git_changes::workspace_git_diff_text;
use crate::runtime::workspace_git_overview::inspect_workspace_git_overview;

pub(crate) fn workspace_commit_message_utility_prompt(
    input: &WorkspaceCommitMessageUtilityInput,
) -> Result<String, DaemonError> {
    let assembly = workspace_commit_message_utility_prompt_assembly(input)?;
    Ok(format!(
        "{}\n\n{}",
        assembly.hidden_system_context, assembly.visible_user_prompt
    ))
}

pub(crate) struct WorkspaceCommitMessageUtilityPrompt {
    pub(crate) visible_user_prompt: String,
    pub(crate) hidden_system_context: String,
}

pub(crate) fn workspace_commit_message_utility_prompt_assembly(
    input: &WorkspaceCommitMessageUtilityInput,
) -> Result<WorkspaceCommitMessageUtilityPrompt, DaemonError> {
    let overview = inspect_workspace_git_overview(
        &input.workspace_id,
        &input.worktree_id,
        input.compare_ref.as_deref(),
    )?;
    if overview.files.is_empty() {
        return Err(DaemonError::LocalTransport {
            operation: "run workspace commit message utility",
            message: "no workspace changes to summarize".to_string(),
        });
    }
    let diff = workspace_git_diff_text(&input.worktree_id, &overview.compare_ref, 60_000)?;
    Ok(WorkspaceCommitMessageUtilityPrompt {
        visible_user_prompt: workspace_commit_message_visible_prompt_from_overview(
            &overview, &diff,
        ),
        hidden_system_context: crate::prompt_assembly::PromptAssemblyService::from_env()?
            .assemble_hidden_context_only(&["utility/workspace-commit-message"])?
            .0,
    })
}

fn workspace_commit_message_visible_prompt_from_overview(
    overview: &WorkspaceGitOverview,
    diff: &str,
) -> String {
    let files = overview
        .files
        .iter()
        .map(|file| {
            format!(
                "- {} {} +{} -{}",
                file.status, file.path, file.additions, file.deletions
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    format!(
        "Workspace: {workspace}\n\
Worktree: {worktree}\n\
Compare ref: {compare_ref}\n\
Totals: {files_count} files, +{additions} -{deletions}\n\n\
Changed files:\n{files}\n\n\
Diff context:\n{diff}",
        workspace = overview.workspace_id,
        worktree = overview.worktree_id,
        compare_ref = overview.compare_ref,
        files_count = overview.totals.files,
        additions = overview.totals.additions,
        deletions = overview.totals.deletions,
        files = files,
        diff = if diff.is_empty() {
            "<no textual diff available>"
        } else {
            diff
        },
    )
}

#[cfg(test)]
mod tests {
    use super::workspace_commit_message_visible_prompt_from_overview;
    use crate::local::{WorkspaceGitChangeTotals, WorkspaceGitFileChange, WorkspaceGitOverview};

    #[test]
    fn commit_message_prompt_projects_change_summary_and_diff() {
        let prompt = workspace_commit_message_visible_prompt_from_overview(
            &WorkspaceGitOverview {
                workspace_id: "/repo".to_string(),
                worktree_id: "/repo/worktree".to_string(),
                repo_root: Some("/repo".to_string()),
                repo_label: Some("org/repo".to_string()),
                branch: Some("feature".to_string()),
                compare_ref: "origin/main".to_string(),
                compare_refs: Vec::new(),
                totals: WorkspaceGitChangeTotals {
                    files: 1,
                    additions: 12,
                    deletions: 3,
                },
                files: vec![WorkspaceGitFileChange {
                    path: "src/lib.rs".to_string(),
                    status: "modified".to_string(),
                    additions: 12,
                    deletions: 3,
                }],
                generated_at_ms: 1,
            },
            "diff --git a/src/lib.rs b/src/lib.rs",
        );

        assert!(prompt.contains("Workspace: /repo"));
        assert!(prompt.contains("Compare ref: origin/main"));
        assert!(prompt.contains("Totals: 1 files, +12 -3"));
        assert!(prompt.contains("- modified src/lib.rs +12 -3"));
        assert!(prompt.contains("diff --git a/src/lib.rs b/src/lib.rs"));
    }

    #[test]
    fn commit_message_prompt_uses_no_textual_diff_fallback() {
        let prompt = workspace_commit_message_visible_prompt_from_overview(
            &WorkspaceGitOverview {
                workspace_id: "/repo".to_string(),
                worktree_id: "/repo".to_string(),
                repo_root: None,
                repo_label: None,
                branch: None,
                compare_ref: "HEAD".to_string(),
                compare_refs: Vec::new(),
                totals: WorkspaceGitChangeTotals {
                    files: 0,
                    additions: 0,
                    deletions: 0,
                },
                files: Vec::new(),
                generated_at_ms: 1,
            },
            "",
        );

        assert!(prompt.contains("<no textual diff available>"));
    }
}
