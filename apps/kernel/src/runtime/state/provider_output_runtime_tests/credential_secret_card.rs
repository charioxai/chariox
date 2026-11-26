//! Row 539: an agent asking a person for a secret raises ONE card, gets its ids at once, and never
//! reads the secret.
use super::*;

const CANARY: &str = "canary-secret-7f3a91c2e4b8d5a6";

fn secret_request(entry: &str) -> serde_json::Value {
    secret_request_with(entry, None)
}

fn secret_request_with(entry: &str, timeout_sec: Option<u64>) -> serde_json::Value {
    let mut request = serde_json::json!({
        "credential": {
            "id": entry,
            "description": "Cloudflare mailbox token",
            "use_binding": {"target": {"kind": "command_environment", "name": "CF_TOKEN", "origin": "https://api.cloudflare.com"}},
            "allowed_uses": ["env"],
            "injection": {"kind": "env", "name": "CF_TOKEN"}
        },
        "prompt": {"title": "Mailbox token", "message": "Paste the Cloudflare mailbox token."}
    });
    if let Some(seconds) = timeout_sec {
        request["prompt"]["timeout_sec"] = seconds.into();
    }
    request
}

/// The Room's Vault is already unlocked, so the only card is the secret's own, not the Room's unlock card.
fn unlock_vault(f: &Fixture, dir: &std::path::Path) {
    std::fs::create_dir_all(dir).unwrap();
    let mut config = f.runtime.owned.config_projection.snapshot();
    config.user_config.credential_vault.path = dir.join("vault").display().to_string();
    let path = std::path::Path::new(&config.user_config.credential_vault.path).to_path_buf();
    crate::secret::create_chariox_encrypted_vault_for_test(&path, "synthetic-passphrase-only")
        .unwrap();
    crate::secret::unlock_chariox_encrypted_vault(
        &path,
        "synthetic-passphrase-only",
        crate::secret::VaultUnlockLease::KernelShutdown,
    )
    .unwrap();
    f.runtime.owned.config_projection.update(config);
}

/// A Vault and a credential registry of its own; the guard keeps the process environment ours.
struct Home {
    _env: crate::env_lock::EnvGuard,
    previous: Option<std::ffi::OsString>,
}

impl Home {
    fn new(f: &Fixture, name: &str) -> Self {
        let env = crate::env_lock::lock();
        let dir = std::env::temp_dir().join(format!(
            "chariox-secretcard-{name}-{}-{}",
            std::process::id(),
            crate::session::unix_epoch_ms()
        ));
        let previous = std::env::var_os("CHARIOX_HOME");
        std::env::set_var("CHARIOX_HOME", dir.join("home"));
        unlock_vault(f, &dir);
        Self {
            _env: env,
            previous,
        }
    }
}

impl Drop for Home {
    fn drop(&mut self) {
        match self.previous.take() {
            Some(value) => std::env::set_var("CHARIOX_HOME", value),
            None => std::env::remove_var("CHARIOX_HOME"),
        }
    }
}

async fn ask_secret(f: &Fixture, entry: &str) -> Result<serde_json::Value, DaemonError> {
    ask_secret_with(f, secret_request(entry)).await
}

async fn ask_secret_with(
    f: &Fixture,
    request: serde_json::Value,
) -> Result<serde_json::Value, DaemonError> {
    let token = f
        .runtime
        .owned
        .provider_store
        .get_run(&f.run)
        .unwrap()
        .runtime_mcp_auth_token()
        .unwrap()
        .to_owned();
    tokio::time::timeout(
        Duration::from_secs(2),
        f.runtime.dispatch_authenticated_runtime_tool_call(
            &token,
            crate::transport::runtime_tools::REQUEST_CREDENTIAL_SECRET_TOOL,
            request,
        ),
    )
    .await
    .expect("the ask returns at once, it does not wait for the person")
    .map(|result| result.payload)
}

fn secret_cards(f: &Fixture) -> Vec<String> {
    f.runtime
        .owned
        .session_store
        .get_session(&f.room)
        .unwrap()
        .active_interactions()
        .iter()
        .filter(|card| {
            card.custom_choice().is_some_and(|c| {
                c.input_kind() == crate::session::RuntimeInteractionInputKind::Secret
            })
        })
        .map(|card| card.id().to_string())
        .collect()
}

