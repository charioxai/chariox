//! MP-08 / MP-10 / MP-11: native composer hooks use the production Skills bridge.
use super::*;
use crate::{
    app::{KernelSessionService, ProviderOutputClaudeNativeBridge},
    provider::{
        LaunchProviderRequest, ProviderClientInterface, ProviderLaunchResult, RuntimeProviderRun,
    },
    skill::{service::SkillCatalogService, storage::SkillCatalogStorage, v2::*},
};
use std::{fs, path::PathBuf};

struct Composer {
    runtime: KernelRuntimeState,
    run: RuntimeProviderRun,
    session_id: String,
    agent_id: String,
    files: crate::test_support::TestWorktree,
    storage: SkillCatalogStorage,
}
impl Composer {
    async fn new() -> Self {
        let files = crate::test_support::TestWorktree::new("typed-native-skill");
        let mut app = DaemonApp::bootstrap(crate::DaemonConfig::for_tests()).unwrap();
        let (session, agent) = KernelSessionService::new(&mut app)
            .create_session(files.session_request())
            .unwrap();
        KernelSessionService::new(&mut app)
            .attach(crate::attachment::AttachRequest::new(
                session.id(),
                "typed-client",
                crate::attachment::ClientCapabilityLevel::FullTerminal,
            ))
            .unwrap();
        let context = files.path().join("context.txt");
        let events = files.path().join("events.jsonl");
        fs::write(&context, "").unwrap();
        fs::write(&events, "").unwrap();
        let request =
            LaunchProviderRequest::new(session.id(), "claude", "claude", "default", "haiku")
                .with_agent_id(agent.id())
                .with_client_interface(ProviderClientInterface::NativeTui);
        let mut run = RuntimeProviderRun::new(
            "typed-native-run",
            &request,
            ProviderLaunchResult {
                endpoint_mode: crate::provider::AgentEndpointMode::Managed,
                process_label: "typed-composer-test".into(),
                pty_target: Some("typed-native-pty".into()),
                pty_program: Some("/bin/sh".into()),
                pty_args: vec!["-c".into(), "exec cat".into()],
                pty_env: [
                    ("HOME".into(), files.path().display().to_string()),
                    (
                        "CLAUDE_CONFIG_DIR".into(),
                        files.path().join("claude").display().to_string(),
                    ),
                    (
                        "CHARIOX_CLAUDE_NATIVE_CONTEXT".into(),
                        context.display().to_string(),
                    ),
                    (
                        "CHARIOX_CLAUDE_NATIVE_EVENTS".into(),
                        events.display().to_string(),
                    ),
                    ("CHARIOX_CLAUDE_NATIVE_CONTEXT_RECEIPTS".into(), "1".into()),
                ]
                .into(),
                pty_env_remove: Vec::new(),
                working_directory: Some(files.path().into()),
                structured_endpoint: None,
            },
        );
        run.mark_running();
        app.pty_mut().spawn_for_run(&run).unwrap();
        app.providers_mut().insert_run_for_test(run.clone());
        app.sessions
            .set_active_provider_run(session.id(), Some(run.id().into()))
            .unwrap();
        let runtime = super::tests::owned_runtime_state(&Arc::new(Mutex::new(app))).await;
        runtime
            .owned
            .skill_inventories
            .set_test_version(run.id(), "2.1.212");
        let cfg = runtime.owned.config_projection.snapshot();
        let storage = SkillCatalogStorage::new(
            cfg.user_config_path.parent().unwrap(),
            runtime.owned.durable_state_store.clone(),
        )
        .unwrap();
        Self {
            runtime,
            run,
            session_id: session.id().into(),
            agent_id: agent.id().into(),
            files,
            storage,
        }
    }
    fn response(&self, request: &str, extension: &str) -> PathBuf {
        self.files
            .path()
            .join(format!("hook-context-responses/{request}.{extension}"))
    }
    async fn send(&self, event: serde_json::Value) {
        fs::write(self.files.path().join("events.jsonl"), event.to_string()).unwrap();
        self.runtime
            .with_app_side_effect(|app| {
                ProviderOutputClaudeNativeBridge::new(app)
                    .process(&self.session_id, self.run.id(), &self.run, None)
                    .unwrap();
            })
            .await;
    }
    async fn active_prompt_id(&self) -> String {
        self.runtime
            .with_app_side_effect(|app| {
                app.prompt_owner_active_prompt_for_agent(&self.session_id, &self.agent_id)
                    .unwrap()
                    .unwrap()
                    .id()
                    .into()
            })
            .await
    }
    async fn close(&self) {
        self.runtime
            .with_app_side_effect(|app| app.pty_mut().remove_process(self.run.id()).unwrap())
            .await;
    }
}

