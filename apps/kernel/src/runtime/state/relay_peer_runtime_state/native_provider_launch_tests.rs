//! MP-08 / MP-10 / MP-11: worker launch/admission regressions, separate from runtime wiring.
use std::sync::Arc;

use tokio::sync::Mutex;

use super::*;

struct NativeLeaseFixture {
    state: KernelRuntimeState,
    leased_agent_id: String,
    matching_request: crate::provider::LaunchProviderRequest,
}

impl NativeLeaseFixture {
    fn insert_live_run(&self, provider_run_id: &str) {
        let mut run = crate::provider::RuntimeProviderRun::new(
            provider_run_id,
            &self.matching_request,
            crate::provider::ProviderLaunchResult {
                endpoint_mode: crate::provider::AgentEndpointMode::Managed,
                process_label: "relay-native-reuse-test".to_string(),
                pty_target: None,
                pty_program: None,
                pty_args: Vec::new(),
                pty_env: std::collections::BTreeMap::new(),
                pty_env_remove: Vec::new(),
                working_directory: self.matching_request.working_directory.clone(),
                structured_endpoint: None,
            },
        );
        assert!(self.matching_request.matches_existing_run_selection(&run));
        run.mark_running();
        self.state
            .owned
            .provider_store
            .write()
            .insert_run_for_test(run);
    }

    async fn launch(
        &self,
        model: &str,
    ) -> Result<crate::provider::RuntimeProviderRun, DaemonError> {
        self.state
            .launch_relay_leased_native_provider_run(
                &self.leased_agent_id,
                "claude",
                "claude",
                "work",
                model,
                None,
                None,
                None,
                Vec::new(),
                Some(Vec::new()),
                crate::extension::RemoteExtensionManifest::default(),
                None,
            )
            .await
    }
}

fn native_lease_fixture() -> NativeLeaseFixture {
    native_lease_fixture_with_account(None)
}

fn native_lease_fixture_with_account(profile_root: Option<&std::path::Path>) -> NativeLeaseFixture {
    let mut config = crate::config::DaemonConfig::for_tests();
    config.accept_remote_leases = true;
    let mut app = crate::app::DaemonApp::bootstrap(config).expect("worker app should bootstrap");
    let account_profile = match profile_root {
        Some(root) => {
            app.provider_account_profile_registry()
                .link_existing("owner-native-reuse", "claude", "work", root)
                .unwrap()
                .profile_id
        }
        None => "work".to_string(),
    };
    let (leased_agent_id, matching_request) = {
        let mut runtime = RemoteLeaseRuntime::new(&mut app);
        let lease = runtime
            .create_execution_lease(
                "home-kernel-native-reuse",
                "home-session-native-reuse",
                "home-agent-native-reuse",
                false,
                "owner-native-reuse",
            )
            .expect("execution lease should create");
        let leased_agent = runtime
            .create_leased_agent(
                &lease.id,
                "claude",
                &account_profile,
                Some("sonnet".to_string()),
                None,
                None,
                None,
                None,
                None,
                None,
            )
            .expect("leased agent should create");
        let request = runtime
            .prepare_leased_native_provider_launch(
                &leased_agent.id,
                "claude",
                "claude",
                &account_profile,
                "sonnet",
                None,
                None,
                None,
                Vec::new(),
                Some(Vec::new()),
                crate::extension::RemoteExtensionManifest::default(),
            )
            .expect("matching native request should prepare");
        (leased_agent.id, request)
    };

    NativeLeaseFixture {
        state: runtime_state_from_app(app),
        leased_agent_id,
        matching_request,
    }
}

