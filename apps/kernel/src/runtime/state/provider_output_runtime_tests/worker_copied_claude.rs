//! Production worker admission and cold recovery; isolated synthetic Claude only.
use super::*;
use crate::account_profile::*;
use crate::transport::relay_peer::*;

struct WorkerFixture {
    root: crate::test_support::TestWorktree,
    app: Arc<Mutex<DaemonApp>>,
    router: crate::runtime::router::CommandRouter,
    runtime: KernelRuntimeState,
    account: String,
    credential: std::path::PathBuf,
}

impl WorkerFixture {
    async fn new(copied: bool) -> Self {
        use std::os::unix::fs::PermissionsExt;
        let root = crate::test_support::TestWorktree::new("worker-copied-claude");
        let executable = root.path().join("claude");
        std::fs::write(&executable, r#"#!/bin/bash
if [[ "$1" == "--version" ]]; then printf 'Claude Code 2.1.207\n'; exit 0; fi
if [[ "$1" == "auth" && "$2" == "status" ]]; then
  printf '%s\n' '{"loggedIn":true,"authMethod":"claude.ai","subscriptionType":"pro"}'
  exit 0
fi
if [[ "$1" == "-p" && "$2" == "/usage" ]]; then
  : > "$CLAUDE_CONFIG_DIR/synthetic-usage-probed"
  printf '%s\n' '{"type":"result","subtype":"success","is_error":false,"duration_api_ms":0,"num_turns":0,"total_cost_usd":0,"result":"Current session: 17% used\nCurrent week (all models): 41% used","usage":{"input_tokens":0,"cache_creation_input_tokens":0,"cache_read_input_tokens":0,"output_tokens":0}}'
  exit 0
fi
if [[ "$1" == "auth" && "$2" == "login" ]]; then
  if [[ -f "$CLAUDE_CONFIG_DIR/allow-synthetic-login" ]]; then
    umask 077
    printf '%s' '{"claudeAiOauth":{"refreshToken":"synthetic-refresh","accessToken":"synthetic-access","expiresAt":9999999999999}}' > "$CLAUDE_CONFIG_DIR/.credentials.json"
    : > "$CLAUDE_CONFIG_DIR/synthetic-login-completed"
    exit 0
  fi
  : > "$CLAUDE_CONFIG_DIR/UNEXPECTED_LOGIN"
  exit 1
fi
: > "$CLAUDE_CONFIG_DIR/synthetic-process-started"
while IFS= read -r line; do
  printf '%s\n' "$line" >> "$CLAUDE_CONFIG_DIR/synthetic-prompts"
  while [[ -f "$CLAUDE_CONFIG_DIR/hold-synthetic-result" ]]; do sleep 0.02; done
  printf '%s\n' '{"type":"result","subtype":"success","is_error":false,"result":"synthetic worker completed"}'
done
"#).unwrap();
        std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o700)).unwrap();
        std::env::set_var("CHARIOX_CLAUDE_BIN", executable);
        std::env::remove_var("CLAUDE_CODE_OAUTH_TOKEN");
        let mut config = crate::DaemonConfig::for_tests();
        config.accept_remote_leases = true;
        config.provider_runtime_init_delay_ms = 0;
        // Both the App cold path and owned launch path use this same fixture Vault.
        std::env::set_var("CHARIOX_ALLOW_VOLATILE_PROCESS_MEMORY_VAULT", "1");
        config.user_config.credential_vault.backend =
            crate::config::CredentialVaultBackend::ProcessMemory;
        let app = DaemonApp::bootstrap(config).unwrap();
        let registry = app.provider_account_profile_registry();
        let account = if copied {
            let source =
                ProviderAccountProfileRegistry::open(root.path().join("source/profiles.json"))
                    .unwrap()
                    .with_machine_identity("synthetic-home-machine", "synthetic-home-kernel");
            let profile = source
                .create_managed("owner", "claude", "Copied Claude")
                .unwrap();
            let environment = source
                .resolve_environment("owner", "claude", &profile.profile_id)
                .unwrap();
            // This test-created fixture is never an existing linked/shared credential.
            std::fs::write(std::path::Path::new(&environment["CLAUDE_CONFIG_DIR"]).join(".credentials.json"),
                br#"{"claudeAiOauth":{"refreshToken":"synthetic-refresh","accessToken":"synthetic-access","expiresAt":9999999999999}}"#).unwrap();
            let materialization = source
                .export_materialization("owner", "claude", &profile.profile_id)
                .unwrap();
            let receiving = registry
                .materialize_replica("owner", &materialization)
                .unwrap();
            registry
                .record_received_account_copy(
                    "owner",
                    &materialization,
                    &receiving.profile_id,
                    ProviderAccountMaterializationTargetKind::Worker,
                )
                .unwrap();
            receiving.profile_id
        } else {
            registry
                .create_managed("owner", "claude", "Setup-token fallback")
                .unwrap()
                .profile_id
        };
        crate::test_support::authenticate_provider_account(&registry, "owner", "claude", &account)
            .unwrap();
        let environment = registry
            .resolve_environment("owner", "claude", &account)
            .unwrap();
        let credential =
            std::path::Path::new(&environment["CLAUDE_CONFIG_DIR"]).join(".credentials.json");
        let app = Arc::new(Mutex::new(app));
        let lanes = app.lock().await.provider_run_operation_lanes();
        let router =
            crate::runtime::router::CommandRouter::with_interactive_capacity_and_provider_lanes(
                Arc::clone(&app),
                16,
                lanes,
            );
        let runtime = router.runtime_state();
        assert!(!crate::provider::provider_account_credential_uses_vault(
            "owner", "claude", &account
        )
        .unwrap());
        Self {
            root,
            app,
            router,
            runtime,
            account,
            credential,
        }
    }

