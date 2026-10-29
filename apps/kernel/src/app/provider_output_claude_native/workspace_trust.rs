//! MP-08 / MP-10 / MP-11: native trust is limited to the kernel-selected workspace.
use std::path::Path;

/// Reusing launch consent never authorizes a wider native trust target.
/// Only headings tolerate missing spaces; the pathname remains exact.
pub(super) fn selected_workspace_is_trust_target(
    selected: &Path,
    working_directory: Option<&Path>,
    rendered: &str,
) -> bool {
    let Some(displayed) = displayed_workspace(rendered) else {
        return false;
    };
    let Ok(selected) = selected.canonicalize() else {
        return false;
    };
    let Some(working_directory) = working_directory.and_then(|path| path.canonicalize().ok())
    else {
        return false;
    };
    let displayed = Path::new(displayed);
    if !displayed.is_absolute()
        || displayed.canonicalize().ok().as_ref() != Some(&selected)
        || working_directory != selected
    {
        return false;
    }
    selected_project_trust_scope(&selected)
}

/// Only the displayed headings tolerate terminal cursor gaps, never the path.
pub(super) fn displayed_workspace(rendered: &str) -> Option<&str> {
    let (_, start) = heading_span(rendered, "Accessing workspace:", true)?;
    let frame = &rendered[start..];
    let (end, _) = heading_span(frame, "Quick safety check", false)?;
    let target = frame[..end].trim_start();
    // The path ends at the line boundary before the heading. Spaces before
    // that boundary belong to the pathname; spaces after it are indentation.
    Some(match target.rfind('\n') {
        Some(boundary) => target[..boundary].trim_end_matches(['\r', '\n']),
        None => target,
    })
}

pub(super) fn selected_startup_workspace(
    app: &crate::app::DaemonApp,
    session_id: &str,
    run: &crate::provider::RuntimeProviderRun,
    context_file: &str,
    rendered: &str,
) -> bool {
    if run.session_id() != session_id
        || super::permission::claude_headless_composer_initialized(context_file)
    {
        return false;
    }
    let Some(agent_id) = run.agent_instance_id() else {
        return false;
    };
    let Ok(session) = app.sessions.get_session(session_id) else {
        return false;
    };
    let Ok(agent) = app.agents.get_agent(agent_id) else {
        return false;
    };
    if agent.session_id() != session_id || session.project_id().is_empty() {
        return false;
    }
    if app
        .providers
        .get_run_for_agent(session_id, agent_id)
        .is_none_or(|current| {
            current.id() != run.id()
                || current.state() != crate::provider::ProviderRunState::Running
        })
    {
        return false;
    }
    let selected = Path::new(agent.worktree_id().unwrap_or_else(|| session.worktree_id()));
    let home = run
        .pty_env()
        .get("HOME")
        .map(std::path::PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(std::path::PathBuf::from))
        .and_then(|path| path.canonicalize().ok());
    let selected_path = selected.canonicalize().ok();
    home.is_some()
        && selected_path.is_some()
        && home != selected_path
        && selected_workspace_is_trust_target(
            selected,
            run.working_directory().map(std::path::PathBuf::as_path),
            rendered,
        )
}

pub(super) fn reuses_workspace_consent(interaction_id: &str) -> bool {
    let id = interaction_id
        .strip_prefix("select-yes:")
        .and_then(|selected| selected.split_once(':').map(|(_, id)| id))
        .unwrap_or(interaction_id);
    id.starts_with("claude-selected-workspace-trust-")
}

pub(super) fn validate_selected_workspace(
    app: &crate::app::DaemonApp,
    run_id: &str,
    context_file: &str,
    interaction_id: &str,
) -> Result<(), crate::error::DaemonError> {
    if !reuses_workspace_consent(interaction_id) {
        return Ok(());
    }
    let run = app.providers.get_run(run_id)?;
    let recent = super::permission::claude_permission_recent_file(context_file)
        .and_then(|path| std::fs::read_to_string(path).ok())
        .unwrap_or_default();
    if selected_startup_workspace(app, run.session_id(), &run, context_file, &recent) {
        return Ok(());
    }
    super::permission::write_claude_native_marker(
        context_file,
        &format!(
            "{}{interaction_id}",
            super::CLAUDE_WORKSPACE_TRUST_SELECTION_FAILED_PREFIX,
        ),
    );
    Err(crate::error::DaemonError::LocalTransport {
        operation: "approve Claude workspace trust",
        message:
            "Claude's selected workspace or startup run changed before approval; retry the launch."
                .into(),
    })
}

