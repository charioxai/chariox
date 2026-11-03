//! MP-08 / MP-10 / MP-11: one bounded metadata measurement per probe operation.
use super::*;
use serde::{Deserialize, Serialize};
use std::{collections::BTreeSet, path::PathBuf, time::Instant};

#[derive(Serialize, Deserialize)]
pub(super) struct WorkspacePreflight {
    pub roots: Vec<PathBuf>,
    pub accessible_folders: BTreeSet<String>,
}

#[derive(Default)]
pub(crate) struct CheckWorkspace {
    measured: Option<Result<WorkspacePreflight, ()>>,
}
impl CheckWorkspace {
    pub(super) fn for_requirement(
        &mut self,
        config: &crate::config::DaemonConfig,
        env: &ProjectEnvironment,
        r: &Requirement,
        deadline: Instant,
    ) -> Result<(Vec<PathBuf>, bool), ()> {
        // Files already perform descriptor-pinned metadata and content reads in
        // their own worker. Project bindings/accounts do not consume any roots.
        if matches!(r.spec, RequirementSpec::Files { .. }) {
            return Ok((vec![], super::check::workspace(env, r).is_some()));
        }
        let needs_roots = match &r.spec {
            RequirementSpec::MachineNeeds {
                minimum_free_disk_bytes,
                ..
            } => minimum_free_disk_bytes.is_some(),
            RequirementSpec::Software { .. }
            | RequirementSpec::Services { .. }
            | RequirementSpec::AgentTools { .. }
            | RequirementSpec::SetupChecks { .. } => true,
            _ => false,
        };
        if !needs_roots && matches!(r.scope, RequirementScope::Project) {
            return Ok((vec![], true));
        }
        let measured = self.measured.get_or_insert_with(|| {
            super::check_files::workspace(config, env, deadline).map_err(|_| ())
        });
        match measured {
            Ok(measured) => Ok((
                if needs_roots {
                    measured.roots.clone()
                } else {
                    vec![]
                },
                match &r.scope {
                    RequirementScope::Project => true,
                    RequirementScope::Folder { folder_id } => {
                        measured.accessible_folders.contains(folder_id)
                    }
                },
            )),
            Err(()) => Err(()),
        }
    }
}

pub(super) fn inspect(folders: &[(String, PathBuf)]) -> WorkspacePreflight {
    #[cfg(test)]
    if let Some(path) = std::env::var_os("CHARIOX_WORKSPACE_CHECK_COUNT") {
        use std::io::Write;
        writeln!(
            std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(path)
                .unwrap(),
            "worker"
        )
        .unwrap();
    }
    #[cfg(test)]
    if let Some(marker) = std::env::var_os("CHARIOX_WORKSPACE_CHECK_STALL_MARKER") {
        std::fs::write(marker, std::process::id().to_string()).unwrap();
        std::thread::sleep(std::time::Duration::from_secs(2));
    }
    let mut measured = WorkspacePreflight {
        roots: vec![],
        accessible_folders: BTreeSet::new(),
    };
    for (id, path) in folders {
        if let Ok(root) = path.canonicalize() {
            if std::fs::read_dir(&root).is_ok() {
                measured.accessible_folders.insert(id.clone());
            }
            measured.roots.push(root);
        }
    }
    measured
}
