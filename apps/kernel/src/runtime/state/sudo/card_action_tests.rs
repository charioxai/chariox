//! Person cards (PCD-06, PCD-12): an agent asks for one specific action; the person decides.
use super::tests::{fixture_with_options, Fixture, PASSKEY};
use super::*;

const TOKEN: &str = "sudo-fixture-bearer";

type CardCall = tokio::task::JoinHandle<Result<serde_json::Value, DaemonError>>;

fn raise(f: &Fixture, title: &str, request: serde_json::Value) -> CardCall {
    let router = f.router.clone();
    let args = serde_json::json!({
        "title": title, "message": "A harmless read.",
        "choices": [{"id": "ok", "label": "OK", "reply": "ok"}],
        "action": {"request": request}
    });
    tokio::spawn(async move {
        Box::pin(router.dispatch_authenticated_runtime_tool_call(
            TOKEN,
            "chariox.request_popup",
            args,
        ))
        .await
        .map(|result| result.payload)
    })
}

async fn card(f: &Fixture) -> PasskeyPrompt {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let Some(prompt) = f
                .state
                .passkey_prompts_for("local")
                .into_iter()
                .find(|p| p.kind == PasskeyPromptKind::CriticalApproval)
            {
                return prompt;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("the action card reaches the owner's terminals")
}

async fn answer(
    f: &Fixture,
    prompt: &PasskeyPrompt,
    choice: &str,
    passkey: Option<&str>,
) -> Result<(), DaemonError> {
    f.state
        .answer_terminal_runtime_interaction(
            &prompt.session_id,
            &prompt.interaction_id,
            choice,
            None,
            Some("local"),
            passkey.map(ApprovalPasskey::new).as_ref(),
            None,
            Some(KernelConnectionClass::Terminal),
        )
        .await
}

fn alias(f: &Fixture) -> Option<String> {
    f.state
        .owned
        .session_store
        .get_session(&f.request.session_id)
        .unwrap()
        .alias()
        .map(str::to_owned)
}

fn rename(f: &Fixture) -> serde_json::Value {
    serde_json::json!({"AliasSession": {"session_id": f.request.session_id, "alias": "renamed-by-card"}})
}

#[tokio::test]
async fn action_card_refuses_a_browser_view_instead_of_ignoring_it() {
    let f = fixture_with_options(None, true);
    let result = f
        .router
        .dispatch_authenticated_runtime_tool_call(
            TOKEN,
            "chariox.request_popup",
            serde_json::json!({
                "message":"Approve", "choices":[{"id":"ok","label":"OK","reply":"ok"}],
                "action":{"request":rename(&f)},
                "view":{"browser_region":{"tab_id":"foreign","ref":"backend:42"}}
            }),
        )
        .await;
    assert!(result.is_err());
    assert!(f.state.passkey_prompts_for("local").is_empty());
}

// A read of the asking agent's own Room asks nothing of the person: the agent gets the answer at once.
#[tokio::test]
async fn a_read_of_the_agents_own_room_runs_without_a_card() {
    let f = fixture_with_options(None, true);
    let reads = [
        serde_json::json!({"ListAgents": {"session_id": f.request.session_id}}),
        serde_json::json!({"GetSessionState": {"session_id": f.request.session_id}}),
    ];
    for read in reads {
        let result = tokio::time::timeout(Duration::from_secs(5), raise(&f, "Look around", read))
            .await
            .expect("a read of the agent's own Room waited for the person")
            .unwrap()
            .unwrap();
        assert_eq!(result["ran"], true, "{result}");
        assert_eq!(result["status"], "answered", "{result}");
        assert!(result.get("response").is_some(), "{result}");
    }
    assert!(f.state.passkey_prompts_for("local").is_empty());
}

#[tokio::test]
async fn a_card_cannot_carry_a_request_the_persons_terminal_keeps() {
    let f = fixture_with_options(None, true);
    let error = raise(&f, "Unlock", serde_json::json!({"ListCredentials": null}))
        .await
        .unwrap()
        .unwrap_err()
        .to_string();
    assert!(error.contains("cannot authorize"), "{error}");
    assert!(f.state.passkey_prompts_for("local").is_empty());
}

fn start_asking_prompt(f: &Fixture) {
    let session = f
        .state
        .owned
        .session_store
        .get_session(&f.request.session_id)
        .unwrap();
    let agent = f.request.target_agent_id.as_deref().unwrap();
    let prompts = &f.state.owned.prompt_state_owner;
    prompts
        .submit_prepared_prompt_with_queue_policy(
            &session,
            crate::session::PromptQueueItem::new(
                "action-asking-prompt",
                &f.request.attachment_id,
                agent,
                "Rename after approval",
                crate::session::PromptStatus::Queued,
            ),
            false,
            false,
        )
        .unwrap();
    let active = prompts
        .active_prompt_for_agent_snapshot(&session, agent)
        .unwrap();
    let mut started = active.clone();
    started.set_status(crate::session::PromptStatus::Running);
    assert!(prompts.replace_active_prompt_if_matches(&session, agent, &active, started));
}

async fn cancel_asking_prompt(f: &Fixture) {
    let request = crate::local::LocalDaemonRequest::CancelActivePrompt(
        crate::local::CancelActivePromptRequest {
            session_id: f.request.session_id.clone(),
            attachment_id: f.request.attachment_id.clone(),
            target_agent_id: f.request.target_agent_id.clone(),
            expected_prompt_id: Some("action-asking-prompt".into()),
        },
    );
    let command = crate::runtime::command::KernelCommand::from_local_request(
        "cancel-card-prompt",
        None,
        None,
        &request,
    );
    f.router.dispatch(command, request).await.unwrap();
}

#[tokio::test]
async fn reviewer_approved_action_rechecks_the_cancelled_asking_prompt() {
    let f = fixture_with_options(None, true);
    start_asking_prompt(&f);
    let request = serde_json::from_value(rename(&f)).unwrap();
    let mut action = f
        .state
        .raise_action_card(&f.run, &request, None)
        .await
        .unwrap();
    let prompt = card(&f).await;
    assert!(action.is_live());
    answer(&f, &prompt, "approve", Some(PASSKEY)).await.unwrap();
    assert_eq!(action.decision().await.unwrap().status, "answered");
    cancel_asking_prompt(&f).await;
    assert!(
        !action.is_live(),
        "resolved approval must not survive its asking prompt"
    );
    assert_ne!(alias(&f).as_deref(), Some("renamed-by-card"));
}

// Row 405: what the owner's agent may do itself runs at once: no card, no passkey, no prompt.
#[tokio::test]
async fn a_read_beyond_the_agents_room_runs_without_a_card() {
    let f = fixture_with_options(None, true);
    for read in [
        serde_json::json!({"ListSessions": null}),
        serde_json::json!({"GetDaemonHealth": null}),
    ] {
        let result = raise(&f, "Look around", read.clone())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(result["ran"], true, "{read}: {result}");
        assert_eq!(result["card"], false, "{read}: {result}");
    }
    assert!(f.state.passkey_prompts_for("local").is_empty());
}

#[tokio::test]
async fn an_owners_agents_action_runs_at_once_without_a_card() {
    let f = fixture_with_options(None, true);
    let result = raise(&f, "Rename", rename(&f)).await.unwrap().unwrap();
    assert_eq!(result["ran"], true, "{result}");
    assert_eq!(result["card"], false, "{result}");
    assert_eq!(alias(&f).as_deref(), Some("renamed-by-card"));
    assert!(f.state.passkey_prompts_for("local").is_empty());
}
