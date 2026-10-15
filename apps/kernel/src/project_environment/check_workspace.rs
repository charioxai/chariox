//! MP-08 / MP-10 / MP-11: filesystem preflight shared by Check and its owned I/O worker.
use super::*;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Serialize, Deserialize)]
pub(super) struct WorkspacePreflight {
    pub roots: Vec<PathBuf>,
    pub folder_accessible: bool,
}

pub(super) fn inspect(env: &ProjectEnvironment, requirement: &Requirement) -> WorkspacePreflight {
    #[cfg(test)]
    if let Some(marker) = std::env::var_os("CHARIOX_WORKSPACE_CHECK_STALL_MARKER") {
        std::fs::write(marker, std::process::id().to_string()).unwrap();
        std::thread::sleep(std::time::Duration::from_secs(2));
    }
    WorkspacePreflight {
        roots: env
            .folders
            .iter()
            .filter(|f| !f.local_workspace_binding.is_empty())
            .filter_map(|f| {
                PathBuf::from(&f.local_workspace_binding)
                    .canonicalize()
                    .ok()
            })
            .collect(),
        folder_accessible: match &requirement.scope {
            RequirementScope::Project => true,
            RequirementScope::Folder { folder_id } => env.folders.iter().any(|f| {
                &f.folder_id == folder_id
                    && !f.local_workspace_binding.is_empty()
                    && std::fs::read_dir(&f.local_workspace_binding).is_ok()
            }),
        },
    }
}
