//! The `git init` step of a Room's setup. The repository starts with one empty commit: a new
//! worktree is created from a commit and publishing pushes one, and a fresh `git init` has neither.
use crate::error::DaemonError;
use crate::runtime::workspace_git_common::{git_command_output, run_git};
use std::path::Path;

const FIRST_COMMIT_MESSAGE: &str = "Initial commit";
/// Used only on a machine whose Git has no identity, where the first commit would otherwise fail.
const FALLBACK_IDENTITY: [&str; 4] = [
    "-c",
    "user.name=Chariox",
    "-c",
    "user.email=room@chariox.invalid",
];

pub(super) fn init_with_first_commit(folder: &Path) -> Result<(), DaemonError> {
    run_git(folder, &["init"])?;
    let has_identity =
        git_command_output(&folder.to_string_lossy(), &["var", "GIT_COMMITTER_IDENT"]).is_some();
    let mut args = Vec::new();
    if !has_identity {
        args.extend(FALLBACK_IDENTITY);
    }
    args.extend(["commit", "--allow-empty", "-m", FIRST_COMMIT_MESSAGE]);
    run_git(folder, &args)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch_folder(name: &str) -> std::path::PathBuf {
        let root = std::env::temp_dir().join(format!(
            "chariox-room-git-{name}-{}-{}",
            std::process::id(),
            crate::session::unix_epoch_ms()
        ));
        std::fs::create_dir_all(&root).unwrap();
        root
    }

    fn git(folder: &Path, args: &[&str]) -> String {
        git_command_output(&folder.to_string_lossy(), args).unwrap_or_default()
    }

    #[test]
    fn a_new_room_folder_has_a_commit_to_publish_and_to_branch_a_worktree_from() {
        let folder = scratch_folder("worktree");

        init_with_first_commit(&folder).unwrap();

        assert_eq!(git(&folder, &["rev-list", "--count", "HEAD"]), "1");
        assert_eq!(
            git(&folder, &["log", "-1", "--format=%s"]),
            FIRST_COMMIT_MESSAGE
        );
        let worktree = folder.with_file_name(format!(
            "{}-worktree",
            folder.file_name().and_then(|name| name.to_str()).unwrap()
        ));
        run_git(
            &folder,
            &[
                "worktree",
                "add",
                "-b",
                "feature",
                worktree.to_str().unwrap(),
            ],
        )
        .expect("a worktree needs a commit to start from");
        std::fs::remove_dir_all(&worktree).unwrap();
        std::fs::remove_dir_all(&folder).unwrap();
    }

    #[test]
    fn files_already_in_the_folder_stay_uncommitted() {
        let folder = scratch_folder("files");
        std::fs::write(folder.join(".env"), "SECRET=1\n").unwrap();

        init_with_first_commit(&folder).unwrap();

        assert_eq!(git(&folder, &["ls-files"]), "");
        std::fs::remove_dir_all(&folder).unwrap();
    }
}
