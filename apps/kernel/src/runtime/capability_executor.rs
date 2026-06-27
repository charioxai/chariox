use crate::capability::{
    CaptureScreenshotRequest, DirectoryTreeService, EditFileRequest, FileCapabilityService,
    GitCapabilityService, InspectGitRequest, ReadDirectoryTreeRequest, ReadFileRequest,
    RunShellCommandRequest, ScreenshotCapabilityService, ShellCommandService,
};
use crate::error::DaemonError;
use crate::local::{LocalDaemonRequest, LocalDaemonResponse};

mod context;
mod health;
mod transfer;
pub(crate) use context::CapabilityRuntimeStore;
use health::spawn_capability;
pub(crate) use health::{CapabilityExecutorHealthSnapshot, CapabilityExecutorHealthStore};
use transfer::store_transferred_file;

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
                        store_transferred_file(context, request)
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
        let (
            config_projection,
            session_store,
            agent_store,
            attachment_store,
            provider_store,
            provider_process_tracking,
            slice_store,
            session_projection,
            provider_run_projection,
            history_store,
            operational_history_store,
            durable_state_store,
            prompt_state_owner,
            active_turns,
            prompt_activity,
            prompt_workspace_claims,
            structured_output_records,
            terminal_stream,
            workflow_design_events,
            metaagent_events,
            workspace_coordinator,
        ) = {
            let app_locked = app.lock().await;
            (
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
                app_locked.prompt_state_owner(),
                app_locked.active_turn_store(),
                app_locked.prompt_activity_store(),
                app_locked.prompt_workspace_claim_store(),
                app_locked.structured_output_record_store(),
                app_locked.terminal_stream_store(),
                app_locked.workflow_design_event_store(),
                app_locked.metaagent_event_store(),
                app_locked.workspace_coordinator(),
            )
        };
        KernelRuntimeState::new_with_owned_state(
            Arc::clone(app),
            config_projection,
            session_store,
            agent_store,
            attachment_store,
            provider_store,
            provider_process_tracking,
            slice_store,
            session_projection,
            provider_run_projection,
            history_store,
            operational_history_store,
            durable_state_store,
            prompt_state_owner,
            active_turns,
            prompt_activity,
            prompt_workspace_claims,
            structured_output_records,
            terminal_stream,
            workflow_design_events,
            metaagent_events,
            workspace_coordinator,
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
