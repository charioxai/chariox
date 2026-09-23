//! Focused local API coverage for slice save admission.

#[cfg(unix)]
use super::*;
#[cfg(unix)]
use std::ffi::OsString;
#[cfg(unix)]
use std::sync::{Arc, Mutex as StdMutex};
#[cfg(unix)]
use tokio::sync::Mutex;

#[cfg(unix)]
static SLICE_DOCKER_ENV_LOCK: StdMutex<()> = StdMutex::new(());

#[cfg(unix)]
struct SliceDockerEnv(Vec<(&'static str, Option<OsString>)>);

#[cfg(unix)]
impl SliceDockerEnv {
    fn use_isolated_provisioner(path: &std::path::Path) -> Self {
        let names = [
            "CHARIOX_SLICE_DOCKER_PROVISIONER",
            "CHARIOX_SLICE_DOCKER_BROKER_SOCKET",
            "CHARIOX_SLICE_DOCKER_BROKER_FD",
            "CHARIOX_SLICE_DOCKER_BROKER_REQUIRED",
        ];
        let previous = names
            .into_iter()
            .map(|name| (name, std::env::var_os(name)))
            .collect();
        for name in &names[1..] {
            std::env::remove_var(*name);
        }
        std::env::set_var("CHARIOX_SLICE_DOCKER_PROVISIONER", path);
        Self(previous)
    }
}

#[cfg(unix)]
impl Drop for SliceDockerEnv {
    fn drop(&mut self) {
        for (name, previous) in self.0.drain(..) {
            match previous {
                Some(previous) => std::env::set_var(name, previous),
                None => std::env::remove_var(name),
            }
        }
    }
}

#[cfg(unix)]
#[tokio::test(flavor = "current_thread")]
async fn save_api_rejects_reconciled_running_provider_before_snapshot_or_park() {
    use std::os::unix::fs::PermissionsExt;

    let _environment_lock = SLICE_DOCKER_ENV_LOCK
        .lock()
        .expect("Docker provisioner test lock should not be poisoned");
    let scratch = tempfile::tempdir().expect("private test root should be created");
    let action_log = scratch.path().join("docker-actions.log");
    let provisioner = scratch.path().join("slice-provisioner.sh");
    std::fs::write(
        &provisioner,
        format!(
            "#!/bin/sh\nprintf '%s\\n' \"$*\" >> '{}'\nexit 73\n",
            action_log.display()
        ),
    )
    .expect("isolated provisioner should be written");
    std::fs::set_permissions(&provisioner, std::fs::Permissions::from_mode(0o700))
        .expect("isolated provisioner should be executable");
    let _provisioner_env = SliceDockerEnv::use_isolated_provisioner(&provisioner);

    let (app, runtime, slice, session_id, agent_id, projected_run_id, worker_run_id) =
        busy_slice_runtime().await;
    let config_projection = app.lock().await.config_projection_store();
    let result = execute_slice_request(
        &runtime,
        &config_projection,
        None,
        &crate::runtime::command::KernelCaller::default(),
        None,
        crate::local::LocalDaemonRequest::SaveSliceState(
            crate::local::SliceStateSaveRequest {
                slice_ref: slice.id.clone(),
                mode: Some(crate::local::SliceStateSaveMode::RestartAgents),
                scope: Some(crate::local::SliceStateSaveScope::ThisSlice),
            },
        ),
    )
    .await;

    let error = result.expect_err("a running attached provider must reject SaveSliceState");
    assert!(error
        .to_string()
        .contains("cannot save slice while agents are running"));
    assert!(error.to_string().contains(&agent_id));

    let app = app.lock().await;
    let saved = app
        .slices()
        .resolve(&slice.id)
        .expect("slice should remain available");
    assert!(
        saved.agent_ids.iter().any(|attached| attached == &agent_id),
        "save admission must see the canonical agent restored by stale-attachment reconciliation"
    );
    assert_eq!(saved.status, crate::slice::SliceStatus::Running);
    assert_eq!(saved.saved_state_ref, None);
    assert_eq!(saved.saved_state_status, None);
    assert!(
        app.slices()
            .active_saved_state_for_slice(&slice.id)
            .expect("saved-state lookup should succeed")
            .is_none(),
        "busy admission must not publish saved-state metadata"
    );
    assert!(
        !action_log.exists(),
        "busy admission must not call the Docker save/volume provisioner"
    );
    assert_eq!(
        app.provider_run_projection_store()
            .get(&projected_run_id)
            .expect("projected provider run should remain available")
            .state(),
        crate::provider::ProviderRunState::Running,
        "busy admission must not park or end the active provider"
    );
    assert_eq!(
        app.session_state_store()
            .get_session(&session_id)
            .expect("session should remain available")
            .active_provider_run_id(),
        Some(projected_run_id.as_str())
    );
    assert_eq!(
        app.agents()
            .get_agent(&agent_id)
            .expect("agent should remain available")
            .remote_execution()
            .expect("slice worker binding should remain available")
            .active_worker_provider_run_id
            .as_deref(),
        Some(worker_run_id.as_str())
    );
}

#[cfg(unix)]
async fn busy_slice_runtime() -> (
    Arc<Mutex<crate::app::DaemonApp>>,
    crate::runtime::state::KernelRuntimeState,
    crate::slice::SliceRecord,
    String,
    String,
    String,
    String,
) {
    let mut app = crate::app::DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests())
        .expect("daemon bootstrap should succeed");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(crate::session::CreateSessionRequest::new(
            "workspace-1",
            "worktree-1",
        ))
        .expect("session should be created");
    let session_id = session.id().to_string();
    let agent_id = agent.id().to_string();
    let worker_run_id = "worker-provider-run".to_string();
    let projected_run_id = crate::provider::projected_leased_provider_run_id(
        "leased-agent-1",
        &worker_run_id,
    );
    let slice = app
        .slices()
        .create(
            "owner-kernel-1",
            "owner-machine-1",
            crate::slice::CreateSliceInput {
                name: "slice-1".to_string(),
                backend: crate::slice::SliceBackendKind::LocalDocker,
                os: "linux".to_string(),
                display_mode: crate::slice::SliceDisplayMode::Headless,
                display_backend: Default::default(),
                workspace_id: Some("workspace-1".to_string()),
                worktree_id: Some("worktree-1".to_string()),
                workspace_mount: None,
                development: None,
                worker_kernel_ref: Some("worker-kernel-1".to_string()),
                display_url: None,
                provider_auth: Vec::new(),
                from_saved_state: None,
                now_ms: 1,
            },
        )
        .expect("slice should be created");
    app.slices()
        .set_status(&slice.id, crate::slice::SliceStatus::Running, 2)
        .expect("slice should be marked running for save admission");
    let slice = app
        .slices()
        .set_worker_presence(
            &slice.id,
            Some("worker-kernel-1".to_string()),
            Some("machine-slice-1".to_string()),
            vec!["codex".to_string()],
            3,
        )
        .expect("slice worker presence should be available");
    app.agents_mut()
        .set_agent_runtime_profile_with_account_profile(
            &agent_id,
            "codex",
            Some("gpt-5.6-sol".to_string()),
            None,
            Some("work".to_string()),
            crate::provider::ProviderResumeState::from_codex_thread_id("thread-before-save"),
        )
        .expect("agent provider session should be recorded");
    app.agents_mut()
        .bind_remote_execution(
            &agent_id,
            crate::agent::RemoteAgentBinding {
                worker_kernel_id: "worker-kernel-1".to_string(),
                worker_machine_id: "machine-slice-1".to_string(),
                execution_lease_id: "lease-1".to_string(),
                leased_agent_id: "leased-agent-1".to_string(),
                active_worker_provider_run_id: Some(worker_run_id.clone()),
                relay_url: None,
                relay_token: None,
                relay_peer_protocol_version: Some(
                    crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION,
                ),
            },
        )
        .expect("agent should bind to the slice worker");
    let launch_request = crate::provider::LaunchProviderRequest::new(
        &session_id,
        "codex",
        "codex",
        "work",
        "gpt-5.6-sol",
    )
    .with_agent_id(&agent_id)
    .with_resume_state(crate::provider::ProviderResumeState::from_codex_thread_id(
        "thread-before-save",
    ));
    let mut projected_run = crate::provider::RuntimeProviderRun::new(
        &projected_run_id,
        &launch_request,
        crate::provider::ProviderLaunchResult {
            endpoint_mode: crate::provider::AgentEndpointMode::Managed,
            process_label: "busy-projected-codex".to_string(),
            pty_target: None,
            pty_program: None,
            pty_args: Vec::new(),
            pty_env: Default::default(),
            pty_env_remove: Vec::new(),
            working_directory: Some(std::path::PathBuf::from("/workspace")),
            structured_endpoint: Some("http://worker.invalid".to_string()),
        },
    );
    projected_run.mark_running();
    app.provider_run_projection_store().update(projected_run);
    app.session_state_store()
        .set_active_provider_run(&session_id, Some(projected_run_id.clone()))
        .expect("provider run should be active in the home session");
    app.prompt_owner_sync_external_active_prompt(
        &session_id,
        &agent_id,
        Some(crate::session::PromptQueueItem::new(
            "active-prompt",
            "attachment-1",
            &agent_id,
            "active provider turn",
            crate::session::PromptStatus::Running,
        )),
    )
    .expect("active provider prompt should be recorded");

    let app = Arc::new(Mutex::new(app));
    let runtime = owned_runtime_state(&app).await;
    (
        app,
        runtime,
        slice,
        session_id,
        agent_id,
        projected_run_id,
        worker_run_id,
    )
}

#[cfg(unix)]
async fn owned_runtime_state(
    app: &Arc<Mutex<crate::app::DaemonApp>>,
) -> crate::runtime::state::KernelRuntimeState {
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
    crate::runtime::state::KernelRuntimeState::new_with_owned_state(
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