    async fn lease(&self) -> crate::execution_lease::LeasedAgent {
        let lease = self
            .runtime
            .create_relay_execution_lease(
                "synthetic-home-kernel",
                "room",
                &format!("home-agent-{}", rand::random::<u64>()),
                false,
                "owner",
            )
            .await
            .unwrap();
        self.runtime
            .create_relay_leased_agent(
                &lease.id,
                "claude",
                &self.account,
                Some("sonnet".into()),
                None,
                None,
                None,
                None,
                Some(self.root.path().display().to_string()),
                None,
            )
            .await
            .unwrap()
    }

    async fn submit(
        &self,
        leased: &crate::execution_lease::LeasedAgent,
        id: &str,
        workflow: bool,
        token: bool,
    ) -> Result<(String, crate::session::PromptSubmissionOutcome), DaemonError> {
        self.runtime
            .submit_relay_leased_prompt(
                &leased.id,
                RelayAgentExecutionProfile {
                    provider: leased.provider.clone(),
                    account_profile: leased.account_profile.clone(),
                    model: leased.model.clone(),
                    effort: leased.effort.clone(),
                },
                id,
                "",
                Vec::new(),
                workflow.then(|| crate::execution_lease::RemoteWorkflowTurnContext {
                    home_kernel_id: "synthetic-home-kernel".into(),
                    home_session_id: "room".into(),
                    home_agent_id: leased.home_agent_id.clone(),
                    workflow_run_id: "workflow".into(),
                    workflow_node_run_id: id.into(),
                    delivery_token: id.into(),
                }),
                Some(RemoteGitTurnContext {
                    home_session_id: "room".into(),
                    home_agent_id: leased.home_agent_id.clone(),
                    home_prompt_id: id.into(),
                    home_turn_id: id.into(),
                    source_attachment_id: None,
                    workspace_live_sync_mode: None,
                    prompt_origin: Some(crate::session::PromptOrigin::Chariox),
                    external_provider: None,
                    external_provider_session_id: None,
                    external_provider_turn_id: None,
                    prompt_summary: id.into(),
                }),
                Vec::new(),
                None,
                crate::extension::RemoteExtensionManifest::default(),
                token.then(|| RemoteProviderLaunchCredential {
                    provider: "claude".into(),
                    account_profile: leased.account_profile.clone(),
                    secret_input: RemoteCredentialSecretInput::new("synthetic-setup-token".into()),
                }),
            )
            .await
    }