fn inbox_text(f: &Fixture) -> String {
    serde_json::to_string(&f.store().agent_inbox(&f.room, &f.agent, 0).unwrap()
        .into_iter()
        .map(|event| event.payload)
        .collect::<Vec<_>>())
    .unwrap()
}

async fn answer_secret(f: &Fixture, id: &str, choice: &str, reply: Option<&str>) {
    answer_secret_result(f, id, choice, reply).await.unwrap();
}

async fn answer_secret_result(
    f: &Fixture,
    id: &str,
    choice: &str,
    reply: Option<&str>,
) -> Result<crate::local::LocalDaemonResponse, DaemonError> {
    use crate::runtime::command::{KernelCaller, KernelCommand, KernelCommandSource};
    let router = crate::runtime::router::CommandRouter::with_interactive_capacity(f.app.clone(), 8);
    let mut caller = KernelCaller::for_source(&KernelCommandSource::LocalCli)
        .with_connection_class(crate::local::KernelConnectionClass::Terminal);
    caller.caller_id = "owner-web".into();
    caller.client_id = Some("owner-web".into());
    caller.user_id = Some("local".into());
    let request = crate::local::LocalDaemonRequest::RespondToInteraction(
        crate::local::RespondToInteractionRequest {
            session_id: f.room.clone(),
            interaction_id: id.into(),
            choice_id: choice.into(),
            custom_reply: reply.map(str::to_string),
            passkey: None,
            passkey_remember_minutes: None,
        },
    );
    let command = KernelCommand::from_local_request_with_caller(
        "secret-answer",
        KernelCommandSource::LocalCli,
        caller,
        None,
        None,
        &request,
    );
    router.dispatch(command, request).await
}

/// Runs the body on a thread with the kernel runtime's stack: the credential tool future is deep in a debug build.
fn on_kernel_stack<F, B>(body: B)
where
    F: std::future::Future<Output = ()>,
    B: FnOnce() -> F + Send + 'static,
{
    std::thread::Builder::new()
        .stack_size(crate::runtime_transport::KERNEL_RUNTIME_THREAD_STACK_SIZE)
        .spawn(move || {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap()
                .block_on(body())
        })
        .unwrap()
        .join()
        .unwrap();
}

/// C1 + C2: asking twice for the same secret raises one card; each ask returns its ids at once.
#[test]
fn asking_twice_for_the_same_secret_raises_one_card() {
    on_kernel_stack(|| async {
        let f = fixture("secret-card-ask-twice", true).await;
        let _home = Home::new(&f, "twice");
        f.begin(&f.prompt_id);
        let first = ask_secret(&f, "cf-mailbox-token").await.unwrap();
        let retry = ask_secret(&f, "cf-mailbox-token").await.unwrap();
        assert_eq!(first["status"], "pending");
        assert!(first["registration_id"].as_str().is_some());
        assert_eq!(retry["interaction_id"], first["interaction_id"]);
        assert_eq!(retry["registration_id"], first["registration_id"]);
        assert_eq!(retry["deduplicated"], true);
        assert_eq!(secret_cards(&f).len(), 1, "one card for one secret");
        // Another secret is another card.
        let other = ask_secret(&f, "cf-other-token").await.unwrap();
        assert_ne!(other["interaction_id"], first["interaction_id"]);
        assert_eq!(secret_cards(&f).len(), 2);
    });
}

