//! MP-08 / MP-10 / MP-11 S02: task consent covers its session, not another one.
use super::tests::{fixture_with_options, popup, PASSKEY};
use super::*;

fn enroll_fixture(f: &super::tests::Fixture) {
    let mut config = f.state.owned.config_projection.snapshot();
    config.cloud_relay = Some(crate::config::PersistedCloudRelayProfile {
        user_id: "enrolled-owner".into(),
        ..Default::default()
    });
    f.state.owned.config_projection.update(config);
}

fn external_grant_for(f: &super::tests::Fixture, owner: &str) -> String {
    let grant_id = f.state.insert_access_grant_for_test(&f.request.session_id);
    f.state
        .owned
        .kernel_access
        .lock()
        .unwrap()
        .grants
        .get_mut(&grant_id)
        .unwrap()
        .summary
        .owner_user_id = owner.into();
    grant_id
}

async fn approve_enrolled_sudo(
    f: &super::tests::Fixture,
    mut pending: tokio::task::JoinHandle<Result<LocalDaemonResponse, DaemonError>>,
) -> LocalDaemonResponse {
    let consent = tokio::select! {
        consent = super::tests::popup_for(&f.state, "enrolled-owner") => consent,
        result = &mut pending => panic!("enrolled external grant was refused: {result:?}"),
    };
    f.state
        .answer_terminal_runtime_interaction(
            &consent.session_id,
            &consent.interaction_id,
            "approve",
            None,
            Some("enrolled-owner"),
            Some(&ApprovalPasskey::new(PASSKEY)),
            None,
            Some(KernelConnectionClass::Terminal),
        )
        .await
        .unwrap();
    pending.await.unwrap().unwrap()
}

// MP-08/MP-10/MP-11: an external grant of the enrolled owner acts for the host of a
// pre-enrollment room; a grant of anyone else does not.
#[tokio::test]
async fn enrolled_external_grant_requests_sudo_in_its_pre_enrollment_room() {
    let f = super::tests::fixture();
    enroll_fixture(&f);
    let stranger = external_grant_for(&f, "another-user");
    assert!(f
        .state
        .request_kernel_sudo(&stranger, super::tests::external_request(&f))
        .await
        .is_err());
    let grant_id = external_grant_for(&f, "enrolled-owner");
    let state = f.state.clone();
    let request = super::tests::external_request(&f);
    let requesting =
        tokio::spawn(async move { state.request_kernel_sudo(&grant_id, request).await });
    let result = approve_enrolled_sudo(&f, requesting).await;
    assert!(matches!(
        result,
        LocalDaemonResponse::KernelSudoRequested { .. }
    ));
    let turn = f.state.list_sudo_turns("enrolled-owner").pop().unwrap();
    assert_eq!(turn.owner_user_id, "enrolled-owner");
}

#[tokio::test]
async fn enrolled_external_grant_submits_sudo_prompt_in_its_pre_enrollment_room() {
    let f = super::tests::fixture();
    enroll_fixture(&f);
    let stranger = external_grant_for(&f, "another-user");
    assert!(f
        .state
        .submit_external_sudo_prompt(&stranger, f.request.clone())
        .await
        .is_err());
    let grant_id = external_grant_for(&f, "enrolled-owner");
    let state = f.state.clone();
    let request = f.request.clone();
    let submitting =
        tokio::spawn(async move { state.submit_external_sudo_prompt(&grant_id, request).await });
    approve_enrolled_sudo(&f, submitting).await;
    assert_eq!(f.state.list_sudo_turns("enrolled-owner").len(), 1);
}

