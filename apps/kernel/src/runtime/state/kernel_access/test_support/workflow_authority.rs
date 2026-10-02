use super::session_authority::external_command;
use super::*;
use crate::runtime::workflow_actor::WorkflowRuntime;
use std::sync::Arc;
use tokio::sync::{Mutex, Notify};
use tokio::time::timeout;

#[tokio::test]
async fn kernel_access_workflow_apply_rechecks_app_wait() {
    revoked_workflow(false, false).await;
}

#[tokio::test]
async fn kernel_access_workflow_run_rechecks_app_wait() {
    revoked_workflow(true, false).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn kernel_access_workflow_apply_rechecks_compiler_wait() {
    revoked_workflow(false, true).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn kernel_access_workflow_run_rechecks_compiler_wait() {
    revoked_workflow(true, true).await;
}

async fn revoked_workflow(run: bool, compiler_wait: bool) {
    let worktree = crate::test_support::TestWorktree::new("access-workflow-source");
    let root = crate::test_support::TestWorktree::new("access-workflow-state");
    let mut config = crate::config::DaemonConfig::for_tests();
    config.user_config.state.path = Some(root.path().join("state.db").display().to_string());
    let artifact_root = config.workflow_code_artifact_root();
    let mut daemon = crate::test_support::bootstrap_authenticated_app(config).unwrap();
    let (session, meta) = crate::app::KernelSessionService::new(&mut daemon)
        .create_session(worktree.session_request())
        .unwrap();
    if run {
        daemon
            .agents_mut()
            .activate_agent_meta_mode(meta.id(), None)
            .unwrap();
        daemon
            .sessions_mut()
            .start_or_update_metaagent_task(session.id(), meta.id(), "hold workflow invocation")
            .unwrap();
    }
    let app = Arc::new(Mutex::new(daemon));
    let router =
        crate::runtime::router::CommandRouter::with_interactive_capacity_from_app(app.clone(), 32);
    let mut state = router.runtime_state();
    let probe = Arc::new(Notify::new());
    state.observe_app_lock_wait_for_test(probe.clone());
    let runtime = WorkflowRuntime::new(
        state.clone(),
        state.owned.session_projection.clone(),
        state.owned.agent_runtime_projection.clone(),
    );
    let grant = state.insert_access_grant_for_test(session.id());
    let source = r#"
workflow.define({ alias: "external-authority" })
const worker = workflow.node({ handle: "worker", agent: workflow.newAgent({ alias: "generated", provider: "dev-stub", model: "default" }), instructions: "Finish.", canCompleteWorkflowRun: true })
workflow.endpoint(worker, { handle: "entry", alias: "entry" })
"#.to_string();
    let started = root.path().join("compile-started");
    let release = root.path().join("compile-release");
    let node_path = if compiler_wait {
        use std::os::unix::fs::PermissionsExt;
        let wrapper = root.path().join("node-gate");
        std::fs::write(&wrapper, format!("#!/bin/sh\nprintf started > '{}'\nwhile [ ! -f '{}' ]; do sleep 0.02; done\nexec node \"$@\"\n", started.display(), release.display())).unwrap();
        std::fs::set_permissions(&wrapper, std::fs::Permissions::from_mode(0o700)).unwrap();
        wrapper.display().to_string()
    } else {
        "node".into()
    };
    let request = if run {
        LocalDaemonRequest::RunWorkflowCode(crate::local::RunWorkflowCodeRequest {
            session_id: session.id().into(),
            node_path: node_path.clone(),
            source,
            language: None,
            provider_rebindings: vec![],
            agent_rebindings: vec![],
            endpoint: Some("entry".into()),
            queue_ref: None,
            prompt: "run after Meta".into(),
        })
    } else {
        LocalDaemonRequest::ApplyWorkflowCode(crate::local::ApplyWorkflowCodeRequest {
            session_id: session.id().into(),
            node_path: node_path.clone(),
            source,
            language: None,
            provider_rebindings: vec![],
            agent_rebindings: vec![],
        })
    };
    let before = state.session_snapshot(session.id()).await.unwrap();
    let registry = crate::workflow_code::WorkflowCodeArtifactRegistry::new(vec![artifact_root]);
    let artifacts = registry.list().unwrap();
    let guard = if compiler_wait {
        None
    } else {
        Some(app.lock().await)
    };
    let pending = tokio::spawn({
        let runtime = runtime.clone();
        let request = request.clone();
        let command = external_command(&request, &grant);
        async move { runtime.dispatch_workflow_command(command, request).await }
    });
    if compiler_wait {
        timeout(Duration::from_secs(5), async {
            while !started.exists() {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
    } else {
        timeout(Duration::from_secs(3), probe.notified())
            .await
            .unwrap();
    }
    state
        .revoke_kernel_access(None, Some(&grant), "explicit_revoke")
        .unwrap();
    drop(guard);
    if compiler_wait {
        std::fs::write(&release, "release").unwrap();
    }
    let result = timeout(Duration::from_secs(5), pending)
        .await
        .unwrap()
        .unwrap();
    let unchanged = state.session_snapshot(session.id()).await.unwrap();
    assert_eq!(
        unchanged.workflows().len(),
        before.workflows().len(),
        "revoked workflow created a definition"
    );
    assert_eq!(
        unchanged.agents().len(),
        before.agents().len(),
        "revoked workflow created an agent"
    );
    assert!(
        unchanged == before,
        "revoked workflow changed session/agents/runs"
    );
    assert_eq!(
        registry.list().unwrap(),
        artifacts,
        "revoked workflow saved an artifact"
    );
    assert!(result
        .unwrap_err()
        .to_string()
        .contains("grant revoked or expired"));
    let command = crate::runtime::command::KernelCommand::from_local_request(
        "terminal-workflow-control",
        None,
        None,
        &request,
    );
    let response = runtime
        .dispatch_workflow_command(command, request)
        .await
        .unwrap();
    assert!(if run {
        matches!(response, LocalDaemonResponse::WorkflowCodeRun { .. })
    } else {
        matches!(response, LocalDaemonResponse::WorkflowCodeApplied { .. })
    });
    let after = state.session_snapshot(session.id()).await.unwrap();
    assert_eq!(after.workflows().len(), before.workflows().len() + 1);
    assert_eq!(after.agents().len(), before.agents().len() + 1);
    if run {
        assert_eq!(after.workflow_queued_prompts().len(), 1);
    } else {
        assert_eq!(registry.list().unwrap().len(), artifacts.len() + 1);
    }
}
