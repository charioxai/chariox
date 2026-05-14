use std::collections::BTreeMap;
use std::path::PathBuf;

use crate::app::DaemonApp;
use crate::artifacts::{OperationalArtifactStore, StoreArtifactRequest};
use crate::capability::{
    CaptureScreenshotRequest, DirectoryTreeService, EditFileRequest, FileCapabilityService,
    FileTransferService, GitCapabilityService, InspectGitRequest, ReadDirectoryTreeRequest,
    ReadFileRequest, RunShellCommandRequest, ScreenshotCapabilityService, ShellCommandService,
    StoreTransferredFileRequest,
};
use crate::error::DaemonError;
use crate::history::{HistoryEventKind, HistoryEventRole, HistoryEventTurnContext};
use crate::local::{LocalDaemonRequest, LocalDaemonResponse};
use crate::runtime::state::KernelRuntimeState;

mod health;
use health::spawn_capability;
pub(crate) use health::{CapabilityExecutorHealthSnapshot, CapabilityExecutorHealthStore};

pub(crate) async fn execute_capability_request(
    store: &CapabilityRuntimeStore,
    health: CapabilityExecutorHealthStore,
    request: LocalDaemonRequest,
) -> Option<Result<LocalDaemonResponse, DaemonError>> {
    match request {
        LocalDaemonRequest::RunShellCommand(request) => {
            let context = store
                .context(&request.session_id, &request.attachment_id, "shell")
                .await;
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
            let context = store
                .context(
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
            let context = store
                .context(&request.session_id, &request.attachment_id, "file_read")
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
            let context = store
                .context(&request.session_id, &request.attachment_id, "file_edit")
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
            let context = store
                .context(&request.session_id, &request.attachment_id, "git_inspect")
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
            let context = store
                .context(&request.session_id, &request.attachment_id, "screenshot")
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
            let context = store
                .context(
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
                        let result = FileTransferService::new().store_file(
                            StoreTransferredFileRequest::new(
                                request.session_id.clone(),
                                request.attachment_id.clone(),
                                context.worktree_root.clone(),
                                artifact_root,
                                request.source_path,
                                request.display_name,
                            ),
                        )?;
                        let artifact_store = OperationalArtifactStore::open(
                            context.operational_artifact_root,
                            context.operational_artifact_index_path,
                        )?;
                        let mut metadata = BTreeMap::new();
                        metadata.insert(
                            "transfer_artifact_id".to_string(),
                            serde_json::Value::String(result.artifact_id.clone()),
                        );
                        metadata.insert(
                            "stored_path".to_string(),
                            serde_json::Value::String(result.stored_path.display().to_string()),
                        );
                        metadata.insert(
                            "stored_name".to_string(),
                            serde_json::Value::String(result.stored_name.clone()),
                        );
                        let artifact_record =
                            artifact_store.store_existing_file(StoreArtifactRequest {
                                source_path: result.stored_path.clone(),
                                display_name: result.display_name.clone(),
                                source_kind: "transfer".to_string(),
                                session_id: Some(request.session_id.clone()),
                                attachment_id: Some(request.attachment_id.clone()),
                                workspace_id: Some(context.workspace_id.clone()),
                                worktree_path: Some(context.worktree_root.display().to_string()),
                                metadata: metadata.clone(),
                            })?;
                        metadata.insert(
                            "artifact_id".to_string(),
                            serde_json::Value::String(artifact_record.artifact_id.clone()),
                        );
                        metadata.insert(
                            "sha256".to_string(),
                            serde_json::Value::String(artifact_record.sha256.clone()),
                        );
                        metadata.insert(
                            "size_bytes".to_string(),
                            serde_json::Value::Number(serde_json::Number::from(
                                artifact_record.size_bytes,
                            )),
                        );
                        let sequence = context.operational_history_store.reserve_sequence();
                        let mut event = crate::history::HistoryEvent::operational(
                            sequence,
                            HistoryEventKind::ArtifactStored,
                            Some(HistoryEventRole::System),
                            Some(format!(
                                "stored artifact `{}` ({})",
                                artifact_record.display_name, artifact_record.artifact_id
                            )),
                            metadata,
                            HistoryEventTurnContext {
                                workspace_id: Some(context.workspace_id.clone()),
                                session_id: Some(request.session_id.clone()),
                                worktree_path: Some(context.worktree_root.display().to_string()),
                                ..HistoryEventTurnContext::default()
                            },
                        );
                        event.content_ref =
                            Some(format!("artifact://sha256/{}", artifact_record.sha256));
                        context.operational_history_store.append(&event)?;
                        if context.history_archive_enabled {
                            context
                                .operational_history_store
                                .enqueue_archive_events(&[event])?;
                        }
                        Ok(LocalDaemonResponse::FileTransferred { result })
                    })
                    .await
                }
                Err(error) => Err(error),
            })
        }
        _ => None,
    }
}

pub(crate) async fn execute_required_capability_request(
    store: &CapabilityRuntimeStore,
    health: CapabilityExecutorHealthStore,
    request: LocalDaemonRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    execute_capability_request(store, health, request)
        .await
        .unwrap_or_else(|| {
            Err(DaemonError::LocalTransport {
                operation: "route capability request",
                message: "capability request was not handled by executor".to_string(),
            })
        })
}