/// C2 + C6: the person's answer is stored by the kernel; the agent reads that it was stored, never the value.
#[test]
fn the_answer_is_stored_in_the_vault_and_the_agent_never_reads_it() {
    on_kernel_stack(|| async {
        let f = fixture("secret-card-answer", true).await;
        let _home = Home::new(&f, "answer");
        f.begin(&f.prompt_id);
        let asked = ask_secret(&f, "cf-mailbox-token").await.unwrap();
        let id = asked["interaction_id"].as_str().unwrap().to_string();
        answer_secret(&f, &id, "secret", Some(CANARY)).await;
        assert!(secret_cards(&f).is_empty(), "the answered card closes");
        let answers: Vec<_> = f
            .store()
            .agent_inbox(&f.room, &f.agent, 0)
            .unwrap()
            .into_iter()
            .filter(|event| event.source_id == id)
            .map(|event| event.payload["public_answer"].clone())
            .collect();
        assert_eq!(answers.len(), 1);
        assert_eq!(answers[0]["status"], "answered");
        assert_eq!(answers[0]["stored"], true);
        assert_eq!(answers[0]["credential_id"], "cf-mailbox-token");
        assert!(answers[0]["reply"].is_null());
        assert!(
            !inbox_text(&f).contains(CANARY),
            "the secret is in no event the agent can read"
        );
        assert!(crate::credential::CharioxCredentialRegistry::user()
            .unwrap()
            .get("cf-mailbox-token")
            .unwrap()
            .is_some());
        // The entry is settled: asking again is a new card.
        let again = ask_secret(&f, "cf-mailbox-token").await.unwrap();
        assert_ne!(again["interaction_id"], asked["interaction_id"]);
    });
}

/// C3: the agent withdraws its own open secret request by its wake id; the card closes for every client.
#[test]
fn the_agent_withdraws_its_open_secret_request_and_the_card_closes() {
    on_kernel_stack(|| async {
        let f = fixture("secret-card-withdraw", true).await;
        let _home = Home::new(&f, "withdraw");
        let task = f.begin(&f.prompt_id);
        let asked = ask_secret(&f, "cf-mailbox-token").await.unwrap();
        let id = asked["interaction_id"].as_str().unwrap().to_string();
        let token = f
            .runtime
            .owned
            .provider_store
            .get_run(&f.run)
            .unwrap()
            .runtime_mcp_auth_token()
            .unwrap()
            .to_owned();
        let withdrawn = call(
            &f,
            &token,
            "chariox.events.cancel_wake",
            &task,
            serde_json::json!({"wake_id": id}),
        )
        .await
        .unwrap()
        .payload;
        assert_eq!(withdrawn["settled"], true);
        assert!(secret_cards(&f).is_empty(), "the card is gone for every client");
        // Nothing can answer it any more, and the secret goes nowhere.
        assert!(!inbox_text(&f).contains(CANARY));
        let retry = ask_secret(&f, "cf-mailbox-token").await.unwrap();
        assert_ne!(retry["interaction_id"], asked["interaction_id"]);
        assert_eq!(secret_cards(&f).len(), 1);
    });
}

/// C4 + C5: no deadline by default (a 7-minute answer still works), and a kernel reload that loses the
/// process-local waiter keeps the card and its entry, so the answer still stores.
#[test]
fn a_slow_answer_after_a_reload_still_reaches_the_vault() {
    on_kernel_stack(|| async {
        let f = fixture("secret-card-reload", true).await;
        let _home = Home::new(&f, "reload");
        let task = f.begin(&f.prompt_id);
        let asked = ask_secret(&f, "cf-mailbox-token").await.unwrap();
        let id = asked["interaction_id"].as_str().unwrap().to_string();
        let wake = f.store().agent_wake(&id).unwrap().unwrap();
        assert!(wake.next_due_ms.is_none(), "an ask has no deadline unless the agent set one");
        f.wait_on(&task, &wake.registration_id);
        // Lose only process-local responders, as a restart does.
        f.runtime.owned.pending_interactions.write().remove(&id);
        let driver = f
            .runtime
            .owned
            .agent_wakes
            .drivers
            .get("interaction")
            .unwrap();
        assert_eq!(driver.recover(&wake), Recovery::Reattached);
        // Seven minutes on, the card is still open.
        f.runtime
            .tick_interaction_wakes(crate::session::unix_epoch_ms() + 7 * 60 * 1000)
            .unwrap();
        assert_eq!(secret_cards(&f), vec![id.clone()]);
        answer_secret(&f, &id, "secret", Some(CANARY)).await;
        assert!(secret_cards(&f).is_empty());
        assert!(crate::credential::CharioxCredentialRegistry::user()
            .unwrap()
            .get("cf-mailbox-token")
            .unwrap()
            .is_some());
        assert!(!inbox_text(&f).contains(CANARY));
    });
}

