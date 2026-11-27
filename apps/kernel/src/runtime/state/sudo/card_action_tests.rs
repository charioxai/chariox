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

// Restored from before row 405 (review P1, L15 squash 1088804874, Mac D174 (b)): an action card
// the agent raises itself is shown to the person, and Cancel runs nothing.
// A read that reaches past the agent's Room (another Room, every Room, a name lookup across
// Rooms, the daemon) is the person's to allow, like the writes a card carries: it waits for the
// passkey card, and Cancel returns nothing.
#[tokio::test]
async fn a_read_beyond_the_agents_room_waits_for_the_persons_card() {
    let f = fixture_with_options(None, true);
    let other_room = {
        let mut app = f.app.lock().await;
        crate::app::KernelSessionService::new(&mut app)
            .create_session(crate::session::CreateSessionRequest::new(
                "other-workspace",
                "other-worktree",
            ))
            .unwrap()
            .0
    };
    let other = other_room.id().to_string();
    let reads = [
        serde_json::json!({"ListSessions": null}),
        serde_json::json!({"GetSessionState": {"session_id": other}}),
        serde_json::json!({"ListAgents": {"session_id": other}}),
        serde_json::json!({"ListSessionMembers": {"session_id": other}}),
        serde_json::json!({"ResolveSession": {"session_ref": other}}),
        serde_json::json!({"GetDaemonHealth": null}),
    ];
    for read in reads {
        let call = raise(&f, "Look around", read.clone());
        let prompt = card(&f).await;
        answer(&f, &prompt, "cancel", None).await.unwrap();
        let result = call.await.unwrap().unwrap();
        assert_eq!(result["ran"], false, "{read}: {result}");
        assert_eq!(result["status"], "cancelled", "{read}: {result}");
        assert!(result.get("response").is_none(), "{read}: {result}");
    }
}

#[tokio::test]
async fn an_action_card_describes_the_request_in_the_kernels_words_and_cancel_runs_nothing() {
    let f = fixture_with_options(None, true);
    let before = alias(&f);
    let call = raise(&f, "Harmless read", rename(&f));
    let prompt = card(&f).await;
    assert_eq!(prompt.title, "Approve one action");
    assert!(
        prompt.message.contains("AliasSession"),
        "{}",
        prompt.message
    );
    assert!(
        prompt.message.contains("renamed-by-card"),
        "{}",
        prompt.message
    );
    assert!(
        !prompt.message.contains("harmless") && !prompt.message.contains("Harmless"),
        "the agent's own wording never reaches the card: {}",
        prompt.message
    );
    assert_eq!(prompt.refuse_choice_id, "cancel");
    answer(&f, &prompt, "cancel", None).await.unwrap();
    let result = call.await.unwrap().unwrap();
    assert_eq!(result["status"], "cancelled");
    assert_eq!(result["ran"], false);
    assert_eq!(alias(&f), before, "Cancel runs nothing");
    assert!(f.state.passkey_prompts_for("local").is_empty());
}

#[tokio::test]
async fn approving_with_the_passkey_runs_the_action_once_and_a_wrong_passkey_runs_nothing() {
    let f = fixture_with_options(None, true);
    let before = alias(&f);
    let call = raise(&f, "Harmless read", rename(&f));
    let prompt = card(&f).await;
    assert!(answer(&f, &prompt, "approve", Some("wrong passkey"))
        .await
        .is_err());
    assert_eq!(alias(&f), before, "a wrong passkey runs nothing");
    answer(&f, &prompt, "approve", Some(PASSKEY)).await.unwrap();
    // PCD-12: the second answer from another client finds the card gone.
    assert!(answer(&f, &prompt, "approve", Some(PASSKEY)).await.is_err());
    let result = call.await.unwrap().unwrap();
    assert_eq!(result["status"], "answered");
    assert_eq!(result["choice_id"], "approve");
    assert_eq!(result["ran"], true, "{result}");
    assert_eq!(alias(&f).as_deref(), Some("renamed-by-card"));
}

#[tokio::test]
async fn an_agent_that_stops_waiting_withdraws_its_card() {
    let f = fixture_with_options(None, true);
    let call = raise(&f, "Harmless read", rename(&f));
    let _ = card(&f).await;
    call.abort();
    let _ = call.await;
    tokio::time::timeout(Duration::from_secs(5), async {
        while !f.state.passkey_prompts_for("local").is_empty() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("the abandoned card is withdrawn");
}

#[tokio::test]
async fn reviewer_cancel_active_prompt_withdraws_its_action_card() {
    let f = fixture_with_options(None, true);
    let before = alias(&f);
    start_asking_prompt(&f);
    let call = raise(&f, "Rename", rename(&f));
    let prompt = card(&f).await;
    cancel_asking_prompt(&f).await;
    let late = answer(&f, &prompt, "approve", Some(PASSKEY)).await;
    assert!(
        late.is_err(),
        "a cancelled prompt must not retain an approvable action"
    );
    let result = call.await.unwrap().unwrap();
    assert_eq!(result["ran"], false);
    assert_eq!(alias(&f), before);
    assert!(f.state.passkey_prompts_for("local").is_empty());
}
