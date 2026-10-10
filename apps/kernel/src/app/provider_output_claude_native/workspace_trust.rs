//! MP-08 / MP-10 / MP-11: native trust is limited to the kernel-selected workspace.
use std::path::Path;

/// Screen fallback retains the same exact Project scope as pre-recorded trust.
/// Only headings tolerate missing spaces; the pathname remains exact.
pub(super) fn selected_workspace_is_trust_target(
    selected: &Path,
    working_directory: Option<&Path>,
    rendered: &str,
) -> bool {
    let Some((_, frame_start)) = heading_span(rendered, "Accessing workspace:", true) else {
        return false;
    };
    let frame = &rendered[frame_start..];
    // The native permission buffer normalizes whitespace across incremental
    // Ink frames. Use Claude's following heading as the path boundary rather
    // than relying on line breaks surviving that shared buffer.
    let Some((path_end, _)) = heading_span(frame, "Quick safety check", false) else {
        return false;
    };
    let displayed = frame[..path_end].trim();
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
    crate::provider::selected_project_trust_scope(&selected)
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
        matches.last()
    } else {
        matches.next()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn pre_recorded_claude_fallback_never_expands_the_selected_path() {
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
        let spaced_frame = format!("Accessingworkspace: {} Quicksafetycheck", spaced.display());
        assert!(selected_workspace_is_trust_target(
            &spaced,
            Some(&spaced),
            &spaced_frame
        ));
        let wrong_frame = format!(
            "Accessingworkspace: {} Quicksafetycheck",
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