/// Review P1: a blocking ask (no running turn) is stored once, by its own consumer, with the default overwrite.
#[test]
fn a_blocking_ask_is_stored_once_with_the_default_overwrite() {
    on_kernel_stack(|| async {
        let f = fixture("secret-card-blocking", false).await;
        let _home = Home::new(&f, "blocking");
        let token = f
            .runtime
            .owned
            .provider_store
            .get_run(&f.run)
            .unwrap()
            .runtime_mcp_auth_token()
            .unwrap()
            .to_owned();
        let ask = f.runtime.dispatch_authenticated_runtime_tool_call(
            &token,
            crate::transport::runtime_tools::REQUEST_CREDENTIAL_SECRET_TOOL,
            secret_request("cf-blocking-token"),
        );
        let answer = async {
            let id = loop {
                if let Some(id) = secret_cards(&f).first() {
                    break id.clone();
                }
                tokio::time::sleep(Duration::from_millis(50)).await;
            };
            answer_secret(&f, &id, "secret", Some(CANARY)).await;
        };
        let (result, ()) = tokio::time::timeout(Duration::from_secs(10), async { tokio::join!(ask, answer) })
            .await
            .expect("the blocking ask ends once the person answers");
        let payload = result.expect("a successful answer is not an error").payload;
        assert_eq!(payload["status"], "stored");
        assert!(crate::credential::CharioxCredentialRegistry::user()
            .unwrap()
            .get("cf-blocking-token")
            .unwrap()
            .is_some());
        assert!(!inbox_text(&f).contains(CANARY));
    });
}

/// Review P2: an answer after the deadline is refused before the Vault is touched.
#[test]
fn an_answer_after_the_deadline_leaves_the_vault_unchanged() {
    on_kernel_stack(|| async {
        let f = fixture("secret-card-expired", true).await;
        let _home = Home::new(&f, "expired");
        f.begin(&f.prompt_id);
        let asked = ask_secret_with(&f, secret_request_with("cf-expired-token", Some(1)))
            .await
            .unwrap();
        let id = asked["interaction_id"].as_str().unwrap().to_string();
        tokio::time::sleep(Duration::from_millis(1300)).await;
        assert!(answer_secret_result(&f, &id, "secret", Some(CANARY)).await.is_err());
        assert!(crate::credential::CharioxCredentialRegistry::user()
            .unwrap()
            .get("cf-expired-token")
            .unwrap()
            .is_none());
        assert!(!inbox_text(&f).contains(CANARY));
    });
}

/// Review P2: while an answer is being stored, a timeout, a cancel and a withdraw leave the card alone,
/// a second answer is refused, and the card is released afterwards.
#[test]
fn a_store_in_progress_holds_the_card_against_timeout_cancel_and_withdraw() {
    on_kernel_stack(|| async {
        let f = fixture("secret-card-reserved", true).await;
        let _home = Home::new(&f, "reserved");
        let task = f.begin(&f.prompt_id);
        let asked = ask_secret(&f, "cf-reserved-token").await.unwrap();
        let id = asked["interaction_id"].as_str().unwrap().to_string();
        let token = f
            .runtime
            .owned
            .provider_store
            .get_run(&f.run)
            .unwrap()
            .runtime_mcp_auth_token()
            .unwrap()
            .to_owned();
        let reservation = f.runtime.owned.reserve_secret_store(&f.room, &id).unwrap();
        assert!(f.runtime.owned.reserve_secret_store(&f.room, &id).is_err());
        f.runtime.owned.timeout_runtime_interaction(&f.room, &id).unwrap();
        assert!(answer_secret_result(&f, &id, "cancel", None).await.is_err());
        let withdraw = call(
            &f,
            &token,
            "chariox.events.cancel_wake",
            &task,
            serde_json::json!({"wake_id": id}),
        )
        .await;
        assert!(withdraw.is_err(), "a withdraw waits for the store");
        assert_eq!(secret_cards(&f), vec![id.clone()], "the card is still open");
        drop(reservation);
        let withdrawn = call(
            &f,
            &token,
            "chariox.events.cancel_wake",
            &task,
            serde_json::json!({"wake_id": id}),
        )
        .await
        .unwrap()
        .payload;
        assert_eq!(withdrawn["settled"], true);
        assert!(secret_cards(&f).is_empty());
    });
}
