//! MP-08 / MP-10 / MP-11: bounded Environment measurements through one owned I/O worker.
use super::check_io_worker::{run, run_at_root, run_with_cancel, Request, Response};
use super::*;
use crate::error::DaemonError;
use std::{
    path::{Path, PathBuf},
    time::Instant,
};

// MP-08 / MP-10 / MP-11: reuse owned, nondumpable metadata I/O for background views.
pub(crate) fn inspect_workspace_metadata(
    root: &std::path::Path,
    inspection: crate::runtime::workspace_inspection::Inspection,
    deadline: std::time::Instant,
) -> Result<crate::runtime::workspace_inspection::Metadata, crate::error::DaemonError> {
    match run_at_root(
        root,
        Request::WorkspaceInspection(inspection),
        deadline,
        || false,
    ) {
        Ok(Response::WorkspaceInspection(Ok(metadata))) => Ok(metadata),
        _ => Err(crate::runtime::workspace_inspection::unavailable()),
    }
}

pub(super) fn machine(
    config: &crate::config::DaemonConfig,
    root: Option<&Path>,
    deadline: Instant,
) -> Result<EnvironmentMeasuredFacts, DaemonError> {
    match run(
        config,
        Request::Machine {
            root: root.map(Path::to_path_buf),
        },
        deadline,
    )? {
        Response::Machine(facts) => Ok(facts),
        _ => Err(environment_error("Machine Check worker result invalid")),
    }
}

pub(super) fn measure(
    config: &crate::config::DaemonConfig,
    current: &ProjectEnvironment,
    draft: &mut EnvironmentRevisionDraft,
    deadline: Instant,
) -> Result<(), DaemonError> {
    match run(
        config,
        Request::Files {
            current: current.clone(),
            draft: draft.clone(),
        },
        deadline,
    )? {
        Response::Files(measured) => {
            *draft = measured;
            Ok(())
        }
        _ => Err(environment_error("Files Check worker result invalid")),
    }
}

pub(super) fn workspace(
    config: &crate::config::DaemonConfig,
    current: &ProjectEnvironment,
    deadline: Instant,
) -> Result<super::check_workspace::WorkspacePreflight, DaemonError> {
    match run(
        config,
        Request::Workspace {
            folders: current
                .folders
                .iter()
                .filter(|f| !f.local_workspace_binding.is_empty())
                .map(|f| {
                    (
                        f.folder_id.clone(),
                        PathBuf::from(&f.local_workspace_binding),
                    )
                })
                .collect(),
        },
        deadline,
    )? {
        Response::Workspace(measured) => Ok(measured),
        _ => Err(environment_error("Workspace Check worker result invalid")),
    }
}

pub(super) fn binding(
    config: &crate::config::DaemonConfig,
    root: Option<&Path>,
    locator: &ProjectEnvironmentLocator,
    deadline: Instant,
) -> Result<super::check_binding::PublicBinding, DaemonError> {
    match run(
        config,
        Request::Binding {
            root: root.map(Path::to_path_buf),
            locator: locator.clone(),
        },
        deadline,
    )? {
        Response::Binding(binding) => Ok(binding),
        _ => Err(environment_error("Binding Check worker result invalid")),
    }
}
pub(super) fn software(
    config: &crate::config::DaemonConfig,
    root: Option<&Path>,
    name: &str,
    source: Option<&str>,
    roots: &[PathBuf],
    deadline: Instant,
) -> Result<(Option<String>, bool), &'static str> {
    match run(
        config,
        Request::Software {
            root: root.map(Path::to_path_buf),
            name: name.into(),
            source: source.map(str::to_owned),
            roots: roots.to_vec(),
            private_root: config.private_runtime_state_root(),
        },
        deadline,
    )
    .map_err(|_| "inventory_unproven")?
    {
        Response::Software(result) => result.map_err(|_| "inventory_unproven"),
        _ => Err("inventory_unproven"),
    }
}

pub(super) fn manual_files(
    root: &Path,
    current: &ProjectEnvironment,
    draft: &mut EnvironmentRevisionDraft,
    deadline: Instant,
) -> Result<(), DaemonError> {
    match run_at_root(
        root,
        Request::ManualFiles {
            current: current.clone(),
            draft: draft.clone(),
        },
        deadline,
        || false,
    )? {
        Response::ManualFiles(Ok(measured)) => {
            *draft = measured;
            Ok(())
        }
        Response::ManualFiles(Err(failure)) => Err(failure.error()),
        _ => Err(environment_error("Files review worker result invalid")),
    }
}

pub(super) fn publication_binding(
    config: &crate::config::DaemonConfig,
    root: &Path,
    locator: &ProjectEnvironmentLocator,
    deadline: Instant,
    cancelled: impl FnMut() -> bool,
) -> Result<super::check_binding::PublicBinding, DaemonError> {
    #[cfg(test)]
    let input = Request::PublicationBinding {
        root: Some(root.to_path_buf()),
        locator: locator.clone(),
    };
    #[cfg(not(test))]
    let input = Request::Binding {
        root: Some(root.to_path_buf()),
        locator: locator.clone(),
    };
    match run_with_cancel(config, input, deadline, cancelled)? {
        Response::Binding(facts) => Ok(facts),
        _ => Err(environment_error("Binding reader result invalid")),
    }
}