// MP-08/MP-10/MP-11 G06: enrollment preserves the host's existing attachment.
#[tokio::test]
async fn enrolled_host_can_authorize_sudo_in_its_pre_enrollment_room() {
    let f = fixture_with_options(None, true);
    enroll_fixture(&f);
    assert!(f
        .state
        .submit_sudo_prompt(f.request.clone(), "another-user", "sudo-terminal")
        .await
        .is_err());
    let state = f.state.clone();
    let request = f.request.clone();
    let mut starting = tokio::spawn(async move {
        state
            .submit_sudo_prompt(request, "enrolled-owner", "sudo-terminal")
            .await
    });
    let consent = tokio::select! {
        consent = super::tests::popup_for(&f.state, "enrolled-owner") => consent,
        result = &mut starting => panic!("enrolled host could not authorize sudo: {result:?}"),
    };
    f.state
        .answer_terminal_runtime_interaction(
            &consent.session_id,
            &consent.interaction_id,
            "approve",
            None,
            Some("enrolled-owner"),
            Some(&ApprovalPasskey::new(PASSKEY)),
            None,
            Some(KernelConnectionClass::Terminal),
        )
        .await
        .unwrap();
    starting.await.unwrap().unwrap();
    let turn = f.state.list_sudo_turns("enrolled-owner").pop().unwrap();
    assert_eq!(turn.owner_user_id, "enrolled-owner");
    assert!(f.state.sudo_task_authorizes_run(&f.run).await);
    let result = f
        .router
        .dispatch_authenticated_runtime_tool_call(
            "sudo-fixture-bearer",
            "chariox_kernel_request",
            serde_json::json!({"request":{"AliasSession":{
                "session_id":turn.session_id,"alias":"enrolled-sudo-work"
            }}}),
        )
        .await
        .unwrap();
    assert!(result.ok, "{result:?}");
    assert_eq!(
        f.state
            .revoke_sudo(
                Some("enrolled-owner"),
                Some(&turn.entry_id),
                "owner_revoked"
            )
            .unwrap(),
        1
    );
}

#[tokio::test]
async fn enrolled_host_can_list_and_revoke_its_existing_sudo_window() {
    let f = fixture_with_options(None, true);
    let turn = super::window_tests::open(&f, None).await;
    enroll_fixture(&f);
    assert!(f.state.list_sudo_turns("another-user").is_empty());
    assert_eq!(f.state.list_sudo_turns("enrolled-owner").len(), 1);
    assert_eq!(
        f.state
            .revoke_sudo(Some("another-user"), Some(&turn.entry_id), "owner_revoked")
            .unwrap(),
        0
    );
    assert_eq!(
        f.state
            .revoke_sudo(
                Some("enrolled-owner"),
                Some(&turn.entry_id),
                "owner_revoked"
            )
            .unwrap(),
        1
    );
}

#[tokio::test]
async fn enrolled_host_can_extend_its_existing_sudo_window() {
    let f = fixture_with_options(None, true);
    let turn = super::window_tests::open(&f, None).await;
    enroll_fixture(&f);
    let request = ExtendKernelSudoRequest {
        session_id: turn.session_id.clone(),
        attachment_id: f.request.attachment_id.clone(),
        entry_id: turn.entry_id.clone(),
        revision: turn.revision,
    };
    assert!(f
        .state
        .extend_sudo_window(request.clone(), "another-user")
        .await
        .is_err());
    let state = f.state.clone();
    let mut extending =
        tokio::spawn(async move { state.extend_sudo_window(request, "enrolled-owner").await });
    let consent = tokio::select! {
        consent = super::tests::popup_for(&f.state, "enrolled-owner") => consent,
        result = &mut extending => panic!("enrolled host could not extend sudo: {result:?}"),
    };
    f.state
        .answer_terminal_runtime_interaction(
            &consent.session_id,
            &consent.interaction_id,
            "approve",
            Some("120"),
            Some("enrolled-owner"),
            Some(&ApprovalPasskey::new(PASSKEY)),
            None,
            Some(KernelConnectionClass::Terminal),
        )
        .await
        .unwrap();
    extending.await.unwrap().unwrap();
    let extended = f.state.list_sudo_turns("enrolled-owner").pop().unwrap();
    assert_eq!(extended.entry_id, turn.entry_id);
    assert_eq!(extended.duration_minutes, 120);
    assert!(extended.revision > turn.revision);
    f.state
        .revoke_sudo(
            Some("enrolled-owner"),
            Some(&turn.entry_id),
            "fixture_cleanup",
        )
        .unwrap();
}

