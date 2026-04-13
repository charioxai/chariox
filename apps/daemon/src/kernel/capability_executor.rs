use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use tokio::sync::Mutex;

use crate::app::DaemonApp;
use crate::capability::{
    CaptureScreenshotRequest, DirectoryTreeService, EditFileRequest, FileCapabilityService,
    FileTransferService, GitCapabilityService, InspectGitRequest, ReadDirectoryTreeRequest,
    ReadFileRequest, RunShellCommandRequest, ScreenshotCapabilityService, ShellCommandService,
    StoreTransferredFileRequest,
};
use crate::error::DaemonError;
use crate::local::{LocalDaemonRequest, LocalDaemonResponse};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct CapabilityExecutorHealthSnapshot {
    pub submitted_jobs: u64,
    pub running_jobs: u64,
    pub completed_jobs: u64,
    pub failed_jobs: u64,
    pub join_errors: u64,
}

#[derive(Clone, Default)]
pub(crate) struct CapabilityExecutorHealthStore {
    submitted_jobs: Arc<AtomicU64>,
    running_jobs: Arc<AtomicU64>,
    completed_jobs: Arc<AtomicU64>,
    failed_jobs: Arc<AtomicU64>,
    join_errors: Arc<AtomicU64>,
}

impl CapabilityExecutorHealthStore {
    pub(crate) fn snapshot(&self) -> CapabilityExecutorHealthSnapshot {
        CapabilityExecutorHealthSnapshot {
            submitted_jobs: self.submitted_jobs.load(Ordering::Relaxed),
            running_jobs: self.running_jobs.load(Ordering::Relaxed),
            completed_jobs: self.completed_jobs.load(Ordering::Relaxed),
            failed_jobs: self.failed_jobs.load(Ordering::Relaxed),
            join_errors: self.join_errors.load(Ordering::Relaxed),
        }
    }
}

pub(crate) async fn execute_capability_request(
    app: &Arc<Mutex<DaemonApp>>,
    health: CapabilityExecutorHealthStore,
    request: LocalDaemonRequest,
) -> Option<Result<LocalDaemonResponse, DaemonError>> {
    match request {
        LocalDaemonRequest::RunShellCommand(request) => {
            let context =
                capability_context(app, &request.session_id, &request.attachment_id, "shell").await;
            Some(match context {
                Ok(context) => {
                    spawn_capability("run shell command", health, move || {
                        ShellCommandService::new()
                            .run(
                                RunShellCommandRequest::new(
                                    request.session_id,
                                    request.attachment_id,
                                    request.command,
                                    request.args,
                                    context.worktree_root,
                                    request.working_directory,
                                )
                                .with_timeout_ms(request.timeout_ms.unwrap_or(5_000)),
                            )
                            .map(|result| LocalDaemonResponse::ShellCommandCompleted { result })
                    })
                    .await
                }
                Err(error) => Err(error),
            })
        }
        LocalDaemonRequest::ReadDirectoryTree(request) => {
            let context = capability_context(
                app,
                &request.session_id,
                &request.attachment_id,
                "directory_tree",
            )
            .await;
            Some(match context {
                Ok(context) => {
                    spawn_capability("read directory tree", health, move || {
                        DirectoryTreeService::new()
                            .read_tree(ReadDirectoryTreeRequest::new(
                                request.session_id,
                                request.attachment_id,
                                context.worktree_root,
                                request.path,
                                request.max_depth,
                            ))
                            .map(|result| LocalDaemonResponse::DirectoryTreeRead { result })
                    })
                    .await
                }
                Err(error) => Err(error),
            })
        }
        LocalDaemonRequest::ReadFile(request) => {
            let context = capability_context(
                app,
                &request.session_id,
                &request.attachment_id,
                "file_read",
            )
            .await;
            Some(match context {
                Ok(context) => {
                    spawn_capability("read file", health, move || {
                        FileCapabilityService::new()
                            .read_file(ReadFileRequest::new(
                                request.session_id,
                                request.attachment_id,
                                context.worktree_root,
                                request.path,
                            ))
                            .map(|result| LocalDaemonResponse::FileRead { result })
                    })
                    .await
                }
                Err(error) => Err(error),
            })
        }
        LocalDaemonRequest::EditFile(request) => {
            let context = capability_context(
                app,
                &request.session_id,
                &request.attachment_id,
                "file_edit",
            )
            .await;
            Some(match context {
                Ok(context) => {
                    spawn_capability("edit file", health, move || {
                        let _claim = context.workspace_coordinator.acquire_worktree_write_claim(
                            context.workspace_id.clone(),
                            context.worktree_root.display().to_string(),
                            request.session_id.clone(),
                            Some(request.attachment_id.clone()),
                            "file_edit",
                        )?;
                        FileCapabilityService::new()
                            .edit_file(EditFileRequest::new(
                                request.session_id,
                                request.attachment_id,
                                context.worktree_root,
                                request.path,
                                request.contents,
                            ))
                            .map(|result| LocalDaemonResponse::FileEdited { result })
                    })
                    .await
                }
                Err(error) => Err(error),
            })
        }
        LocalDaemonRequest::InspectGit(request) => {
            let context = capability_context(
                app,
                &request.session_id,
                &request.attachment_id,
                "git_inspect",
            )
            .await;
            Some(match context {
                Ok(context) => {
                    spawn_capability("inspect git", health, move || {
                        GitCapabilityService::new()
                            .inspect(InspectGitRequest::new(
                                request.session_id,
                                request.attachment_id,
                                context.worktree_root,
                                request.working_directory,
                            ))
                            .map(|result| LocalDaemonResponse::GitInspected { result })
                    })
                    .await
                }
                Err(error) => Err(error),
            })
        }
        LocalDaemonRequest::CaptureScreenshot(request) => {
            let context = capability_context(
                app,
                &request.session_id,
                &request.attachment_id,
                "screenshot",
            )
            .await;
            Some(match context {
                Ok(context) => {
                    spawn_capability("capture screenshot", health, move || {
                        ScreenshotCapabilityService::new()
                            .capture(CaptureScreenshotRequest::new(
                                request.session_id,
                                request.attachment_id,
                                context.artifact_root("screenshots"),
                            ))
                            .map(|result| LocalDaemonResponse::ScreenshotCaptured { result })
                    })
                    .await
                }
                Err(error) => Err(error),
            })
        }
        LocalDaemonRequest::StoreTransferredFile(request) => {
            let context = capability_context(
                app,
                &request.session_id,
                &request.attachment_id,
                "transfer_store",
            )
            .await;
            Some(match context {
                Ok(context) => {
                    spawn_capability("store transferred file", health, move || {
                        let _claim = context.workspace_coordinator.acquire_worktree_write_claim(
                            context.workspace_id.clone(),
                            context.worktree_root.display().to_string(),
                            request.session_id.clone(),
                            Some(request.attachment_id.clone()),
                            "transfer_store",
                        )?;
                        let artifact_root = context.artifact_root("transfers");
                        FileTransferService::new()
                            .store_file(StoreTransferredFileRequest::new(
                                request.session_id,
                                request.attachment_id,
                                context.worktree_root,
                                artifact_root,
                                request.source_path,
                                request.display_name,
                            ))
                            .map(|result| LocalDaemonResponse::FileTransferred { result })
                    })
                    .await
                }
                Err(error) => Err(error),
            })
        }
        _ => None,
    }
}

