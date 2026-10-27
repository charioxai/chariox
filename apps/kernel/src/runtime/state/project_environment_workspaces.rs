//! MP-08 / MP-10 / MP-11: Resolve Project identities to this session's repository paths.
use crate::managed_context::development::{
    DevelopmentRepositoryRole, DevelopmentRepositorySelection,
};
use crate::session::{RuntimeProject, RuntimeSession};
use std::path::PathBuf;

pub(super) fn project_environment_repository_selections(
    project: &RuntimeProject,
    session: &RuntimeSession,
) -> Vec<DevelopmentRepositorySelection> {
    project
        .workspace_ids()
        .iter()
        .map(|workspace| {
            let primary = workspace == session.workspace_id();
            let worktree = if primary {
                session.worktree_id()
            } else {
                workspace
            };
            DevelopmentRepositorySelection {
                workspace_id: workspace.clone(),
                worktree_id: Some(worktree.to_string()),
                worktree_path: PathBuf::from(worktree),
                role: if primary {
                    DevelopmentRepositoryRole::Primary
                } else {
                    DevelopmentRepositoryRole::Supporting
                },
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project_environment::index_project_environment;
    use crate::session::RuntimeProjectKind;
    use std::collections::BTreeMap;

    #[test]
    fn mp08_mp10_mp11_worker_reexport_indexes_mounted_worktree_not_logical_workspace() {
        struct Scratch(PathBuf);
        impl Drop for Scratch {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }
        let scratch = Scratch(std::env::temp_dir().join(format!(
            "chariox-envliveg-worker-export-{}-{}",
            std::process::id(),
            rand::random::<u64>()
        )));
        let mounted = scratch.0.join("mounted");
        let supporting = scratch.0.join("supporting");
        std::fs::create_dir_all(&mounted).unwrap();
        std::fs::create_dir_all(&supporting).unwrap();
        for root in [&mounted, &supporting] {
            assert!(std::process::Command::new("git")
                .args(["init", "--quiet"])
                .arg(root)
                .status()
                .unwrap()
                .success());
        }
        std::fs::write(mounted.join("app.c"), "getenv(\"APP_LABEL\")\n").unwrap();
        let logical = scratch.0.join("unavailable-home").display().to_string();
        let mut project = RuntimeProject::new(
            "project-1",
            "owner-1",
            &logical,
            "Imported Project",
            RuntimeProjectKind::Named,
        );
        // The session's primary is authoritative even when another repository is first.
        project.replace_workspace_ids(vec![supporting.display().to_string(), logical.clone()]);
        let session = RuntimeSession::new(
            "session-1",
            None,
            &logical,
            mounted.display().to_string(),
            "worker-machine",
            "worker-kernel",
        );
        let selections = project_environment_repository_selections(&project, &session);
        let roots: BTreeMap<_, _> = selections
            .iter()
            .map(|r| (r.workspace_id.clone(), r.worktree_path.clone()))
            .collect();
        let index = index_project_environment(&roots, &BTreeMap::new())
            .expect("a worker must index its actual mounted Project repository");
        assert!(index
            .references
            .iter()
            .any(|entry| entry.name == "APP_LABEL" && entry.workspace_id == logical));
        let primary = selections
            .iter()
            .find(|r| r.role == DevelopmentRepositoryRole::Primary)
            .unwrap();
        assert_eq!(primary.worktree_path, mounted);
        assert_eq!(primary.worktree_id.as_deref(), mounted.to_str());
        assert_eq!(selections[0].role, DevelopmentRepositoryRole::Supporting);
        assert_eq!(selections[0].worktree_path, supporting);
        assert!(!std::path::Path::new(&logical).exists());
    }
}