#[tokio::test]
async fn sudo_task_consent_covers_three_operations_but_not_another_session() {
    let f = fixture_with_options(None, true);
    let state = f.state.clone();
    let request = f.request.clone();
    let starting = tokio::spawn(async move {
        state
            .submit_sudo_prompt(request, "local", "sudo-terminal")
            .await
    });
    let consent = popup(&f.state).await;
    assert!(
        !consent.message.contains("changing focus"),
        "owner sheet must match room admission"
    );
    f.state
        .answer_terminal_runtime_interaction(
            &consent.session_id,
            &consent.interaction_id,
            "approve",
            None,
            Some("local"),
            Some(&ApprovalPasskey::new(PASSKEY)),
            None,
            Some(KernelConnectionClass::Terminal),
        )
        .await
        .unwrap();
    starting.await.unwrap().unwrap();
    let window = f.state.list_sudo_turns("local").pop().unwrap();
    for alias in ["task-first", "task-second", "task-third"] {
        let request = LocalDaemonRequest::AliasSession(AliasSessionRequest {
            session_id: window.session_id.clone(),
            alias: alias.into(),
        });
        // The normal provider tool path includes both admission and pre-effect
        // checks. A duplicate popup causes this bounded wait to fail on base.
        let reply = tokio::time::timeout(
            Duration::from_millis(500),
            f.router.dispatch_authenticated_runtime_tool_call(
                "sudo-fixture-bearer",
                "chariox_kernel_request",
                serde_json::json!({"request": request}),
            ),
        )
        .await
        .expect("covered task operation asked for consent again")
        .unwrap();
        assert!(reply.ok, "{reply:?}");
        assert_eq!(
            f.state
                .owned
                .session_store
                .get_session(&window.session_id)
                .unwrap()
                .alias(),
            Some(alias)
        );
        assert!(f.state.passkey_prompts_for("local").is_empty());
    }
    // Human focus grants resource authority; a sudo provider cannot mint it.
    for request in [
        LocalDaemonRequest::FocusAgent(FocusAgentRequest {
            session_id: window.session_id.clone(),
            agent_id: window.agent_id.clone(),
        }),
        LocalDaemonRequest::CycleAgentFocus(CycleAgentFocusRequest {
            session_id: window.session_id.clone(),
        }),
    ] {
        assert!(
            f.state
                .require_sudo_scope(&window.entry_id, &request)
                .is_err(),
            "focus must not be advertised as covered task scope"
        );
        let rejected = tokio::time::timeout(
            Duration::from_millis(500),
            f.router.dispatch_authenticated_runtime_tool_call(
                "sudo-fixture-bearer",
                "chariox_kernel_request",
                serde_json::json!({"request": request}),
            ),
        )
        .await
        .expect("focus refusal must not ask for scope consent")
        .unwrap_err();
        assert!(rejected.to_string().contains("focus"), "{rejected}");
        assert!(f.state.passkey_prompts_for("local").is_empty());
    }
    let other = crate::session::RuntimeSession::new("outside-task", None, "w", "wt", "m", "k");
    f.state.owned.session_store.write().restore_session(other);
    let router = f.router.clone();
    let outside = tokio::spawn(async move {
        router.dispatch_authenticated_runtime_tool_call(
            "sudo-fixture-bearer", "chariox_kernel_request",
            serde_json::json!({"request":{"AliasSession":{"session_id":"outside-task","alias":"new-scope"}}}),
        ).await
    });
    let added_scope = popup(&f.state).await;
    assert!(added_scope.interaction_id.contains(":scope:"));
    assert!(!outside.is_finished());
    f.state
        .answer_terminal_runtime_interaction(
            &added_scope.session_id,
            &added_scope.interaction_id,
            "refuse",
            None,
            Some("local"),
            None,
            None,
            Some(KernelConnectionClass::Terminal),
        )
        .await
        .unwrap();
    assert!(outside.await.unwrap().is_err());
    assert_eq!(
        f.state
            .owned
            .session_store
            .get_session("outside-task")
            .unwrap()
            .alias(),
        None
    );
    f.state
        .revoke_sudo(Some("local"), Some(&window.entry_id), "test_complete")
        .unwrap();
    assert!(f
        .state
        .authorize_sudo_request(
            &window.entry_id,
            &LocalDaemonRequest::AliasSession(AliasSessionRequest {
                session_id: window.session_id,
                alias: "after-revoke".into(),
            })
        )
        .is_err());
}

