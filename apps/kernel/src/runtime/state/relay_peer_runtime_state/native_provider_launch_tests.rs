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
                crate::extension::RemoteExtensionManifest::default(),
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
                crate::extension::RemoteExtensionManifest::default(),
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
                crate::extension::RemoteExtensionManifest::default(),
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
async fn a_cold_claude_prompt_needs_no_credential_from_the_home() {
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
            crate::extension::RemoteExtensionManifest::default(),
            Default::default(),
        )
        .await
        .expect_err("the fake worker has no signed-in account to launch");
    // Row 577: the home sends no credential. Whatever the worker answers, it never asks the
    // home for one.
    assert_no_home_credential_request(&error);
}

fn assert_no_home_credential_request(error: &DaemonError) {
    let text = error.to_string();
    assert!(
        !text.contains("launch remote provider without credential")
            && !text.contains("provider_launch_credential_required"),
        "the worker must not ask the home for a credential: {error:?}"
    );
}

// A selected profile the worker does not hold answers with its own code and
// leaves the prompt unfenced: the home hands the account over again and submits
// the same prompt, which a durable rejection receipt would block.
#[tokio::test]
async fn unavailable_claude_profile_leaves_the_prompt_open_for_a_fresh_handover() {
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
            crate::extension::RemoteExtensionManifest::default(),
            Default::default(),
        )
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        DaemonError::LocalTransport {
            operation: crate::transport::relay_peer::REMOTE_PROVIDER_ACCOUNT_MISSING_OPERATION,
            ..
        }
    ));
    assert!(fixture
        .state
        .query_relay_leased_prompt_receipt(&fixture.leased_agent_id, home_prompt_id)
        .await
        .unwrap()
        .is_none());
}

#[tokio::test]
async fn stale_active_run_hint_cold_launches_from_the_workers_own_account() {
    let fixture = native_lease_fixture();

    let outcome = fixture.launch("sonnet").await;

    if let Err(error) = &outcome {
        assert_no_home_credential_request(error);
    }
}

#[tokio::test]
async fn stale_active_run_hint_without_worker_account_returns_recoverable_missing() {
    let fixture = native_lease_fixture();
    let error = fixture.launch("sonnet").await.unwrap_err();
    assert!(
        matches!(
            error,
            DaemonError::LocalTransport {
                operation: crate::transport::relay_peer::REMOTE_PROVIDER_ACCOUNT_MISSING_OPERATION,
                ..
            }
        ),
        "a cold native launch must request tracked-copy recovery: {error}"
    );
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

// Row 577: on an owner-enrolled worker the copy sits under the account authority (`local`), while
// the lease and its backing agent belong to the Cloud user; the launch still finds the copy.
#[tokio::test]
async fn a_cloud_owned_lease_launches_from_the_copy_under_the_account_authority() {
    let mut config = crate::config::DaemonConfig::for_tests();
    config.accept_remote_leases = true;
    config.cloud_relay = Some(crate::config::PersistedCloudRelayProfile {
        relay_url: "wss://cloud-relay".into(),
        user_id: "cloud-owner".into(),
        realm_id: "test".into(),
        ..Default::default()
    });
    let authority =
        crate::account_profile::provider_account_authority_owner_user_id(&config, "cloud-owner");
    assert_ne!(authority, "cloud-owner");
    let mut app = crate::app::DaemonApp::bootstrap(config).expect("worker app should bootstrap");
    let registry = app.provider_account_profile_registry();
    let copy = registry
        .create_managed(&authority, "claude", "copy")
        .unwrap();
    registry
        .record_transfer_origin(
            &authority,
            &crate::account_profile::ManagedContextProviderAccountReceipt {
                context_id: "context".into(),
                package_sha256: "package".into(),
                materialization_sha256: "materialization".into(),
                provider: "claude".into(),
                profile_id: copy.profile_id.clone(),
            },
            "home-kernel",
            "home-work",
        )
        .unwrap();
    let request = {
        let mut runtime = RemoteLeaseRuntime::new(&mut app);
        let lease = runtime
            .create_execution_lease(
                "home-kernel",
                "home-session",
                "home-agent",
                false,
                "cloud-owner",
            )
            .unwrap();
        let leased = runtime
            .create_leased_agent(
                &lease.id,
                "claude",
                "home-work",
                Some("sonnet".to_string()),
                None,
                None,
                None,
                None,
                None,
                None,
            )
            .unwrap();
        runtime
            .prepare_leased_native_provider_launch(
                &leased.id,
                "claude",
                "claude",
                "home-work",
                "sonnet",
                None,
                None,
                None,
                Vec::new(),
                crate::extension::RemoteExtensionManifest::default(),
            )
            .unwrap()
    };
    assert_eq!(request.account_profile, copy.profile_id);
    let state = runtime_state_from_app(app);
    let prepared = state
        .owned
        .prepare_leased_provider_launch_account(request)
        .expect("the copy is found under the account authority");
    assert_eq!(prepared.account_profile, copy.profile_id);
}