#[derive(Debug, Clone)]
struct CapabilityContext {
    session_id: String,
    attachment_id: String,
    workspace_id: String,
    worktree_root: PathBuf,
    workspace_coordinator: crate::kernel::workspace_coordinator::WorkspaceCoordinator,
}

impl CapabilityContext {
    fn artifact_root(&self, category: &str) -> PathBuf {
        DaemonApp::attachment_artifact_root(&self.session_id, &self.attachment_id, category)
    }
}

async fn capability_context(
    app: &Arc<Mutex<DaemonApp>>,
    session_id: &str,
    attachment_id: &str,
    capability: &'static str,
) -> Result<CapabilityContext, DaemonError> {
    let app = app.lock().await;
    let context = app.capability_context(session_id, attachment_id, capability)?;
    let workspace_coordinator = app.workspace_coordinator();
    Ok(CapabilityContext {
        session_id: session_id.to_string(),
        attachment_id: attachment_id.to_string(),
        workspace_id: context.workspace_id,
        worktree_root: context.worktree_root,
        workspace_coordinator,
    })
}

async fn spawn_capability<F>(
    operation: &'static str,
    health: CapabilityExecutorHealthStore,
    task: F,
) -> Result<LocalDaemonResponse, DaemonError>
where
    F: FnOnce() -> Result<LocalDaemonResponse, DaemonError> + Send + 'static,
{
    health.submitted_jobs.fetch_add(1, Ordering::Relaxed);
    health.running_jobs.fetch_add(1, Ordering::Relaxed);
    let joined = tokio::task::spawn_blocking(task).await;
    decrement_saturating(&health.running_jobs);
    match joined {
        Ok(Ok(response)) => {
            health.completed_jobs.fetch_add(1, Ordering::Relaxed);
            Ok(response)
        }
        Ok(Err(error)) => {
            health.failed_jobs.fetch_add(1, Ordering::Relaxed);
            Err(error)
        }
        Err(error) => {
            health.join_errors.fetch_add(1, Ordering::Relaxed);
            Err(DaemonError::LocalTransport {
                operation,
                message: error.to_string(),
            })
        }
    }
}

fn decrement_saturating(value: &AtomicU64) {
    let _ = value.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |current| {
        current.checked_sub(1)
    });
}
