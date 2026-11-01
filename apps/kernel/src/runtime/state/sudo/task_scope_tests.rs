//! MP-08 / MP-10 / MP-11 S02: the task consent covers the kernel; no second passkey.
use super::tests::{fixture_with_options, popup, PASSKEY};
use super::*;

#[tokio::test]
async fn sudo_task_consent_covers_three_operations_and_another_session_without_another_passkey() {
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
    // A live window covers the whole kernel, so another session needs no
    // second passkey either.
    let other = crate::session::RuntimeSession::new("outside-task", None, "w", "wt", "m", "k");
    f.state.owned.session_store.write().restore_session(other);
    let outside = tokio::time::timeout(
        Duration::from_millis(500),
        f.router.dispatch_authenticated_runtime_tool_call(
            "sudo-fixture-bearer",
            "chariox_kernel_request",
            serde_json::json!({"request":{"AliasSession":{"session_id":"outside-task","alias":"new-scope"}}}),
        ),
    )
    .await
    .expect("another session asked for consent again")
    .unwrap();
    assert!(outside.ok, "{outside:?}");
    assert_eq!(
        f.state
            .owned
            .session_store
            .get_session("outside-task")
            .unwrap()
            .alias(),
        Some("new-scope")
    );
    assert!(f.state.passkey_prompts_for("local").is_empty());
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

// MP-08 / MP-10 / MP-11: "/sudo or an existing grant means zero prompts". A
// live window answers a harmless kernel request without a second passkey.
#[tokio::test]
async fn sudo_window_answers_a_read_only_kernel_request_without_another_passkey() {
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
            serde_json::json!({"request":{"GetDaemonHealth":null}}),
        ),
    )
    .await;
    assert!(
        f.state.passkey_prompts_for("local").is_empty(),
        "a live sudo window asked the owner for another passkey"
    );
    assert!(reply.is_ok(), "the request waited for an owner approval");
}