pub(super) fn runtime_state_from_app(app: crate::app::DaemonApp) -> KernelRuntimeState {
    let config_projection = app.config_projection_store();
    let session_store = app.session_state_store();
    let agent_store = app.agents().clone();
    let attachment_store = app.attachments().clone();
    let provider_store = app.providers().clone();
    let provider_process_tracking = app.provider_process_tracking_store();
    let slice_store = app.slices();
    let session_projection = app.session_state_projection_store();
    let provider_run_projection = app.provider_run_projection_store();
    let operational_history_store = app.operational_history_store();
    let durable_state_store = app.durable_state_store();
    let prompt_state_owner = app.prompt_state_owner();
    let active_turns = app.active_turn_store();
    let prompt_activity = app.prompt_activity_store();
    let prompt_workspace_claims = app.prompt_workspace_claim_store();
    let structured_output_records = app.structured_output_record_store();
    let terminal_stream = app.terminal_stream_store();
    let workflow_design_events = app.workflow_design_event_store();
    let metaagent_events = app.metaagent_event_store();
    let workspace_coordinator = app.workspace_coordinator();
    KernelRuntimeState::new_with_owned_state(
        Arc::new(Mutex::new(app)),
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

#[tokio::test]
async fn relay_native_launch_reuses_matching_live_worker_run_without_credential() {
    let fixture = native_lease_fixture();
    fixture.insert_live_run("worker-native-run-1");

    let run = fixture
        .launch("sonnet")
        .await
        .expect("matching live run should be reused without a cold credential");

    assert_eq!(run.id(), "worker-native-run-1");
    assert!(fixture
        .state
        .owned
        .provider_run_projection
        .is_leased_provider_run(run.id()));
    assert_eq!(fixture.state.owned.provider_store.list_runs().len(), 1);
}

#[tokio::test]
async fn relay_native_launch_rejects_mismatched_live_worker_selection() {
    let fixture = native_lease_fixture();
    fixture.insert_live_run("worker-native-run-mismatch");

    let error = fixture
        .launch("opus")
        .await
        .expect_err("a live native run with a different model must not be replaced");

    assert!(matches!(error, DaemonError::InvalidProviderRunState { .. }));
    assert_eq!(fixture.state.owned.provider_store.list_runs().len(), 1);
}

#[tokio::test]
async fn concurrent_relay_native_launches_serialize_and_reuse_one_worker_run() {
    let fixture = native_lease_fixture();
    let operation = fixture
        .state
        .leased_agent_operations
        .lock(&fixture.leased_agent_id)
        .await;
    let first_state = fixture.state.clone();
    let first_leased_agent_id = fixture.leased_agent_id.clone();
    let mut first = tokio::spawn(async move {
        first_state
            .launch_relay_leased_native_provider_run(
                &first_leased_agent_id,
                "claude",
                "claude",
                "work",
                "sonnet",
                None,
                None,
                None,
                Vec::new(),
                Some(Vec::new()),
                crate::extension::RemoteExtensionManifest::default(),
                None,
            )
            .await
    });
    let second_state = fixture.state.clone();
    let second_leased_agent_id = fixture.leased_agent_id.clone();
    let mut second = tokio::spawn(async move {
        second_state
            .launch_relay_leased_native_provider_run(
                &second_leased_agent_id,
                "claude",
                "claude",
                "work",
                "sonnet",
                None,
                None,
                None,
                Vec::new(),
                Some(Vec::new()),
                crate::extension::RemoteExtensionManifest::default(),
                None,
            )
            .await
    });

    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(25), &mut first)
            .await
            .is_err(),
        "the first launch must wait for the leased-agent operation lane"
    );
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(25), &mut second)
            .await
            .is_err(),
        "the concurrent launch must wait for the same operation lane"
    );
    fixture.insert_live_run("worker-native-run-serialized");
    drop(operation);

    let first_run = first
        .await
        .expect("first launch task should finish")
        .expect("first launch should reuse the serialized run");
    let second_run = second
        .await
        .expect("second launch task should finish")
        .expect("second launch should reuse the serialized run");
    assert_eq!(first_run.id(), "worker-native-run-serialized");
    assert_eq!(second_run.id(), first_run.id());
    assert_eq!(fixture.state.owned.provider_store.list_runs().len(), 1);
}

// MP-08 / MP-10 / MP-11: a typed cold-launch preflight rejection is
// retryable with the selected credential, not a settled prompt rejection.
#[tokio::test]
async fn cold_claude_credential_preflight_does_not_fence_the_home_prompt() {
    let profile_root = crate::test_support::TestWorktree::new("cold-claude-credential-profile");
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(profile_root.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let fixture = native_lease_fixture_with_account(Some(profile_root.path()));
    let context = crate::transport::relay_peer::RemoteGitTurnContext {
        home_session_id: "home-session-native-reuse".into(),
        home_agent_id: "home-agent-native-reuse".into(),
        home_prompt_id: "home-cold-credential-preflight".into(),
        home_turn_id: "home-cold-credential-preflight".into(),
        source_attachment_id: None,
        workspace_live_sync_mode: None,
        prompt_origin: None,
        external_provider: None,
        external_provider_session_id: None,
        external_provider_turn_id: None,
        prompt_summary: "resume the same native conversation".into(),
    };
    let error = fixture
        .state
        .submit_relay_leased_prompt(
            &fixture.leased_agent_id,
            crate::transport::relay_peer::RelayAgentExecutionProfile {
                provider: "claude".into(),
                account_profile: fixture.matching_request.account_profile.clone(),
                model: Some("sonnet".into()),
                effort: None,
            },
            None,
            "resume the same native conversation",
            "",
            Vec::new(),
            None,
            Some(context),
            Vec::new(),
            Some(Vec::new()),
            crate::extension::RemoteExtensionManifest::default(),
            None,
        )
        .await
        .expect_err("a cold run requires the selected Claude credential");
    assert!(
        matches!(
            &error,
            DaemonError::LocalTransport {
                operation: "launch remote provider without credential",
                ..
            }
        ),
        "expected credential preflight, got {error:?}"
    );
    let receipt = fixture
        .state
        .query_relay_leased_prompt_receipt(
            &fixture.leased_agent_id,
            "home-cold-credential-preflight",
        )
        .await
        .expect("the same home prompt can query its preflight");
    assert!(
        receipt.is_none(),
        "credential preflight must leave this exact home prompt retryable"
    );
    let leased_id = fixture.leased_agent_id.clone();
    let begun = fixture
        .state
        .with_app_side_effect(move |app| {
            RemoteLeaseRuntime::new(app)
                .begin_leased_prompt_receipt(&leased_id, "home-cold-credential-preflight")
        })
        .await
        .unwrap();
    assert!(
        begun.is_none(),
        "credential retry must be a fresh admission"
    );
    let receipt = fixture
        .state
        .query_relay_leased_prompt_receipt(
            &fixture.leased_agent_id,
            "home-cold-credential-preflight",
        )
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        receipt.phase,
        crate::transport::relay_peer::LeasedPromptReceiptPhase::SteerDispatching
    );
}