    async fn wait_for_prompt(&self, prompt: &str) {
        let marker = self.credential.parent().unwrap().join("synthetic-prompts");
        let result = tokio::time::timeout(Duration::from_secs(10), async {
            while !std::fs::read_to_string(&marker).is_ok_and(|text| text.contains(prompt)) {
                self.runtime.pump_transport_runtime().await;
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await;
        if result.is_err() {
            for run in self.runtime.owned.provider_store.list_runs() {
                let session = self
                    .runtime
                    .owned
                    .session_store
                    .get_session(run.session_id())
                    .unwrap();
                eprintln!(
                    "synthetic dispatch timeout: run={} state={:?} workflow={} interactions={:?}",
                    run.id(),
                    run.state(),
                    run.workflow_tools_enabled(),
                    session
                        .active_interactions()
                        .iter()
                        .map(|interaction| interaction.title())
                        .collect::<Vec<_>>()
                );
            }
            eprintln!(
                "synthetic login completed={} artifact exists={} harness started={}",
                marker
                    .parent()
                    .unwrap()
                    .join("synthetic-login-completed")
                    .exists(),
                self.credential.exists(),
                marker
                    .parent()
                    .unwrap()
                    .join("synthetic-process-started")
                    .exists()
            );
            self.app.lock().await.shutdown_cleanup().unwrap();
        }
        result.expect("the production worker must deliver the admitted prompt to the synthetic native harness");
    }
}

#[tokio::test]
async fn review_worker_copied_claude_admits_cold_ordinary_and_workflow_prompts() {
    if crate::test_support::isolate_environment_test() {
        return;
    }
    let fixture = WorkerFixture::new(true).await;
    for workflow in [false, true] {
        let leased = fixture.lease().await;
        let prompt = if workflow {
            "cold-workflow"
        } else {
            "cold-ordinary"
        };
        let (run, _) = fixture.submit(&leased, prompt, workflow, false).await
            .expect("a renewable receiving Claude login must pass production worker prompt admission without a setup token");
        fixture.wait_for_prompt(prompt).await;
        assert_eq!(
            fixture
                .runtime
                .owned
                .provider_store
                .get_run(&run)
                .unwrap()
                .account_profile(),
            fixture.account
        );
        let receipt = fixture
            .runtime
            .query_relay_leased_prompt_receipt(&leased.id, prompt)
            .await
            .unwrap()
            .unwrap();
        assert_ne!(receipt.phase, LeasedPromptReceiptPhase::SteerRejected);
    }
    fixture.app.lock().await.shutdown_cleanup().unwrap();
}

#[tokio::test]
async fn review_worker_cold_missing_claude_copy_keeps_work_and_requests_human_login() {
    if crate::test_support::isolate_environment_test() {
        return;
    }
    let fixture = WorkerFixture::new(true).await;
    let leased = fixture.lease().await;
    std::fs::remove_file(&fixture.credential).unwrap(); // Only the synthetic receiving fixture.
    let prepared = crate::app::RemoteLeaseRuntime::new(&mut *fixture.app.lock().await)
        .prepare_leased_prompt_submission(
            &leased.id,
            "retained-cold",
            "",
            Vec::new(),
            None,
            None,
            Vec::new(),
            None,
            crate::extension::RemoteExtensionManifest::default(),
        )
        .unwrap();
    let crate::app::PreparedLeasedProviderRun::LaunchRequired(request) = prepared.provider_run
    else {
        panic!("must be a cold launch")
    };
    // Exercise the production launch service directly so the admission bug cannot mask
    // a credential-preparation failure; no preinserted provider run or Vault credential.
    let run = fixture.runtime.launch_provider_for_remote_lease_detached(request, None).await
        .expect("missing copied Claude artifact must enter receiving login recovery before credential preparation rejects it");
    for (id, workflow) in [("retained-cold", false), ("queued-workflow", true)] {
        let (accepted_run, outcome) = fixture.submit(&leased, id, workflow, false).await.unwrap();
        assert_eq!(accepted_run, run.id());
        assert!(matches!(
            outcome,
            crate::session::PromptSubmissionOutcome::Queued { .. }
        ));
        let receipt = fixture
            .runtime
            .query_relay_leased_prompt_receipt(&leased.id, id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(receipt.phase, LeasedPromptReceiptPhase::Active);
    }
    let session = fixture
        .runtime
        .owned
        .session_store
        .get_session(&leased.backing_session_id)
        .unwrap();
    let interaction = session
        .active_interaction_for_agent(&leased.backing_agent_id)
        .expect("receiving human login interaction");
    assert_eq!(
        interaction.title(),
        Some("Log in to Claude on this machine")
    );
    assert!(interaction
        .choices()
        .iter()
        .any(|choice| choice.id() == "login"));
    assert_eq!(
        fixture
            .runtime
            .owned
            .prompt_state_owner
            .peek_next_queued_prompt(&session, &leased.backing_agent_id)
            .unwrap()
            .prompt(),
        "retained-cold"
    );
    assert_eq!(
        fixture
            .runtime
            .owned
            .provider_store
            .get_run(run.id())
            .unwrap()
            .state(),
        crate::provider::ProviderRunState::Starting
    );
    assert!(!fixture
        .credential
        .parent()
        .unwrap()
        .join("synthetic-process-started")
        .exists());
    assert!(!fixture
        .credential
        .parent()
        .unwrap()
        .join("UNEXPECTED_LOGIN")
        .exists());
    let profile = fixture
        .runtime
        .owned
        .provider_account_profiles
        .get("owner", "claude", &fixture.account)
        .unwrap();
    assert!(profile
        .materializations
        .iter()
        .filter_map(|status| status.copy.as_ref())
        .any(|copy| copy.auth_state == ProviderAccountCopyAuthState::NeedsLogin));
    fixture.app.lock().await.shutdown_cleanup().unwrap();
}

#[tokio::test]
async fn review_worker_claude_without_login_retains_typed_setup_token_fallback() {
    if crate::test_support::isolate_environment_test() {
        return;
    }
    let fixture = WorkerFixture::new(false).await;
    let leased = fixture.lease().await;
    let error = fixture
        .submit(&leased, "missing-token", false, false)
        .await
        .unwrap_err();
    assert!(error
        .to_string()
        .contains(REMOTE_PROVIDER_LAUNCH_CREDENTIAL_REQUIRED_CODE));
    fixture
        .submit(&leased, "with-token", false, true)
        .await
        .unwrap();
    fixture.wait_for_prompt("with-token").await;
    fixture.app.lock().await.shutdown_cleanup().unwrap();
}

#[tokio::test]
async fn review_worker_missing_claude_copy_admits_first_prompt_before_human_login() {
    if crate::test_support::isolate_environment_test() {
        return;
    }
    let fixture = WorkerFixture::new(true).await;
    let leased = fixture.lease().await;
    std::fs::remove_file(&fixture.credential).unwrap();
    let (run, outcome) = fixture
        .submit(&leased, "cold-missing-workflow", true, false)
        .await
        .expect("the production worker must admit the first turn while receiving login is pending");
    assert!(matches!(
        outcome,
        crate::session::PromptSubmissionOutcome::Queued { .. }
    ));
    let session = fixture
        .runtime
        .owned
        .session_store
        .get_session(&leased.backing_session_id)
        .unwrap();
    let interaction = session
        .active_interaction_for_agent(&leased.backing_agent_id)
        .unwrap();
    assert!(interaction.provider_login_is_human_only());
    assert_eq!(
        interaction.title(),
        Some("Log in to Claude on this machine")
    );
    assert_eq!(
        fixture
            .runtime
            .owned
            .prompt_state_owner
            .peek_next_queued_prompt(&session, &leased.backing_agent_id)
            .unwrap()
            .prompt(),
        "cold-missing-workflow"
    );
    assert_eq!(
        fixture
            .runtime
            .owned
            .provider_store
            .get_run(&run)
            .unwrap()
            .state(),
        crate::provider::ProviderRunState::Starting
    );
    assert!(!fixture
        .credential
        .parent()
        .unwrap()
        .join("synthetic-process-started")
        .exists());
    assert!(!fixture
        .credential
        .parent()
        .unwrap()
        .join("UNEXPECTED_LOGIN")
        .exists());
    fixture.app.lock().await.shutdown_cleanup().unwrap();
}

// MP-08/MP-10/MP-11: a receiving copy can be missing while token auth is usable.
#[tokio::test]
async fn review_worker_missing_claude_copy_with_supplied_token_delivers_prompts() {
    if crate::test_support::isolate_environment_test() {
        return;
    }
    let fixture = WorkerFixture::new(true).await;
    let leased = fixture.lease().await;
    std::fs::remove_file(&fixture.credential).unwrap();
    let (run_id, _) = fixture
        .submit(&leased, "supplied-token-first", false, true)
        .await
        .unwrap();
    fixture.wait_for_prompt("supplied-token-first").await;
    fixture
        .submit(&leased, "supplied-token-second", true, false)
        .await
        .unwrap();
    fixture.wait_for_prompt("supplied-token-second").await;
    assert_eq!(
        fixture
            .runtime
            .owned
            .provider_store
            .get_run(&run_id)
            .unwrap()
            .state(),
        crate::provider::ProviderRunState::Running
    );
    assert!(fixture
        .runtime
        .owned
        .session_store
        .get_session(&leased.backing_session_id)
        .unwrap()
        .active_interactions()
        .is_empty());
    fixture.app.lock().await.shutdown_cleanup().unwrap();
}

impl WorkerFixture {
    async fn local_request(
        &self,
        request: crate::local::LocalDaemonRequest,
    ) -> Result<crate::local::LocalDaemonResponse, DaemonError> {
        let mut caller = crate::runtime::command::KernelCaller::default()
            .with_connection_class(crate::local::KernelConnectionClass::Terminal);
        caller.user_id = Some("owner".into());
        let command = crate::runtime::command::KernelCommand::from_local_request_with_caller(
            format!("receiving-local-{}", rand::random::<u64>()),
            crate::runtime::command::KernelCommandSource::LocalCli,
            caller,
            None,
            None,
            &request,
        );
        self.router.dispatch(command, request).await
    }

    async fn local_launch(
        &self,
        leased: &crate::execution_lease::LeasedAgent,
    ) -> crate::provider::PublicProviderRun {
        let response = self.local_request(crate::local::LocalDaemonRequest::LaunchProviderRun(crate::local::LaunchProviderRunRequest {
            session_id: leased.backing_session_id.clone(), agent_id: Some(leased.backing_agent_id.clone()),
            adapter_key: "claude".into(), provider: "claude".into(), account_profile: self.account.clone(),
            model: "sonnet".into(), variant: None, structured_endpoint: None, provider_session_id: None, native_tui: false,
        })).await.expect("public receiving-kernel launch must recover or use its registered token before preparation");
        let (crate::local::LocalDaemonResponse::ProviderRunLaunched { provider_run }
        | crate::local::LocalDaemonResponse::ProviderRunLaunchAccepted { provider_run }) = response
        else {
            panic!("launch response");
        };
        provider_run
    }
}

#[tokio::test]
async fn review_local_missing_claude_copy_requests_login_before_preparation() {
    if crate::test_support::isolate_environment_test() {
        return;
    }
    let fixture = WorkerFixture::new(true).await;
    let leased = fixture.lease().await;
    std::fs::remove_file(&fixture.credential).unwrap();
    let run = fixture.local_launch(&leased).await;
    assert_eq!(run.state(), crate::provider::ProviderRunState::Starting);
    assert!(!fixture
        .runtime
        .owned
        .provider_run_projection
        .is_leased_provider_run(run.id()));
    let session = fixture
        .runtime
        .owned
        .session_store
        .get_session(&leased.backing_session_id)
        .unwrap();
    let interaction = session
        .active_interaction_for_agent(&leased.backing_agent_id)
        .expect("human recovery interaction");
    assert_eq!(
        interaction.title(),
        Some("Log in to Claude on this machine")
    );
    assert!(interaction.provider_login_is_human_only());
    assert!(!fixture
        .credential
        .parent()
        .unwrap()
        .join("synthetic-process-started")
        .exists());
    assert!(!fixture
        .credential
        .parent()
        .unwrap()
        .join("UNEXPECTED_LOGIN")
        .exists());
    fixture.app.lock().await.shutdown_cleanup().unwrap();
}

#[tokio::test]
async fn review_local_missing_claude_copy_with_registered_token_delivers_prompt() {
    if crate::test_support::isolate_environment_test() {
        return;
    }
    let fixture = WorkerFixture::new(true).await;
    let leased = fixture.lease().await;
    std::fs::remove_file(&fixture.credential).unwrap();
    fixture.register_token();
    let run = fixture.local_launch(&leased).await;
    tokio::time::timeout(Duration::from_secs(10), async {
        while fixture
            .runtime
            .owned
            .provider_store
            .get_run(run.id())
            .unwrap()
            .state()
            == crate::provider::ProviderRunState::Starting
        {
            fixture.runtime.pump_transport_runtime().await;
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("token-authenticated launch must complete");
    assert_eq!(
        fixture
            .runtime
            .owned
            .provider_store
            .get_run(run.id())
            .unwrap()
            .state(),
        crate::provider::ProviderRunState::Running
    );
    fixture
        .submit(&leased, "registered-token-prompt", false, false)
        .await
        .unwrap();
    fixture.wait_for_prompt("registered-token-prompt").await;
    assert!(fixture
        .runtime
        .owned
        .session_store
        .get_session(&leased.backing_session_id)
        .unwrap()
        .active_interactions()
        .is_empty());
    fixture.app.lock().await.shutdown_cleanup().unwrap();
}

// MP-08/MP-10/MP-11: an ordinary local agent's first prompt launches through the
// app-level cold path, not `LaunchProviderRun` or leased submission.
impl WorkerFixture {
    async fn local_agent(&self, native_tui: bool) -> (String, String, String) {
        let mut app = self.app.lock().await;
        let (session, _) = crate::app::KernelSessionService::new(&mut app)
            .create_session(self.root.session_request().with_owner_user_id("owner"))
            .unwrap();
        let agent = crate::app::KernelSessionService::new(&mut app)
            .spawn_agent(
                crate::agent::CreateAgentRequest::new(session.id(), "claude")
                    .with_owner_user_id("owner")
                    .with_model("sonnet")
                    .with_account_profile(self.account.clone()),
            )
            .unwrap();
        let attachment = crate::app::KernelSessionService::new(&mut app)
            .attach(crate::attachment::AttachRequest::for_user(
                session.id(),
                "client",
                crate::attachment::ClientCapabilityLevel::FullTerminal,
                "owner",
            ))
            .unwrap();
        drop(app);
        if native_tui {
            let launch = self
                .local_request(crate::local::LocalDaemonRequest::LaunchProviderRun(
                    crate::local::LaunchProviderRunRequest {
                        session_id: session.id().into(),
                        agent_id: Some(agent.id().into()),
                        adapter_key: "claude".into(),
                        provider: "claude".into(),
                        account_profile: self.account.clone(),
                        model: "sonnet".into(),
                        variant: None,
                        structured_endpoint: None,
                        provider_session_id: None,
                        native_tui: true,
                    },
                ))
                .await;
            assert!(launch.is_ok(), "native TUI launch: {launch:?}");
            // Delete the copy only after the first native launch has finished;
            // otherwise its completion recovery races the reattach under test.
            tokio::time::timeout(Duration::from_secs(10), async {
                loop {
                    if self
                        .runtime
                        .owned
                        .provider_store
                        .get_run_for_agent(session.id(), agent.id())
                        .is_some_and(|run| {
                            run.state() == crate::provider::ProviderRunState::Running
                        })
                    {
                        break;
                    }
                    self.runtime.pump_transport_runtime().await;
                    tokio::time::sleep(Duration::from_millis(20)).await;
                }
            })
            .await
            .expect("first native launch reaches Running");
        }
        (
            session.id().to_string(),
            agent.id().to_string(),
            attachment.id().to_string(),
        )
    }

    async fn local_prompt(
        &self,
        (session_id, agent_id, attachment_id): &(String, String, String),
        prompt: &str,
    ) -> Result<crate::session::PromptSubmissionOutcome, DaemonError> {
        let response = self
            .local_request(crate::local::LocalDaemonRequest::SubmitPrompt(
                crate::local::SubmitPromptRequest {
                    session_id: session_id.clone(),
                    attachment_id: attachment_id.clone(),
                    target_agent_id: Some(agent_id.clone()),
                    prompt: prompt.into(),
                    attachments: Vec::new(),
                },
            ))
            .await?;
        let crate::local::LocalDaemonResponse::PromptSubmitted { outcome, .. } = response else {
            panic!("prompt response");
        };
        Ok(outcome)
    }

    fn register_token(&self) {
        let config = self.runtime.owned.config_projection.snapshot();
        crate::provider::store_provider_account_credential(
            &config,
            "owner",
            "claude",
            &self.account,
            "synthetic-registered-token",
            false,
        )
        .unwrap();
    }
}

#[tokio::test]
async fn review_local_cold_prompt_missing_claude_copy_requests_login_before_preparation() {
    if crate::test_support::isolate_environment_test() {
        return;
    }
    let fixture = WorkerFixture::new(true).await;
    let local = fixture.local_agent(false).await;
    std::fs::remove_file(&fixture.credential).unwrap();
    let outcome = fixture.local_prompt(&local, "cold-local-missing").await
        .expect("an ordinary cold prompt must enter receiving login recovery before credential preparation rejects it");
    assert!(matches!(
        outcome,
        crate::session::PromptSubmissionOutcome::Queued { .. }
    ));
    let session = fixture
        .runtime
        .owned
        .session_store
        .get_session(&local.0)
        .unwrap();
    let interaction = session
        .active_interaction_for_agent(&local.1)
        .expect("human recovery interaction");
    assert_eq!(
        interaction.title(),
        Some("Log in to Claude on this machine")
    );
    assert_eq!(
        fixture
            .runtime
            .owned
            .prompt_state_owner
            .peek_next_queued_prompt(&session, &local.1)
            .unwrap()
            .prompt(),
        "cold-local-missing"
    );
    let run = fixture
        .runtime
        .owned
        .provider_store
        .get_run_for_agent(&local.0, &local.1)
        .unwrap();
    assert_eq!(run.state(), crate::provider::ProviderRunState::Starting);
    for marker in ["synthetic-process-started", "UNEXPECTED_LOGIN"] {
        assert!(!fixture.credential.parent().unwrap().join(marker).exists());
    }
    fixture.app.lock().await.shutdown_cleanup().unwrap();
}

#[tokio::test]
async fn review_local_cold_prompt_missing_claude_copy_with_registered_token_delivers() {
    if crate::test_support::isolate_environment_test() {
        return;
    }
    let fixture = WorkerFixture::new(true).await;
    let local = fixture.local_agent(false).await;
    std::fs::remove_file(&fixture.credential).unwrap();
    fixture.register_token();
    fixture
        .local_prompt(&local, "cold-local-registered-token")
        .await
        .unwrap();
    fixture.wait_for_prompt("cold-local-registered-token").await;
    assert!(fixture
        .runtime
        .owned
        .session_store
        .get_session(&local.0)
        .unwrap()
        .active_interactions()
        .is_empty());
    fixture.app.lock().await.shutdown_cleanup().unwrap();
}

#[tokio::test]
async fn review_local_native_tui_reattach_reuses_live_run_before_copy_recovery() {
    if crate::test_support::isolate_environment_test() {
        return;
    }
    let fixture = WorkerFixture::new(true).await;
    let local = fixture.local_agent(true).await;
    let live = fixture
        .runtime
        .owned
        .provider_store
        .get_run_for_agent(&local.0, &local.1)
        .unwrap();
    std::fs::remove_file(&fixture.credential).unwrap();
    let request = crate::local::LaunchProviderRunRequest {
        session_id: local.0.clone(),
        agent_id: Some(local.1.clone()),
        adapter_key: "claude".into(),
        provider: "claude".into(),
        account_profile: fixture.account.clone(),
        model: "sonnet".into(),
        variant: None,
        structured_endpoint: None,
        provider_session_id: None,
        native_tui: true,
    };
    for _ in 0..3 {
        let response = fixture
            .local_request(crate::local::LocalDaemonRequest::LaunchProviderRun(
                request.clone(),
            ))
            .await
            .expect("native TUI reattach");
        let (crate::local::LocalDaemonResponse::ProviderRunLaunched { provider_run }
        | crate::local::LocalDaemonResponse::ProviderRunLaunchAccepted { provider_run }) = response
        else {
            panic!("launch response");
        };
        assert_eq!(provider_run.id(), live.id());
    }
    let mut mismatched = request;
    mismatched.model = "opus".into();
    let error = fixture
        .local_request(crate::local::LocalDaemonRequest::LaunchProviderRun(
            mismatched,
        ))
        .await
        .expect_err("native TUI reattach must still reject different parameters");
    assert!(matches!(
        error,
        DaemonError::InvalidProviderRunState {
            operation: "launch native TUI provider run with different parameters",
            ..
        }
    ));
    assert_eq!(
        fixture
            .runtime
            .owned
            .provider_store
            .get_run(live.id())
            .unwrap()
            .state(),
        crate::provider::ProviderRunState::Running
    );
    assert_eq!(
        fixture
            .runtime
            .owned
            .session_store
            .get_session(&local.0)
            .unwrap()
            .active_provider_run_id(),
        Some(live.id())
    );
    let runs = fixture.runtime.owned.provider_store.list_runs();
    assert_eq!(
        runs.iter()
            .filter(|run| run.agent_instance_id() == Some(local.1.as_str()))
            .count(),
        1
    );
    assert!(fixture
        .runtime
        .owned
        .session_store
        .get_session(&local.0)
        .unwrap()
        .active_interactions()
        .is_empty());
    fixture.app.lock().await.shutdown_cleanup().unwrap();
}

// MP-08/MP-10/MP-11: environment replacement uses this synchronous App launch
// rather than the detached cold path or the runtime launch-completion service.
#[tokio::test]
async fn review_app_missing_claude_copy_token_launch_retains_auth_mode() {
    if crate::test_support::isolate_environment_test() {
        return;
    }
    let fixture = WorkerFixture::new(true).await;
    let local = fixture.local_agent(false).await;
    std::fs::remove_file(&fixture.credential).unwrap();
    fixture.register_token();
    let run = fixture
        .app
        .lock()
        .await
        .launch_provider(
            crate::provider::LaunchProviderRequest::new(
                &local.0,
                "claude",
                "claude",
                &fixture.account,
                "sonnet",
            )
            .with_agent_id(&local.1),
        )
        .unwrap();
    assert!(fixture
        .runtime
        .owned
        .provider_store
        .claude_run_uses_setup_token(run.id()));
    fixture
        .local_prompt(&local, "app-token-auth-mode")
        .await
        .unwrap();
    fixture.wait_for_prompt("app-token-auth-mode").await;
    assert!(fixture
        .runtime
        .owned
        .session_store
        .get_session(&local.0)
        .unwrap()
        .active_interactions()
        .is_empty());
    fixture.app.lock().await.shutdown_cleanup().unwrap();
}

// MP-08/MP-10/MP-11: local workflow admission shares cold-copy recovery with
// ordinary prompts, retaining the prepared node and its tool/context binding.
impl WorkerFixture {
    async fn local_workflow(&self) -> ((String, String, String), String, String) {
        let local = self.local_agent(false).await;
        let app = self.app.lock().await;
        let workflow = app
            .sessions_mut()
            .create_workflow(&local.0, Some("copy-recovery".into()))
            .unwrap();
        let node = app
            .sessions_mut()
            .add_workflow_node(&local.0, workflow.id(), &local.1)
            .unwrap();
        app.sessions_mut()
            .set_workflow_node_can_complete_run(&local.0, workflow.id(), node.id(), true)
            .unwrap();
        let endpoint = app
            .sessions_mut()
            .create_workflow_endpoint(&local.0, workflow.id(), node.id(), Some("entry".into()))
            .unwrap();
        app.sessions_mut()
            .set_workflow_endpoint_owner(&local.0, workflow.id(), endpoint.id(), "owner".into())
            .unwrap();
        (local, workflow.id().into(), endpoint.id().into())
    }

    async fn invoke_local_workflow(
        &self,
        local: &(String, String, String),
        workflow: &str,
        endpoint: &str,
        prompt: &str,
    ) -> crate::session::WorkflowRun {
        let response = self
            .local_request(crate::local::LocalDaemonRequest::InvokeWorkflowEndpoint(
                crate::local::InvokeWorkflowEndpointRequest {
                    session_id: local.0.clone(),
                    workflow_ref: workflow.into(),
                    endpoint_ref: endpoint.into(),
                    queue_ref: None,
                    prompt: Some(prompt.into()),
                    publication_invocation: None,
                },
            ))
            .await
            .expect("a cold local workflow must admit its node before receiving login recovery");
        let crate::local::LocalDaemonResponse::WorkflowRunInvoked { workflow_run, .. } = response
        else {
            panic!("workflow invocation response");
        };
        workflow_run
    }
}

#[tokio::test]
async fn review_local_workflow_missing_claude_copy_retains_node_and_requests_login() {
    if crate::test_support::isolate_environment_test() {
        return;
    }
    let fixture = WorkerFixture::new(true).await;
    let (local, workflow, endpoint) = fixture.local_workflow().await;
    std::fs::remove_file(&fixture.credential).unwrap();
    let workflow_run = fixture
        .invoke_local_workflow(&local, &workflow, &endpoint, "retained-local-workflow")
        .await;
    let session = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let session = fixture
                .runtime
                .owned
                .session_store
                .get_session(&local.0)
                .unwrap();
            if session.active_interaction_for_agent(&local.1).is_some() {
                break session;
            }
            fixture.runtime.pump_transport_runtime().await;
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("detached local workflow must request receiving login before resolving credentials");
    let interaction = session.active_interaction_for_agent(&local.1).unwrap();
    assert_eq!(
        interaction.title(),
        Some("Log in to Claude on this machine")
    );
    assert!(interaction.provider_login_is_human_only());
    let run = fixture
        .runtime
        .owned
        .provider_store
        .get_run_for_agent(&local.0, &local.1)
        .unwrap();
    assert_eq!(run.state(), crate::provider::ProviderRunState::Starting);
    assert!(run.workflow_tools_enabled());
    assert!(run.runtime_mcp_server_url().is_some());
    let queued = fixture
        .runtime
        .owned
        .prompt_state_owner
        .peek_next_queued_prompt(&session, &local.1)
        .unwrap();
    assert_eq!(queued.prompt(), "retained-local-workflow");
    assert_eq!(queued.workflow_run_id(), Some(workflow_run.id()));
    assert_eq!(
        queued.workflow_node_run_id(),
        Some(workflow_run.node_runs()[0].id())
    );
    assert!(!queued.hidden_system_context().is_empty());
    assert_eq!(
        run.workflow_fresh_context_node_run_id(),
        queued.workflow_node_run_id()
    );
    assert!(fixture
        .runtime
        .owned
        .prompt_state_owner
        .active_prompt_for_agent(&session, &local.1)
        .is_none());
    for marker in ["synthetic-process-started", "UNEXPECTED_LOGIN"] {
        assert!(!fixture.credential.parent().unwrap().join(marker).exists());
    }
    // MP-08/MP-10/MP-11: drive receiving login through the actual recovery
    // continuation, then exercise the replacement's authenticated workflow tools.
    // These opt-in files affect only this isolated synthetic provider/profile.
    let profile_dir = fixture.credential.parent().unwrap();
    std::fs::write(profile_dir.join("allow-synthetic-login"), "").unwrap();
    std::fs::write(profile_dir.join("hold-synthetic-result"), "").unwrap();
    fixture
        .runtime
        .resolve_runtime_interaction(&local.0, interaction.id(), "login", None)
        .await
        .unwrap();
    fixture.wait_for_prompt("retained-local-workflow").await;
    assert!(profile_dir.join("synthetic-login-completed").exists());
    let replacement = fixture
        .runtime
        .owned
        .provider_store
        .get_run_for_agent(&local.0, &local.1)
        .unwrap();
    assert_ne!(replacement.id(), run.id());
    let tool_names = fixture
        .runtime
        .runtime_tool_specs_for_auth_token_async(
            replacement.runtime_mcp_auth_token().unwrap().to_string(),
        )
        .await
        .unwrap()
        .into_iter()
        .map(|spec| spec.name)
        .collect::<std::collections::BTreeSet<_>>();
    // Settle fixture processes on RED too, before the failing identity assertion.
    if !replacement.workflow_tools_enabled() {
        fixture.app.lock().await.shutdown_cleanup().unwrap();
    }
    assert!(
        replacement.workflow_tools_enabled(),
        "login relaunch lost workflow tools"
    );
    assert_eq!(
        replacement.workflow_fresh_context_node_run_id(),
        queued.workflow_node_run_id()
    );
    use crate::transport::runtime_tools::{
        ACK_WORKFLOW_TURN_TOOL, VALIDATE_AND_SUBMIT_WORKFLOW_RUN_OUTPUT_TOOL,
    };
    for tool in [
        ACK_WORKFLOW_TURN_TOOL,
        VALIDATE_AND_SUBMIT_WORKFLOW_RUN_OUTPUT_TOOL,
    ] {
        assert!(
            tool_names.contains(tool),
            "replacement MCP discovery omitted {tool}"
        );
    }
    let session = fixture
        .runtime
        .owned
        .session_store
        .get_session(&local.0)
        .unwrap();
    let active = fixture
        .runtime
        .owned
        .prompt_state_owner
        .active_prompt_for_agent(&session, &local.1)
        .unwrap();
    // Admission replaces the provisional queue id with a durable prompt id.
    assert_eq!(active.prompt(), queued.prompt());
    assert_eq!(active.workflow_run_id(), queued.workflow_run_id());
    assert_eq!(active.workflow_node_run_id(), queued.workflow_node_run_id());
    assert_eq!(
        active.hidden_system_context(),
        queued.hidden_system_context()
    );
    let delivery_token = session
        .workflow_runs()
        .iter()
        .find(|candidate| candidate.id() == workflow_run.id())
        .unwrap()
        .node_runs()[0]
        .turn_envelope()
        .unwrap()
        .delivery_token()
        .to_string();
    let auth = replacement.runtime_mcp_auth_token().unwrap();
    let acknowledged = fixture
        .runtime
        .dispatch_authenticated_runtime_tool_call(
            auth,
            ACK_WORKFLOW_TURN_TOOL,
            serde_json::json!({"delivery_token": delivery_token}),
        )
        .await
        .unwrap();
    assert!(acknowledged.ok);
    let submitted = fixture.runtime.dispatch_authenticated_runtime_tool_call(
        auth, VALIDATE_AND_SUBMIT_WORKFLOW_RUN_OUTPUT_TOOL,
        serde_json::json!({"workflow_output_json": "{\"answer\":\"recovered\"}", "delivery_token": delivery_token}),
    ).await.unwrap();
    assert!(submitted.ok);
    std::fs::remove_file(profile_dir.join("hold-synthetic-result")).unwrap();
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            fixture.runtime.pump_transport_runtime().await;
            // MP-08/MP-10/MP-11: completed runs move into workflow history.
            let response = fixture
                .local_request(crate::local::LocalDaemonRequest::GetWorkflowRun(
                    crate::local::GetWorkflowRunRequest {
                        session_id: local.0.clone(),
                        workflow_run_ref: workflow_run.id().to_string(),
                    },
                ))
                .await
                .unwrap();
            let crate::local::LocalDaemonResponse::WorkflowRun {
                workflow_run: completed,
            } = response
            else {
                panic!("workflow history response");
            };
            if completed.status() == crate::session::WorkflowRunStatus::Completed {
                assert_eq!(completed.final_output_valid(), Some(true));
                assert_eq!(
                    completed.final_output().unwrap().message(),
                    "{\"answer\":\"recovered\"}"
                );
                assert_eq!(
                    completed.node_runs()[0].status(),
                    crate::session::WorkflowNodeRunStatus::Completed
                );
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("retained node must complete after receiving login and output submission");
    fixture.app.lock().await.shutdown_cleanup().unwrap();
}

#[tokio::test]
async fn review_local_workflow_missing_claude_copy_with_registered_token_delivers() {
    if crate::test_support::isolate_environment_test() {
        return;
    }
    let fixture = WorkerFixture::new(true).await;
    let (local, workflow, endpoint) = fixture.local_workflow().await;
    std::fs::remove_file(&fixture.credential).unwrap();
    fixture.register_token();
    fixture
        .invoke_local_workflow(&local, &workflow, &endpoint, "token-local-workflow")
        .await;
    fixture.wait_for_prompt("token-local-workflow").await;
    let run = fixture
        .runtime
        .owned
        .provider_store
        .get_run_for_agent(&local.0, &local.1)
        .unwrap();
    assert_eq!(run.state(), crate::provider::ProviderRunState::Running);
    assert!(run.workflow_tools_enabled());
    assert!(fixture
        .runtime
        .owned
        .session_store
        .get_session(&local.0)
        .unwrap()
        .active_interactions()
        .is_empty());
    fixture.app.lock().await.shutdown_cleanup().unwrap();
}