// MP-08 / MP-10 / MP-11: one sudo task can create an ordinary session in its workspace without a second passkey.
#[tokio::test]
async fn sudo_task_creates_a_same_workspace_session_without_another_passkey() {
    let f = fixture_with_options(None, true);
    let state = f.state.clone();
    let request = f.request.clone();
    let starting = tokio::spawn(async move {
        state
            .submit_sudo_prompt(request, "local", "sudo-terminal")
            .await
    });
    let consent = popup(&f.state).await;
    f.state
        .answer_terminal_runtime_interaction(
            &consent.session_id,
            &consent.interaction_id,
            "approve",
            None,
            Some("local"),
            Some(&ApprovalPasskey::new(PASSKEY)),
            None,
            Some(KernelConnectionClass::Terminal),
        )
        .await
        .unwrap();
    starting.await.unwrap().unwrap();
    let window = f.state.list_sudo_turns("local").pop().unwrap();
    let session = f
        .state
        .owned
        .session_store
        .get_session(&window.session_id)
        .unwrap();
    let request =
        crate::session::CreateSessionRequest::new(session.workspace_id(), session.worktree_id())
            .with_alias("same-task-session");
    let reply = tokio::time::timeout(
        Duration::from_secs(5),
        f.router.dispatch_authenticated_runtime_tool_call(
            "sudo-fixture-bearer",
            "chariox_kernel_request",
            serde_json::json!({"request":{"CreateSession": request}}),
        ),
    )
    .await
    .expect("same-workspace session creation asked for a second passkey")
    .unwrap();
    let new_id = reply.payload["SessionCreated"]["session"]["id"]
        .as_str()
        .expect("created session");
    let created = f.state.owned.session_store.get_session(new_id).unwrap();
    assert_eq!(created.workspace_id(), session.workspace_id());
    assert_eq!(created.worktree_id(), session.worktree_id());
    assert_eq!(created.owner_user_id(), window.owner_user_id);
    assert_ne!(created.id(), window.session_id);
    assert!(f.state.passkey_prompts_for("local").is_empty());
    assert!(
        f.state
            .list_sudo_turns("local")
            .iter()
            .all(|t| t.session_id == window.session_id),
        "new session must not inherit elevation"
    );

    for change in [
        "workspace",
        "worktree",
        "owner",
        "hidden",
        "metaagent",
        "worker",
        "slice",
        "account",
    ] {
        let mut outside = request.clone();
        match change {
            "workspace" => outside.workspace_id = "unrelated-workspace".into(),
            "worktree" => outside.worktree_id = "unrelated-worktree".into(),
            "owner" => outside.owner_user_id = "another-owner".into(),
            "hidden" => outside.hidden = true,
            "metaagent" => outside.metaagent = true,
            "worker" => outside.kernel_ref = Some("unrelated-worker".into()),
            "slice" => outside.slice_ref = Some("new".into()),
            "account" => {
                outside.agent_defaults = Some(
                    crate::session::SessionAgentDefaults::new("codex")
                        .with_account_profile("unrelated-profile"),
                )
            }
            _ => unreachable!(),
        }
        assert!(
            f.state
                .require_sudo_scope(
                    &window.entry_id,
                    &LocalDaemonRequest::CreateSession(outside)
                )
                .is_err(),
            "new resource {change} must not borrow this workspace scope"
        );
    }
    f.state
        .revoke_sudo(Some("local"), Some(&window.entry_id), "explicit_revoke")
        .unwrap();
    let denied = tokio::time::timeout(
        Duration::from_secs(5),
        f.router.dispatch_authenticated_runtime_tool_call(
            "sudo-fixture-bearer",
            "chariox_kernel_request",
            serde_json::json!({"request":{"CreateSession": request}}),
        ),
    )
    .await
    .expect("revoked work must refuse immediately")
    .unwrap_err();
    assert!(denied.to_string().contains("sudo"));
    assert!(f.state.passkey_prompts_for("local").is_empty());
}