// MP-08 / MP-10 / MP-11: delaying the retryable preflight must not
// remove the durable fence for a genuinely unavailable selected profile.
#[tokio::test]
async fn unavailable_claude_profile_keeps_a_definitive_rejection_receipt() {
    let fixture = native_lease_fixture();
    let home_prompt_id = "home-unavailable-profile";
    let context = crate::transport::relay_peer::RemoteGitTurnContext {
        home_session_id: "home-session-native-reuse".into(),
        home_agent_id: "home-agent-native-reuse".into(),
        home_prompt_id: home_prompt_id.into(),
        home_turn_id: home_prompt_id.into(),
        source_attachment_id: None,
        workspace_live_sync_mode: None,
        prompt_origin: None,
        external_provider: None,
        external_provider_session_id: None,
        external_provider_turn_id: None,
        prompt_summary: "unavailable selected profile".into(),
    };
    let error = fixture
        .state
        .submit_relay_leased_prompt(
            &fixture.leased_agent_id,
            crate::transport::relay_peer::RelayAgentExecutionProfile {
                provider: "claude".into(),
                account_profile: "work".into(),
                model: Some("sonnet".into()),
                effort: None,
            },
            None,
            "unavailable selected profile",
            "",
            Vec::new(),
            None,
            Some(context),
            Vec::new(),
            Some(Vec::new()),
            crate::extension::RemoteExtensionManifest::default(),
            None,
        )
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        DaemonError::LocalTransport {
            operation: "update leased agent profile",
            ..
        }
    ));
    let receipt = fixture
        .state
        .query_relay_leased_prompt_receipt(&fixture.leased_agent_id, home_prompt_id)
        .await
        .unwrap()
        .expect("permanent rejection must remain durable");
    assert_eq!(
        receipt.phase,
        crate::transport::relay_peer::LeasedPromptReceiptPhase::SteerRejected
    );
}

#[tokio::test]
async fn stale_active_run_hint_requires_cold_claude_credential_on_worker() {
    let fixture = native_lease_fixture();

    let error = fixture
        .launch("sonnet")
        .await
        .expect_err("a missing hinted worker run must not silently cold-launch Claude");

    match error {
        DaemonError::LocalTransport { operation, message } => {
            assert_eq!(operation, "launch remote provider without credential");
            assert!(message.contains(
                crate::transport::relay_peer::REMOTE_PROVIDER_LAUNCH_CREDENTIAL_REQUIRED_CODE
            ));
        }
        other => panic!("unexpected worker cold-launch error: {other}"),
    }
    assert!(fixture.state.owned.provider_store.list_runs().is_empty());
}

#[tokio::test]
async fn leased_permission_session_grant_ends_after_a_scope_round_trip() {
    use crate::runtime::state::claude_permission_rules::{session_grant, GrantScope};
    use std::path::Path;
    let fixture = native_lease_fixture();
    let state = &fixture.state;
    let agent = state
        .update_relay_leased_agent_config(
            &fixture.leased_agent_id,
            AgentExecutionMode::Build,
            AgentPermissionLevel::Required,
        )
        .await
        .unwrap();
    let scope = GrantScope {
        working_directory: Some(Path::new("/work/app").to_path_buf()),
        permission_level: AgentPermissionLevel::Required,
        execution_mode: AgentExecutionMode::Build,
    };
    let grant = session_grant(
        "Bash",
        &serde_json::json!({"command":"cargo test"}),
        Some(Path::new("/work/app")),
    )
    .unwrap();
    for (mode, level) in [
        (AgentExecutionMode::Plan, AgentPermissionLevel::Required),
        (AgentExecutionMode::Build, AgentPermissionLevel::Yolo),
    ] {
        state
            .claude_session_grants
            .remember(&agent.backing_agent_id, scope.clone(), &grant);
        state
            .update_relay_leased_agent_config(&fixture.leased_agent_id, mode, level)
            .await
            .unwrap();
        state
            .update_relay_leased_agent_config(
                &fixture.leased_agent_id,
                AgentExecutionMode::Build,
                AgentPermissionLevel::Required,
            )
            .await
            .unwrap();
        assert!(state
            .claude_session_grants
            .covering(&agent.backing_agent_id, &scope, &grant)
            .is_none());
    }
}
