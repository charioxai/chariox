//! MP-08 / MP-10 / MP-11: native trust is limited to the kernel-selected workspace.
use std::path::Path;

/// Use Claude's documented interactive trust decision, not a permission bypass
/// or edits to provider-owned account state. Claude trusts a Git root (the main
/// checkout for worktrees), so a parent repository must never be trusted merely
/// because its child directory was selected.
pub(super) fn selected_workspace_is_trust_target(
    selected: &Path,
    working_directory: Option<&Path>,
    rendered: &str,
) -> bool {
    let Some((_, frame)) = rendered.rsplit_once("Accessing workspace:") else {
        return false;
    };
    // The native permission buffer normalizes whitespace across incremental
    // Ink frames. Use Claude's following heading as the path boundary rather
    // than relying on line breaks surviving that shared buffer.
    let Some((displayed, _)) = frame.split_once("Quick safety check") else {
        return false;
    };
    let displayed = displayed.trim();
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
    for ancestor in selected.ancestors() {
        let git = ancestor.join(".git");
        match git.symlink_metadata() {
            Ok(_) => {
                // A .git file points at a main checkout whose trust scope may
                // exceed this selection; leave that decision to the user.
                return ancestor == selected
                    && git.is_dir()
                    && git
                        .canonicalize()
                        .is_ok_and(|path| path.starts_with(&selected));
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => return false,
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_workspace_trust_never_expands_the_selected_path() {
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
            &frame.replace('\n', " ")
        ));
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
