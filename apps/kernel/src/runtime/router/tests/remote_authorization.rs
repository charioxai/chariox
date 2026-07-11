use super::*;
use crate::local::UpdateWorkflowNodeInstructionsRequest;

fn run_remote_authorization_large_stack_test<Fut>(name: &str, test: fn() -> Fut)
where
    Fut: std::future::Future<Output = ()> + 'static,
{
    std::thread::Builder::new()
        .name(name.to_string())
        .stack_size(64 * 1024 * 1024)
        .spawn(move || {
            tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .thread_stack_size(64 * 1024 * 1024)
                .enable_all()
                .build()
                .expect("remote authorization test runtime should build")
                .block_on(test());
        })
        .expect("remote authorization test thread should spawn")
        .join()
        .expect("remote authorization test thread should not panic");
}

fn home_extension_script_workspace(label: &str) -> std::path::PathBuf {
    let workspace = std::env::temp_dir().join(format!(
        "arroba-home-extension-{label}-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock should be after unix epoch")
            .as_nanos()
    ));
    let script_dir = workspace.join(".arroba").join("scripts").join("home-tool");
    std::fs::create_dir_all(&script_dir).expect("script dir should be created");
    std::fs::write(
        script_dir.join("metadata.json"),
        r#"{
  "name": "home-tool",
  "runtime": "python",
  "entrypoint": "script.py",
  "description": "Home-owned replay test script",
  "input_schema": {"type": "object", "properties": {}},
  "definition_hash": "replay-test-hash",
  "timeout_sec": 10
}
"#,
    )
    .expect("script metadata should be written");
    std::fs::write(
        script_dir.join("script.py"),
        "def run():\n    return {\"executed\": True}\n",
    )
    .expect("script should be written");
    let env_dir = workspace.join(".arroba").join("envs");
    std::fs::create_dir_all(&env_dir).expect("env dir should be created");
    std::fs::write(
        env_dir.join("test-env.json"),
        r#"{
  "name": "test-env",
  "runtime": {"type": "python", "python": "/usr/bin/python3"}
}
"#,
    )
    .expect("environment should be written");
    workspace
}

fn home_extension_script_router(
    workspace: &std::path::Path,
    alias: &str,
) -> (
    CommandRouter,
    crate::transport::relay_peer::RemoteExtensionInvocationContext,
    crate::extension::RemoteExtensionInvocationMetadata,
    crate::extension::RemoteExtensionTool,
    String,
) {
    let config = DaemonConfig::for_tests();
    let home_kernel_id = config.daemon_id.clone();
    let mut app = DaemonApp::bootstrap(config).expect("daemon should boot");
    let session = app
        .sessions_mut()
        .create_session(CreateSessionRequest::new(
            workspace.to_string_lossy(),
            workspace.to_string_lossy(),
        ))
        .expect("session should be created");
    let session_id = session.id().to_string();
    let agent = spawn_test_agent(&mut app, &session_id, alias, "dev-stub");
    app.agents()
        .bind_remote_execution(
            agent.id(),
            crate::agent::RemoteAgentBinding {
                worker_kernel_id: "worker-kernel".to_string(),
                worker_machine_id: "worker-machine".to_string(),
                execution_lease_id: "lease-1".to_string(),
                leased_agent_id: "leased-agent-1".to_string(),
                active_worker_provider_run_id: Some("provider-run-1".to_string()),
                relay_url: None,
                relay_token: None,
            },
        )
        .expect("agent should be remote-backed");
    let granted_agent = app
        .agents()
        .grant_extension(
            agent.id(),
            crate::extension::ExtensionGrant::script("home-tool", "test-env"),
        )
        .expect("script grant should be recorded");
    let hinted_tool = app
        .remote_extension_manifest_for_agent(&granted_agent)
        .expect("home manifest should be rebuilt from current state")
        .home_proxy_tool("home-tool")
        .expect("home script should be projected")
        .clone();
    let agent_id = agent.id().to_string();
    let app = Arc::new(Mutex::new(app));
    let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 4);
    let metadata = crate::extension::RemoteExtensionInvocationMetadata::new(
        "provider-run-1",
        "home-tool",
        None,
    );
    let context = crate::transport::relay_peer::RemoteExtensionInvocationContext {
        home_kernel_id,
        home_session_id: session_id,
        home_agent_id: agent_id.clone(),
        leased_agent_id: "leased-agent-1".to_string(),
        worker_provider_run_id: "provider-run-1".to_string(),
        worker_kernel_id: Some("worker-kernel".to_string()),
        worker_machine_id: Some("worker-machine".to_string()),
    };
    (router, context, metadata, hinted_tool, agent_id)
}

fn remote_home_invocation_router_with_active_prompt(
    label: &str,
) -> (
    CommandRouter,
    crate::transport::relay_peer::RemoteExtensionInvocationContext,
    String,
) {
    let config = DaemonConfig::for_tests();
    let home_kernel_id = config.daemon_id.clone();
    let mut app = DaemonApp::bootstrap(config).expect("daemon should boot");
    let session = app
        .sessions_mut()
        .create_session(CreateSessionRequest::new(
            format!("workspace-{label}"),
            format!("worktree-{label}"),
        ))
        .expect("session should be created");
    let session_id = session.id().to_string();
    let attachment = app
        .attach(crate::attachment::AttachRequest::new(
            &session_id,
            format!("client-{label}"),
            crate::attachment::ClientCapabilityLevel::FullTerminal,
        ))
        .expect("attachment should be created");
    let agent = spawn_test_agent(&mut app, &session_id, label, "dev-stub");
    app.agents()
        .bind_remote_execution(
            agent.id(),
            crate::agent::RemoteAgentBinding {
                worker_kernel_id: "worker-kernel".to_string(),
                worker_machine_id: "worker-machine".to_string(),
                execution_lease_id: "lease-1".to_string(),
                leased_agent_id: "leased-agent-1".to_string(),
                active_worker_provider_run_id: None,
                relay_url: None,
                relay_token: None,
            },
        )
        .expect("agent should be remote-backed");
    let prompt = crate::session::PromptQueueItem::new(
        app.sessions_mut().reserve_prompt_id(),
        attachment.id(),
        agent.id(),
        "active remote prompt",
        crate::session::PromptStatus::Queued,
    );
    assert!(matches!(
        app.prompt_owner_submit_prepared_prompt(&session_id, prompt, false)
            .expect("active prompt should be recorded"),
        crate::session::PromptSubmissionOutcome::Started { .. }
    ));
    let context = crate::transport::relay_peer::RemoteExtensionInvocationContext {
        home_kernel_id,
        home_session_id: session_id,
        home_agent_id: agent.id().to_string(),
        leased_agent_id: "leased-agent-1".to_string(),
        worker_provider_run_id: "provider-run-first".to_string(),
        worker_kernel_id: Some("worker-kernel".to_string()),
        worker_machine_id: Some("worker-machine".to_string()),
    };
    let agent_id = agent.id().to_string();
    let app = Arc::new(Mutex::new(app));
    (
        CommandRouter::with_interactive_capacity(Arc::clone(&app), 4),
        context,
        agent_id,
    )
}

mod home_extension_credentials;
mod home_extension_denials;
mod home_extension_grants;
mod home_extension_replay;
mod membership;
mod ownership;
mod projection_redaction;
