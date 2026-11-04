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