#[derive(Debug, Clone)]
struct CapabilityContext {
    session_id: String,
    attachment_id: String,
    workspace_id: String,
    worktree_root: PathBuf,
    workspace_coordinator: crate::runtime::workspace_coordinator::WorkspaceCoordinator,
    operational_history_store: crate::history::OperationalHistoryStore,
    operational_artifact_root: PathBuf,
    operational_artifact_index_path: PathBuf,
    history_archive_enabled: bool,
}

impl CapabilityContext {
    fn artifact_root(&self, category: &str) -> PathBuf {
        DaemonApp::attachment_artifact_root(&self.session_id, &self.attachment_id, category)
    }
}

#[derive(Clone)]
pub(crate) struct CapabilityRuntimeStore {
    state: KernelRuntimeState,
}

impl CapabilityRuntimeStore {
    pub(crate) fn new(state: KernelRuntimeState) -> Self {
        Self { state }
    }

    async fn context(
        &self,
        session_id: &str,
        attachment_id: &str,
        capability: &'static str,
    ) -> Result<CapabilityContext, DaemonError> {
        let snapshot = self
            .state
            .capability_context(session_id, attachment_id, capability)
            .await?;
        Ok(CapabilityContext {
            session_id: session_id.to_string(),
            attachment_id: attachment_id.to_string(),
            workspace_id: snapshot.workspace_id,
            worktree_root: snapshot.worktree_root,
            workspace_coordinator: snapshot.workspace_coordinator,
            operational_history_store: snapshot.operational_history_store,
            operational_artifact_root: snapshot.operational_artifact_root,
            operational_artifact_index_path: snapshot.operational_artifact_index_path,
            history_archive_enabled: snapshot.history_archive_enabled,
        })
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use tokio::sync::Mutex;

    use super::{
        execute_capability_request, CapabilityExecutorHealthStore, CapabilityRuntimeStore,
    };
    use crate::attachment::{AttachRequest, ClientCapabilityLevel};
    use crate::error::DaemonError;
    use crate::local::{LocalDaemonRequest, RunShellCapabilityRequest};
    use crate::runtime::state::KernelRuntimeState;
    use crate::session::CreateSessionRequest;
    use crate::{DaemonApp, DaemonConfig};

    async fn owned_runtime_state(app: &Arc<Mutex<DaemonApp>>) -> KernelRuntimeState {
        let app_locked = app.lock().await;
        KernelRuntimeState::new_with_owned_state(
            Arc::clone(app),
            app_locked.config_projection_store(),
            app_locked.session_state_store(),
            app_locked.agents().clone(),
            app_locked.attachments().clone(),
            app_locked.providers().clone(),
            app_locked.provider_process_tracking_store(),
            app_locked.slices(),
            app_locked.session_state_projection_store(),
            app_locked.provider_run_projection_store(),
            app_locked.history_store(),
            app_locked.operational_history_store(),
            app_locked.durable_state_store(),
            app_locked.session_history_projection_store(),
            app_locked.prompt_state_owner(),
            app_locked.active_turn_store(),
            app_locked.prompt_activity_store(),
            app_locked.prompt_workspace_claim_store(),
            app_locked.structured_output_record_store(),
            app_locked.terminal_stream_store(),
            app_locked.workspace_coordinator(),
        )
    }

    #[tokio::test]
    async fn capability_executor_rejects_when_concurrency_limit_is_exhausted() {
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
        let (session, _agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(CreateSessionRequest::new("workspace", "worktree"))
            .expect("session should be created");
        let attachment = crate::app::KernelSessionService::new(&mut app)
            .attach(AttachRequest::new(
                session.id(),
                "cli-capability-overload",
                ClientCapabilityLevel::FullTerminal,
            ))
            .expect("attachment should attach");
        let app = Arc::new(Mutex::new(app));
        let health = CapabilityExecutorHealthStore::new(0);

        let response = execute_capability_request(
            &CapabilityRuntimeStore::new(owned_runtime_state(&app).await),
            health.clone(),
            LocalDaemonRequest::RunShellCommand(RunShellCapabilityRequest {
                session_id: session.id().to_string(),
                attachment_id: attachment.id().to_string(),
                command: "/bin/true".to_string(),
                args: Vec::new(),
                working_directory: None,
                timeout_ms: Some(1_000),
            }),
        )
        .await
        .expect("shell command should be a capability request");

        match response.expect_err("overloaded executor should reject work") {
            DaemonError::LocalTransport { operation, message } => {
                assert_eq!(operation, "run shell command");
                assert_eq!(message, "capability executor is overloaded");
            }
            error => panic!("unexpected error: {error}"),
        }

        let snapshot = health.snapshot();
        assert_eq!(snapshot.max_concurrent_jobs, 0);
        assert_eq!(snapshot.available_permits, 0);
        assert_eq!(snapshot.submitted_jobs, 0);
        assert_eq!(snapshot.running_jobs, 0);
        assert_eq!(snapshot.completed_jobs, 0);
        assert_eq!(snapshot.failed_jobs, 0);
        assert_eq!(snapshot.rejected_jobs, 1);
        assert_eq!(snapshot.join_errors, 0);
    }
}