fn selected_project_trust_scope(selected: &Path) -> bool {
    // Claude may trust an ancestor Git root or a worktree's main checkout.
    // Selecting the child folder never authorizes either wider scope.
    for ancestor in selected.ancestors() {
        let git = ancestor.join(".git");
        match git.symlink_metadata() {
            Ok(_) => {
                return ancestor == selected
                    && git.is_dir()
                    && git
                        .canonicalize()
                        .is_ok_and(|path| path.starts_with(selected));
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => return false,
        }
    }
    true
}

fn heading_span(text: &str, heading: &str, last: bool) -> Option<(usize, usize)> {
    let chars = text
        .char_indices()
        .filter(|(_, ch)| !ch.is_whitespace())
        .collect::<Vec<_>>();
    let needle = heading
        .chars()
        .filter(|ch| !ch.is_whitespace())
        .collect::<Vec<_>>();
    if needle.is_empty() {
        return None;
    }
    let mut matches = chars
        .windows(needle.len())
        .filter(|window| {
            window
                .iter()
                .zip(&needle)
                .all(|((_, ch), expected)| ch.eq_ignore_ascii_case(expected))
        })
        .map(|window| {
            let (end, ch) = window[window.len() - 1];
            (window[0].0, end + ch.len_utf8())
        });
    if last {
        matches.next_back()
    } else {
        matches.next()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_trust_never_expands_the_selected_path() {
        let root = crate::test_support::TestWorktree::new("claude-trust-scope");
        let selected = root.path();
        let frame = format!(
            "Accessing workspace:\n{}\nQuick safety check",
            selected.display()
        );
        assert!(selected_workspace_is_trust_target(
            selected,
            Some(selected),
            &frame
        ));
        assert!(selected_workspace_is_trust_target(
            selected,
            Some(selected),
            &frame.replace('\n', "\r\n")
        ));
        assert!(
            !selected_workspace_is_trust_target(
                selected,
                Some(selected),
                &frame.replace('\n', " ")
            ),
            "spaces are pathname characters, never line boundaries"
        );
        assert!(
            selected_workspace_is_trust_target(
                selected,
                Some(selected),
                &frame
                    .replace("Accessing workspace:", "Accessingworkspace:")
                    .replace("Quick safety check", "Quicksafetycheck")
            ),
            "fallback headings must tolerate missing spaces without collapsing path names"
        );
        assert!(!selected_workspace_is_trust_target(selected, None, &frame));
        assert!(!selected_workspace_is_trust_target(
            selected,
            Some(selected),
            "Quick safety check"
        ));
        let child = selected.join("child");
        std::fs::create_dir(&child).unwrap();
        let child_frame = format!(
            "Accessing workspace: {}\nQuick safety check",
            child.display()
        );
        assert!(!selected_workspace_is_trust_target(
            selected,
            Some(selected),
            &child_frame
        ));
        assert!(!selected_workspace_is_trust_target(
            selected,
            Some(&child),
            &frame
        ));
        let spaced = selected.join("folder  name");
        let compacted = selected.join("foldername");
        std::fs::create_dir(&spaced).unwrap();
        std::fs::create_dir(&compacted).unwrap();
        let spaced_frame = format!("Accessingworkspace: {}\nQuicksafetycheck", spaced.display());
        assert!(selected_workspace_is_trust_target(
            &spaced,
            Some(&spaced),
            &spaced_frame
        ));
        let wrong_frame = format!(
            "Accessingworkspace: {}\nQuicksafetycheck",
            compacted.display()
        );
        assert!(
            !selected_workspace_is_trust_target(&spaced, Some(&spaced), &wrong_frame),
            "path spaces must not be compacted like headings"
        );
        std::fs::create_dir(selected.join(".git")).unwrap();
        assert!(selected_workspace_is_trust_target(
            selected,
            Some(selected),
            &frame
        ));
        assert!(
            !selected_workspace_is_trust_target(&child, Some(&child), &child_frame),
            "ancestor Git trust exceeds selected child"
        );
        std::fs::remove_dir(selected.join(".git")).unwrap();
        std::fs::write(
            selected.join(".git"),
            "gitdir: /unchosen/main/.git/worktrees/child",
        )
        .unwrap();
        assert!(
            !selected_workspace_is_trust_target(selected, Some(selected), &frame),
            "main-checkout trust must not be inferred from a worktree selection"
        );
    }
}
