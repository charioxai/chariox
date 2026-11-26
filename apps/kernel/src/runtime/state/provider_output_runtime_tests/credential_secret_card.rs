//! Row 539: an agent asking a person for a secret raises ONE card, gets its ids at once, and never
//! reads the secret.
use super::*;

const CANARY: &str = "canary-secret-7f3a91c2e4b8d5a6";

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
            secret_request(entry),
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
    router.dispatch(command, request).await.unwrap();
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