// MP-08 / MP-10 / MP-11 G06: with the kernel's default agent settings configured, as on any
// signed-in kernel, the session lane re-checks the create after filling those defaults in.
// An enrolled (non-local) owner's sudo task must still create its same-workspace session.
#[tokio::test]
async fn sudo_task_creates_a_same_workspace_session_with_kernel_default_agent_settings() {
    const OWNER: &str = "enrolled-owner";
    let f = super::tests::fixture_for_owner(None, true, "dev-stub", "dev-stub", None, OWNER);
    let state = f.state.clone();
    let request = f.request.clone();
    let starting = tokio::spawn(async move {
        state
            .submit_sudo_prompt(request, OWNER, "sudo-terminal")
            .await
    });
    let mut starting = starting;
    let consent = tokio::select! {
        consent = super::tests::popup_for(&f.state, OWNER) => consent,
        ended = &mut starting => panic!("sudo start ended before its popup: {ended:?}"),
    };
    f.state
        .answer_terminal_runtime_interaction(
            &consent.session_id,
            &consent.interaction_id,
            "approve",
            None,
            Some(OWNER),
            Some(&ApprovalPasskey::new(PASSKEY)),
            None,
            Some(KernelConnectionClass::Terminal),
        )
        .await
        .unwrap();
    starting.await.unwrap().unwrap();
    for (key, value) in [
        ("providers.default", "codex"),
        ("providers.model", "gpt-test"),
    ] {
        f.state
            .set_user_config_value(key.to_string(), value.to_string())
            .await
            .unwrap();
    }
    let window = f.state.list_sudo_turns(OWNER).pop().unwrap();
    let session = f
        .state
        .owned
        .session_store
        .get_session(&window.session_id)
        .unwrap();
    let request =
        crate::session::CreateSessionRequest::new(session.workspace_id(), session.worktree_id())
            .with_alias("enrolled-task-session");
    let reply = tokio::time::timeout(
        Duration::from_secs(5),
        f.router.dispatch_authenticated_runtime_tool_call(
            "sudo-fixture-bearer",
            "chariox_kernel_request",
            serde_json::json!({"request":{"CreateSession": request}}),
        ),
    )
    .await
    .expect("same-workspace session creation asked for a second passkey");
    let reply = reply.unwrap_or_else(|error| panic!("create failed: {error:?}"));
    let new_id = reply.payload["SessionCreated"]["session"]["id"]
        .as_str()
        .expect("created session");
    let created = f.state.owned.session_store.get_session(new_id).unwrap();
    assert_eq!(created.owner_user_id(), OWNER);
}

// MP-08 / MP-10 / MP-11: a request naming an empty session id is rejected up front, not held for an owner approval.
#[tokio::test]
async fn sudo_request_with_an_empty_session_id_is_rejected_without_an_owner_prompt() {
    let f = fixture_with_options(None, true);
    let state = f.state.clone();
    let request = f.request.clone();
    let starting = tokio::spawn(async move {
        state
            .submit_sudo_prompt(request, "local", "sudo-terminal")
            .await
    });
    let consent = popup(&f.state).await;
    f.state
        .answer_terminal_runtime_interaction(
            &consent.session_id,
            &consent.interaction_id,
            "approve",
            None,
            Some("local"),
            Some(&ApprovalPasskey::new(PASSKEY)),
            None,
            Some(KernelConnectionClass::Terminal),
        )
        .await
        .unwrap();
    starting.await.unwrap().unwrap();
    let reply = tokio::time::timeout(
        Duration::from_secs(2),
        f.router.dispatch_authenticated_runtime_tool_call(
            "sudo-fixture-bearer",
            "chariox_kernel_request",
            serde_json::json!({"request":{"GetSessionHistoryOutline":{"session_id":"","include_prefetched_content":false}}}),
        ),
    )
    .await
    .expect("an empty session id waited for an owner approval");
    let error = reply.unwrap_err().to_string();
    assert!(error.contains("empty session_id"), "{error}");
    assert!(f.state.passkey_prompts_for("local").is_empty());
}
