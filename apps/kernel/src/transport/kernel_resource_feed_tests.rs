use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use serde_json::{json, Value};
use tokio::sync::{mpsc, Mutex};

use super::*;
use crate::local::{KernelResourceWatch, UpsertCredentialRequest};
use crate::runtime::command::{KernelCaller, KernelCommand, KernelCommandSource};

/// A kernel with its own CHARIOX_HOME, so credentials land in a scratch directory.
struct Kernel {
    router: Arc<CommandRouter>,
    root: PathBuf,
    previous_home: Option<std::ffi::OsString>,
    _environment: crate::env_lock::EnvGuard,
}

impl Kernel {
    fn new() -> Self {
        let environment = crate::env_lock::lock();
        let root = std::env::temp_dir().join(format!(
            "chariox-resource-feed-{}-{}",
            std::process::id(),
            crate::session::unix_epoch_ms()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let previous_home = std::env::var_os("CHARIOX_HOME");
        std::env::set_var("CHARIOX_HOME", root.join("home"));
        let app = crate::DaemonApp::bootstrap(crate::DaemonConfig::for_tests()).unwrap();
        let router = Arc::new(CommandRouter::with_interactive_capacity(
            Arc::new(Mutex::new(app)),
            8,
        ));
        Self {
            router,
            root,
            previous_home,
            _environment: environment,
        }
    }

    fn feed(
        &self,
    ) -> (
        KernelResourceWatchSender,
        mpsc::UnboundedReceiver<KernelEvent>,
    ) {
        let (inbox, updates) = watch_inbox();
        let (events_tx, events) = mpsc::unbounded_channel();
        tokio::spawn(run_kernel_resource_feed(
            Arc::clone(&self.router),
            KernelCommandSource::RelayClient,
            updates,
            Duration::from_secs(60),
            move |event| {
                let sent = events_tx.send(event).is_ok();
                async move { sent }
            },
        ));
        (inbox, events)
    }

    async fn upsert_credential(&self, id: &str) {
        let request = LocalDaemonRequest::UpsertCredential(UpsertCredentialRequest {
            credential: crate::config::UserCredentialConfig {
                id: id.into(),
                description: None,
                source: crate::config::UserCredentialSourceConfig::Vault { key: id.into() },
                allowed_hosts: vec!["fixture.test".into()],
                allowed_uses: vec![crate::config::UserCredentialUse::Browser],
                injection: crate::config::UserCredentialInjectionConfig::Browser,
                metadata: None,
            },
        });
        // The owner changes the Vault from a terminal of their own.
        let terminal = self
            .router
            .local_terminal_caller(KernelCommandSource::LocalCli, "owner-terminal")
            .await;
        let command = KernelCommand::from_local_request_with_caller(
            format!("upsert-{id}"),
            KernelCommandSource::LocalCli,
            terminal,
            None,
            None,
            &request,
        );
        self.router.dispatch(command, request).await.unwrap();
    }
}

impl Drop for Kernel {
    fn drop(&mut self) {
        match &self.previous_home {
            Some(previous) => std::env::set_var("CHARIOX_HOME", previous),
            None => std::env::remove_var("CHARIOX_HOME"),
        }
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

/// A web client of `user_id`, signed in through the relay.
fn relay_client(user_id: &str) -> KernelCaller {
    KernelCaller::for_relay_request(Some(chariox_relay::protocol::RelayCallerIdentity {
        realm_id: "test".into(),
        subject: format!("client-{user_id}"),
        subject_kind: chariox_relay::auth::RelaySubjectKind::Client,
        expires_at_ms: u64::MAX,
        token_id: None,
        user_id: Some(user_id.into()),
        public_key_thumbprint: None,
    }))
}

fn owner() -> KernelCaller {
    relay_client(crate::session::DEFAULT_LOCAL_USER_ID)
}

fn collaborator() -> KernelCaller {
    relay_client("member")
}

fn watch(resource: &str, known_revision: Option<&str>) -> KernelResourceWatch {
    KernelResourceWatch {
        resource: resource.into(),
        params: json!({}),
        known_revision: known_revision.map(str::to_owned),
    }
}

async fn next_change(events: &mut mpsc::UnboundedReceiver<KernelEvent>) -> (String, Value) {
    loop {
        let event = tokio::time::timeout(Duration::from_secs(10), events.recv())
            .await
            .expect("a kernel_resource_changed event within 10 s")
            .expect("the feed is running");
        if let KernelEvent::KernelResourceChanged { revision, body, .. } = event {
            return (revision, serde_json::to_value(body).unwrap());
        }
    }
}

fn credential_ids(body: &Value) -> Vec<String> {
    body["snapshot"]["data"]["credentials"]
        .as_array()
        .expect("a snapshot of the credential list")
        .iter()
        .map(|credential| credential["id"].as_str().unwrap().to_owned())
        .collect()
}

#[tokio::test]
async fn a_watched_resource_is_answered_then_pushed_on_change() {
    let kernel = Kernel::new();
    let (inbox, mut events) = kernel.feed();
    let refused = submit_watch_set(&inbox, vec![watch("vault_entries", None)], owner())
        .await
        .unwrap();
    assert!(refused.is_empty(), "{refused:?}");
    let (first, body) = next_change(&mut events).await;
    assert!(credential_ids(&body).is_empty());

    kernel.upsert_credential("login").await;
    let (second, body) = next_change(&mut events).await;
    assert_ne!(first, second, "a change carries a new revision");
    assert_eq!(credential_ids(&body), ["login"]);
}

#[tokio::test]
async fn the_revision_a_client_holds_is_answered_unchanged() {
    let kernel = Kernel::new();
    kernel.upsert_credential("login").await;
    let (inbox, mut events) = kernel.feed();
    submit_watch_set(&inbox, vec![watch("vault_entries", None)], owner())
        .await
        .unwrap();
    let (revision, _) = next_change(&mut events).await;

    // A reload: a new subscription, holding that revision.
    let (inbox, mut events) = kernel.feed();
    submit_watch_set(
        &inbox,
        vec![watch("vault_entries", Some(&revision))],
        owner(),
    )
    .await
    .unwrap();
    let (answered, body) = next_change(&mut events).await;
    assert_eq!(answered, revision);
    assert_eq!(body, json!({ "unchanged": {} }));

    // A copy the kernel no longer has gets the snapshot.
    let (inbox, mut events) = kernel.feed();
    submit_watch_set(&inbox, vec![watch("vault_entries", Some("stale"))], owner())
        .await
        .unwrap();
    let (_, body) = next_change(&mut events).await;
    assert_eq!(credential_ids(&body), ["login"]);
}

#[tokio::test]
async fn a_watch_set_keeps_answered_keys_and_answers_new_ones() {
    let kernel = Kernel::new();
    let (inbox, mut events) = kernel.feed();
    submit_watch_set(&inbox, vec![watch("vault_entries", None)], owner())
        .await
        .unwrap();
    let (revision, _) = next_change(&mut events).await;
    submit_watch_set(
        &inbox,
        vec![
            watch("vault_entries", Some(&revision)),
            watch("vault_status", None),
        ],
        owner(),
    )
    .await
    .unwrap();
    let (resource, body) = loop {
        let event = tokio::time::timeout(Duration::from_secs(10), events.recv())
            .await
            .unwrap()
            .unwrap();
        if let KernelEvent::KernelResourceChanged { resource, body, .. } = event {
            break (resource, body);
        }
    };
    assert_eq!(resource, "vault_status");
    let body = serde_json::to_value(body).unwrap();
    assert_eq!(body["snapshot"]["data"]["status"]["unlocked"], json!(false));
    // Heartbeats keep arriving; no resource is answered again.
    let answered_again = tokio::time::timeout(Duration::from_millis(300), async {
        while let Some(event) = events.recv().await {
            if let KernelEvent::KernelResourceChanged { resource, .. } = event {
                return resource;
            }
        }
        std::future::pending().await
    })
    .await;
    assert!(
        answered_again.is_err(),
        "the kept vault_entries watch is not answered again: {answered_again:?}"
    );
}

#[tokio::test]
async fn refused_watches_are_listed_and_never_answered_empty() {
    let kernel = Kernel::new();
    let (inbox, mut events) = kernel.feed();
    let refused = submit_watch_set(
        &inbox,
        vec![
            watch("room_list_v9", None),
            KernelResourceWatch {
                resource: "vault_entries".into(),
                params: json!({ "session_id": "room" }),
                known_revision: None,
            },
        ],
        owner(),
    )
    .await
    .unwrap();
    let codes = refused
        .iter()
        .map(|refusal| (refusal.resource.as_str(), refusal.code.as_str()))
        .collect::<Vec<_>>();
    assert_eq!(
        codes,
        [
            ("room_list_v9", "unknown_resource"),
            ("vault_entries", "invalid_params")
        ]
    );

    // A Room collaborator is refused the owner's Vault, as ListCredentials refuses it.
    let refused = submit_watch_set(
        &inbox,
        vec![watch("vault_entries", None), watch("vault_status", None)],
        collaborator(),
    )
    .await
    .unwrap();
    assert_eq!(refused.len(), 2, "{refused:?}");
    assert!(
        tokio::time::timeout(Duration::from_millis(300), async {
            loop {
                if let Some(KernelEvent::KernelResourceChanged { .. }) = events.recv().await {
                    return;
                }
            }
        })
        .await
        .is_err(),
        "a refused watch gets no snapshot"
    );
}

#[test]
fn an_unlocked_vault_is_read_again_when_its_lease_ends() {
    let ends_at_ms = crate::session::unix_epoch_ms() + 2_000;
    let unlocked = json!({ "status": { "unlocked": true, "expires_at_ms": ends_at_ms } });
    assert_eq!(lease_end(&unlocked), Some(ends_at_ms));
    let locked = json!({ "status": { "unlocked": false, "expires_at_ms": ends_at_ms } });
    assert_eq!(lease_end(&locked), None);
    assert_eq!(lease_end(&json!({ "credentials": [] })), None);
}

#[test]
fn only_resource_changing_commands_signal_the_feed() {
    for changing in [
        "credential.upsert",
        "credential_vault.unlock",
        "credential_vault.lock",
        "provider_account.rename",
        "provider.logout",
        "app.uninstall",
    ] {
        assert!(may_change_resources(changing), "{changing}");
    }
    for unrelated in [
        "credential.list",
        "credential_vault.status",
        "provider_account.list",
        "kernel_browser",
        "prompt.submit",
        "app.user_view.call",
    ] {
        assert!(!may_change_resources(unrelated), "{unrelated}");
    }
}

#[test]
fn an_account_check_alone_does_not_change_the_accounts_revision() {
    let checked_at = |at: u64| {
        let mut accounts = serde_json::json!({ "profiles": [{ "provider": "codex", "profile_id": "work", "auth_state": "authenticated", "last_validated_at_ms": at }] });
        without_validation_times(&mut accounts);
        accounts
    };
    assert_eq!(revision_of(&checked_at(1)), revision_of(&checked_at(2)));
    assert!(checked_at(1)["profiles"][0]
        .get("last_validated_at_ms")
        .is_none());
    let mut signed_out = checked_at(1);
    signed_out["profiles"][0]["auth_state"] = "not_configured".into();
    assert_ne!(
        revision_of(&checked_at(1)),
        revision_of(&signed_out),
        "what a client draws still changes the revision"
    );
}

#[tokio::test]
async fn file_changes_are_native_bounded_and_resync_after_resubscription() {
    let kernel = Kernel::new();
    let dir = kernel.root.join("visible");
    std::fs::create_dir(&dir).unwrap();
    let params = json!({"paths":[dir.display().to_string()]});
    let interest = KernelResourceWatch {
        resource: "file_changes".into(),
        params: params.clone(),
        known_revision: None,
    };
    let (inbox, mut events) = kernel.feed();
    assert!(submit_watch_set(&inbox, vec![interest.clone()], owner())
        .await
        .unwrap()
        .is_empty());
    let (first, _) = next_change(&mut events).await;
    for i in 0..20 {
        std::fs::write(dir.join(format!("{i}.txt")), "native").unwrap();
    }
    let (second, _) = tokio::time::timeout(Duration::from_millis(250), next_change(&mut events))
        .await
        .unwrap();
    assert_ne!(first, second);
    // A complete interest replacement registers first, then sends a baseline even
    // when the client remembers the old epoch: reconnect cannot miss a gap.
    let interest = KernelResourceWatch {
        known_revision: Some(second.clone()),
        ..interest
    };
    assert!(submit_watch_set(&inbox, vec![interest], owner())
        .await
        .unwrap()
        .is_empty());
    let (resumed, _) = next_change(&mut events).await;
    assert_ne!(second, resumed);
    assert!(submit_watch_set(&inbox, vec![], owner())
        .await
        .unwrap()
        .is_empty());
    while events.try_recv().is_ok() {}
    std::fs::write(dir.join("closed"), "no subscriber").unwrap();
    assert!(
        tokio::time::timeout(Duration::from_millis(80), next_change(&mut events))
            .await
            .is_err()
    );
    let too_many = KernelResourceWatch {
        resource: "file_changes".into(),
        params: json!({"paths": vec![dir.display().to_string(); 65]}),
        known_revision: None,
    };
    assert_eq!(
        submit_watch_set(&inbox, vec![too_many], owner())
            .await
            .unwrap()
            .len(),
        1
    );
    let denied = KernelResourceWatch {
        resource: "file_changes".into(),
        params,
        known_revision: None,
    };
    assert_eq!(
        submit_watch_set(&inbox, vec![denied], collaborator())
            .await
            .unwrap()
            .len(),
        1
    );
    drop(inbox);
    drop(events);
    kernel
        .router
        .runtime_state()
        .shutdown_cleanup()
        .await
        .unwrap();
}
