//! Row 539: an agent asking a person for a secret raises ONE card.
use super::*;
use std::time::Duration;

fn secret_request(entry: &str) -> serde_json::Value {
    serde_json::json!({
        "credential": {
            "id": entry,
            "description": "Cloudflare mailbox token",
            "use_binding": {"target": {"kind": "command_environment", "name": "CF_TOKEN", "origin": "https://api.cloudflare.com"}},
            "allowed_uses": ["env"],
            "injection": {"kind": "env", "name": "CF_TOKEN"}
        },
        "prompt": {"title": "Mailbox token", "message": "Paste the Cloudflare mailbox token."}
    })
}

async fn run_token(f: &Fixture) -> String {
    f.runtime
        .owned
        .provider_store
        .get_run(&f.run)
        .unwrap()
        .runtime_mcp_auth_token()
        .unwrap()
        .to_owned()
}

/// Both calls block on the person today; the test gives each a short window, then looks at the cards.
async fn ask_secret(f: &Fixture, entry: &str) {
    let token = run_token(f).await;
    let _ = tokio::time::timeout(
        Duration::from_millis(600),
        f.runtime.dispatch_authenticated_runtime_tool_call(
            &token,
            crate::transport::runtime_tools::REQUEST_CREDENTIAL_SECRET_TOOL,
            secret_request(entry),
        ),
    )
    .await;
}

fn secret_cards(f: &Fixture) -> usize {
    f.runtime
        .owned
        .session_store
        .get_session(&f.room)
        .unwrap()
        .active_interactions()
        .iter()
        .filter(|card| card.custom_choice().is_some_and(|c| c.input_kind() == crate::session::RuntimeInteractionInputKind::Secret))
        .count()
}

/// Runs `body` on a thread with the kernel runtime's stack: the credential tool future is deep in a debug build.
fn on_kernel_stack<F: std::future::Future<Output = ()> + Send + 'static>(body: F) {
    std::thread::Builder::new()
        .stack_size(crate::runtime_transport::KERNEL_RUNTIME_THREAD_STACK_SIZE)
        .spawn(move || {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap()
                .block_on(body)
        })
        .unwrap()
        .join()
        .unwrap();
}

/// C1: asking twice for the same secret leaves one card on screen.
#[test]
fn asking_twice_for_the_same_secret_raises_one_card() {
    on_kernel_stack(async {
        let f = fixture("secret-card-ask-twice", true).await;
        f.begin(&f.prompt_id);
        ask_secret(&f, "cf-mailbox-token").await;
        ask_secret(&f, "cf-mailbox-token").await;
        assert_eq!(secret_cards(&f), 1, "one card for one secret");
    });
}

/// C1, the retry that reaches the kernel while the first ask is still open (the first ask has not been
/// abandoned): one card, not two. Both calls stay alive until the cards are counted.
#[test]
fn a_retry_while_the_first_ask_is_open_joins_its_card() {
    on_kernel_stack(async {
        let f = fixture("secret-card-retry-joins", true).await;
        f.begin(&f.prompt_id);
        let token = run_token(&f).await;
        let call = |label: &'static str| {
            let (runtime, token) = (f.runtime.clone(), token.clone());
            async move {
                let result = runtime
                    .dispatch_authenticated_runtime_tool_call(
                        &token,
                        crate::transport::runtime_tools::REQUEST_CREDENTIAL_SECRET_TOOL,
                        secret_request("cf-mailbox-token"),
                    )
                    .await;
                (label, result.map(|r| r.payload.to_string()))
            }
        };
        let count = async {
            tokio::time::sleep(Duration::from_millis(1500)).await;
            secret_cards(&f)
        };
        let second = async {
            tokio::time::sleep(Duration::from_millis(500)).await;
            call("second").await
        };
        let cards = tokio::select! {
            ended = call("first") => panic!("the first ask returned before anyone answered: {ended:?}"),
            ended = second => panic!("the second ask returned before anyone answered: {ended:?}"),
            cards = count => cards,
        };
        assert_eq!(cards, 1, "a retry joins the open card");
    });
}