#[tokio::test]
async fn typed_native_only_claude_skill_runs_the_prepared_command_before_loaded() {
    let composer = Composer::new().await;
    let native = composer.files.path().join("claude/skills/review");
    fs::create_dir_all(&native).unwrap();
    fs::write(native.join("SKILL.md"), "---\nname: review\ndescription: Review source\ndisable-model-invocation: true\n---\nInspect the source.\n").unwrap();
    composer.send(serde_json::json!({"hook_event_name":"UserPromptSubmit", "prompt":"$review inspect source", "hook_context_request_id":"typed-original"})).await;
    let prompt_id = composer.active_prompt_id().await;
    let redirected = composer.response("typed-original", "error").exists();
    composer.send(serde_json::json!({"hook_event_name":"CharioxContextDelivered", "hook_context_request_id":"typed-original"})).await;
    let premature_loads = composer.storage.loads(&composer.session_id).unwrap();
    if redirected {
        composer.send(serde_json::json!({"hook_event_name":"CharioxNativeSkillRedirected", "hook_context_request_id":"typed-original"})).await;
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        composer
            .send(serde_json::json!({"hook_event_name":"Other"}))
            .await;
        let retry: serde_json::Value = serde_json::from_slice(
            &fs::read(composer.files.path().join("headless-submit-retry.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(retry["visible_prompt"], "/review inspect source");
        assert_eq!(
            composer.active_prompt_id().await,
            prompt_id,
            "a redirect manufactured another turn"
        );
        composer.send(serde_json::json!({"hook_event_name":"UserPromptSubmit", "prompt":"/review inspect source", "hook_context_request_id":"typed-replacement"})).await;
        assert!(composer.response("typed-replacement", "txt").exists());
        assert!(composer
            .storage
            .loads(&composer.session_id)
            .unwrap()
            .is_empty());
        composer.send(serde_json::json!({"hook_event_name":"CharioxContextDelivered", "hook_context_request_id":"typed-replacement"})).await;
        let loads = composer.storage.loads(&composer.session_id).unwrap();
        assert_eq!(loads.len(), 1);
        assert_eq!(loads[0].reason, SkillLoadReason::ConfirmedNative);
        assert_eq!(loads[0].prompt_id, prompt_id);
    }
    composer.close().await;
    assert!(
        redirected,
        "the original dollar text must be blocked before its native replacement"
    );
    assert!(
        premature_loads.is_empty(),
        "the original hook never delivered /review: {premature_loads:?}"
    );
}

#[tokio::test]
async fn typed_installed_claude_skill_with_a_missing_package_fails_the_hook() {
    let composer = Composer::new().await;
    let service = SkillCatalogService::new(composer.storage.clone());
    let mutation = SkillCatalogMutation { scope: SkillScope::Chariox, expected_revision: 0, actions: vec![SkillCatalogAction::Install {
        source: SkillInstallSource::Content { skill_md: "---\nname: review\ndescription: Review source\ndisable-model-invocation: true\n---\nInspect the source.\n".into() }, collision: SkillCollisionAction::Reject, package_id: None,
    }] };
    let prepared = service.prepare(&mutation).unwrap();
    let catalog = service.publish_authorized(&mutation, &prepared).unwrap();
    let skill = &catalog.catalogs[0].skills[0];
    let package = service.package_root(skill);
    fs::remove_dir_all(&package).unwrap();
    composer.send(serde_json::json!({"hook_event_name":"UserPromptSubmit", "prompt":"$review inspect source", "hook_context_request_id":"typed-missing"})).await;
    let failed = composer.response("typed-missing", "error").exists();
    let succeeded = composer.response("typed-missing", "txt").exists();
    let loads = composer.storage.loads(&composer.session_id).unwrap();
    composer.close().await;
    assert!(
        failed && !succeeded,
        "a resolved selection silently succeeded without its body"
    );
    assert!(loads.is_empty());
}
