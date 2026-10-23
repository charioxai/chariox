//! MP-08 / MP-10 / MP-11 S02: task consent covers its session, not another one.
use super::tests::{fixture_with_options, popup, PASSKEY};
use super::*;

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
