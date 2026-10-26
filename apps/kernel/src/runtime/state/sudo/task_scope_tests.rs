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
