//! Production worker admission and cold recovery; isolated synthetic Claude only.
use super::*;
use crate::account_profile::*;
use crate::transport::relay_peer::*;

struct WorkerFixture {
    root: crate::test_support::TestWorktree,
    app: Arc<Mutex<DaemonApp>>,
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
if [[ "$1" == "auth" && "$2" == "login" ]]; then
  : > "$CLAUDE_CONFIG_DIR/UNEXPECTED_LOGIN"
  exit 1
fi
: > "$CLAUDE_CONFIG_DIR/synthetic-process-started"
while IFS= read -r line; do
  printf '%s\n' "$line" >> "$CLAUDE_CONFIG_DIR/synthetic-prompts"
  printf '%s\n' '{"type":"result","subtype":"success","is_error":false,"result":"synthetic worker completed"}'
done
"#).unwrap();
        std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o700)).unwrap();
        std::env::set_var("CHARIOX_CLAUDE_BIN", executable);
        std::env::remove_var("CLAUDE_CODE_OAUTH_TOKEN");
        let mut config = crate::DaemonConfig::for_tests();
        config.accept_remote_leases = true;
        config.provider_runtime_init_delay_ms = 0;
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
        let runtime = owned_runtime_state(&app).await;
        assert!(!crate::provider::provider_account_credential_uses_vault(
            "owner", "claude", &account
        )
        .unwrap());
        Self {
            root,
            app,
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
        tokio::time::timeout(Duration::from_secs(10), async {
            while !std::fs::read_to_string(&marker).is_ok_and(|text| text.contains(prompt)) {
                self.runtime.pump_transport_runtime().await;
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        }).await.expect("the production worker must deliver the admitted prompt to the synthetic native harness");
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
