//! MP-08 / MP-09 / MP-10 / MP-11: ordinary Vault generation and protected login through
//! the real MCP router. The browser is a fixture controller that records what
//! the kernel physically inserted; live Chromium/provider acceptance is separate.
use super::tests::{fixture, fixture_with_options, running, Fixture, PASSKEY};
use super::*;
use crate::config::{UserCredentialConfig, UserCredentialMetadataConfig};
use serde_json::{json, Value};

// The public tool name, pinned literally so these tests also run on the base.
const VAULT_GENERATE: &str = "chariox.vault.generate";
const TOKEN: &str = "sudo-fixture-bearer";
const PASTE: &str = "chariox.kernel_browser_paste_secret";
const SITE: &str = "http://127.0.0.1:8123";

pub(super) fn names(f: &Fixture) -> Vec<String> {
    f.router
        .runtime_tool_specs_for_auth_token(TOKEN)
        .into_iter()
        .map(|spec| spec.name)
        .collect()
}

pub(super) async fn call(f: &Fixture, tool: &str, arguments: Value) -> Result<Value, DaemonError> {
    Box::pin(
        f.router
            .dispatch_authenticated_runtime_tool_call(TOKEN, tool, arguments),
    )
    .await
    .map(|result| result.payload)
}

fn vault_path(f: &Fixture) -> String {
    f.state
        .owned
        .config_projection
        .snapshot()
        .user_config
        .credential_vault
        .path
}

pub(super) fn unlock(f: &Fixture) {
    crate::secret::unlock_chariox_encrypted_vault(
        vault_path(f),
        PASSKEY,
        crate::secret::VaultUnlockLease::KernelShutdown,
    )
    .unwrap();
}

fn stored(f: &Fixture, handle: &str) -> zeroize::Zeroizing<String> {
    zeroize::Zeroizing::new(
        f.state
            .home_runtime_secret_service()
            .unwrap()
            .browser_secret_input_for_target_url(handle, &format!("{SITE}/login"))
            .unwrap(),
    )
}

/// A live sudo window whose turn is actually running, as during provider work.
pub(super) fn elevated(f: &Fixture) -> KernelSudoTurn {
    let turn = running(f);
    let prompts = &f.state.owned.prompt_state_owner;
    let session = f
        .state
        .owned
        .session_store
        .get_session(&turn.session_id)
        .unwrap();
    let active = prompts
        .active_prompt_for_agent_snapshot(&session, &turn.agent_id)
        .unwrap();
    let mut started = active.clone();
    started.set_status(crate::session::PromptStatus::Running);
    assert!(prompts.replace_active_prompt_if_matches(&session, &turn.agent_id, &active, started));
    turn
}

async fn generate(f: &Fixture, request_id: &str, origin: &str) -> Result<Value, DaemonError> {
    call(
        f,
        VAULT_GENERATE,
        json!({"request_id": request_id, "origin": origin}),
    )
    .await
}

// MP-08 / MP-09 / MP-10 / MP-11: owner-client "once" covers an agent operation too.
async fn client_unlock_once(f: &Fixture) {
    use crate::local::{
        LocalDaemonRequest, UnlockCredentialVaultRequest, VaultPassphrase, VaultUnlockDuration,
        VaultUnlockSurface,
    };
    use crate::runtime::command::{KernelCaller, KernelCommand, KernelCommandSource};
    let request = LocalDaemonRequest::UnlockCredentialVault(UnlockCredentialVaultRequest {
        passphrase: VaultPassphrase::new(PASSKEY),
        duration: VaultUnlockDuration::Once,
        surface: VaultUnlockSurface::VaultWindow,
    });
    let caller = KernelCaller::for_source(&KernelCommandSource::LocalCli)
        .with_connection_class(crate::local::KernelConnectionClass::Terminal);
    let command = KernelCommand::from_local_request_with_caller(
        "owner-once",
        KernelCommandSource::LocalCli,
        caller,
        None,
        None,
        &request,
    );
    f.router.dispatch(command, request).await.unwrap();
    assert!(crate::secret::chariox_vault_single_use_unlocked(vault_path(f)).unwrap());
}

fn assert_vault_locked(f: &Fixture) {
    assert!(
        !crate::secret::chariox_encrypted_vault_status(vault_path(f))
            .unwrap()
            .unlocked,
        "the agent operation must consume the owner's single-use unlock",
    );
}

#[tokio::test]
async fn single_use_owner_unlock_is_consumed_by_agent_generation() {
    crate::test_support::isolated_env_test!();
    let f = fixture();
    client_unlock_once(&f).await;
    assert_eq!(
        generate(&f, "first-once", SITE).await.unwrap()["created"],
        true
    );
    assert_vault_locked(&f);
    // A new request must ask again; an idempotent retry is not another write.
    let next = generate(&f, "second-once", SITE);
    tokio::pin!(next);
    tokio::select! {
        result = &mut next => panic!("a second generation bypassed the unlock: {result:?}"),
        () = async {
            owner_unlocks(&f, &f.request.session_id, || {}).await;
        } => {}
    }
    assert_eq!(next.await.unwrap()["created"], true);
    f.state.shutdown_cleanup().await.unwrap();
}

#[tokio::test]
async fn single_use_owner_notification_is_consumed_by_waiting_agent_generation() {
    crate::test_support::isolated_env_test!();
    let f = fixture();
    let generate = generate(&f, "notified-once", SITE);
    let owner = async {
        tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                if !f
                    .state
                    .owned
                    .session_snapshot(&f.request.session_id)
                    .unwrap()
                    .active_interactions()
                    .is_empty()
                {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        client_unlock_once(&f).await;
    };
    let (created, ()) = tokio::join!(generate, owner);
    assert_eq!(created.unwrap()["created"], true);
    assert_vault_locked(&f);
    assert!(f
        .state
        .owned
        .session_snapshot(&f.request.session_id)
        .unwrap()
        .active_interactions()
        .is_empty());
    f.state.shutdown_cleanup().await.unwrap();
}

#[tokio::test]
async fn single_use_owner_unlock_is_consumed_by_agent_browser_fill() {
    crate::test_support::isolated_env_test!();
    let login = login_room().await;
    let f = &login.f;
    unlock(f);
    let handle = generate(f, "fill-once", SITE).await.unwrap()["credential_id"]
        .as_str()
        .unwrap()
        .to_owned();
    for tool in [PASTE, FILL] {
        crate::secret::lock_chariox_encrypted_vault(vault_path(f)).unwrap();
        client_unlock_once(f).await;
        assert_eq!(
            call(f, tool, paste_args(&handle)).await.unwrap(),
            json!({"inserted": true})
        );
        assert!(inserted(&login).is_some());
        assert_vault_locked(f);
    }
    f.state.shutdown_cleanup().await.unwrap();
}

#[tokio::test]
async fn single_use_owner_unlock_covers_browser_fill_with_signing_key_preparation() {
    crate::test_support::isolated_env_test!();
    let login = login_room().await;
    let f = &login.f;
    unlock(f);
    let handle = generate(f, "signed-fill-once", SITE).await.unwrap()["credential_id"]
        .as_str()
        .unwrap()
        .to_owned();
    let mut config = f.state.owned.config_projection.snapshot();
    let session = f
        .state
        .owned
        .session_store
        .get_session(&f.request.session_id)
        .unwrap();
    config.user_config.web_bot_auth.projects = vec![session.project_id().to_owned()];
    config.user_config.web_bot_auth.signature_agent = Some("https://signer.example.com".into());
    f.state.owned.config_projection.update(config);
    crate::secret::lock_chariox_encrypted_vault(vault_path(f)).unwrap();
    client_unlock_once(f).await;
    assert_eq!(
        call(f, PASTE, paste_args(&handle)).await.unwrap(),
        json!({"inserted": true})
    );
    assert!(inserted(&login).is_some());
    assert_vault_locked(f);
    f.state.shutdown_cleanup().await.unwrap();
}

// MP-08 / MP-09 / MP-10 / MP-11: ordinary Vault creation needs no elevation.
#[tokio::test]
async fn vopen_plain_agent_generates_for_one_origin_without_a_prompt() {
    crate::test_support::isolated_env_test!();
    let f = fixture();
    unlock(&f);
    assert!(f.state.owned.sudo_turns.lock().unwrap().is_empty());
    let created = generate(&f, "ordinary-signup", SITE).await.unwrap();
    let handle = created["credential_id"].as_str().unwrap();
    assert_eq!(created["origin"], SITE);
    assert_eq!(created["created"], true);
    let value = stored(&f, handle);
    assert!(!created.to_string().contains(value.as_str()));
    assert_eq!(
        generate(&f, "ordinary-signup", SITE).await.unwrap()["credential_id"],
        handle
    );
    assert!(generate(&f, "ordinary-signup", "https://example.com")
        .await
        .is_err());
    assert!(f
        .state
        .home_runtime_secret_service()
        .unwrap()
        .browser_secret_input_for_target_url(handle, "http://127.0.0.1:9999/login")
        .is_err());
    let snapshot = f
        .state
        .owned
        .session_snapshot(&f.request.session_id)
        .unwrap();
    assert!(snapshot.active_interactions().is_empty());
    assert!(f.state.owned.sudo_turns.lock().unwrap().is_empty());
    assert!(call(
        &f,
        "chariox_kernel_request",
        json!({"request":{"ListSessions":null}})
    )
    .await
    .is_err());
}

#[tokio::test]
async fn vopen_an_enrolled_kernel_owner_generates_and_other_users_cannot() {
    crate::test_support::isolated_env_test!();
    let f = super::tests::fixture_for_owner(None, false, "claude", "claude", None, "cloud-owner");
    unlock(&f);
    assert!(generate(&f, "before-enrollment", SITE).await.is_err());
    let mut config = f.state.owned.config_projection.snapshot();
    config.cloud_relay = Some(crate::config::PersistedCloudRelayProfile {
        user_id: "cloud-owner".into(),
        ..Default::default()
    });
    f.state.owned.config_projection.update(config);
    assert_eq!(generate(&f, "signup", SITE).await.unwrap()["created"], true);
}

#[tokio::test]
async fn a06_elevated_generation_commits_one_handle_and_never_returns_the_value() {
    crate::test_support::isolated_env_test!();
    let f = fixture();
    let turn = elevated(&f);
    unlock(&f);
    assert!(names(&f).iter().any(|name| name == VAULT_GENERATE));
    let created = generate(&f, "signup", &format!("{SITE}/register"))
        .await
        .unwrap();
    let handle = created["credential_id"].as_str().unwrap().to_string();
    assert!(handle.starts_with("gen-"));
    assert_eq!(
        created,
        json!({"credential_id": handle, "origin": SITE, "created": true})
    );
    let secret = stored(&f, &handle);
    assert_eq!(secret.chars().count(), 24);
    let session =
        serde_json::to_string(&f.state.owned.session_snapshot(&turn.session_id).unwrap()).unwrap();
    assert!(!session.contains(secret.as_str()));
    assert!(!created.to_string().contains(secret.as_str()));
    // A lost acknowledgement: the retry returns the same committed value.
    let retried = generate(&f, "signup", SITE).await.unwrap();
    assert_eq!(retried["credential_id"], handle.as_str());
    assert_eq!(retried["created"], false);
    assert_eq!(stored(&f, &handle).as_str(), secret.as_str());
    assert!(
        generate(&f, "signup", "https://example.com").await.is_err(),
        "a request_id never rebinds to another site"
    );
    assert!(f
        .state
        .home_runtime_secret_service()
        .unwrap()
        .browser_secret_input_for_target_url(&handle, "http://127.0.0.1:9999/login")
        .is_err());
    let metadata = crate::credential::CharioxCredentialRegistry::user()
        .unwrap()
        .get(&handle)
        .unwrap()
        .unwrap()
        .metadata
        .unwrap();
    assert_eq!(metadata.created_by_kind.as_deref(), Some("vault_generate"));
    assert_eq!(
        metadata.session_id.as_deref(),
        Some(turn.session_id.as_str())
    );
    assert_eq!(
        metadata.created_by_id.as_deref(),
        Some(turn.agent_id.as_str())
    );
    for refused in ["http://example.com", "https://u:p@example.com"] {
        assert!(generate(&f, "other", refused).await.is_err(), "{refused}");
    }
    f.state
        .revoke_sudo(Some("local"), Some(&turn.entry_id), "explicit_revoke")
        .unwrap();
    assert!(names(&f).iter().any(|name| name == VAULT_GENERATE));
    assert!(
        generate(&f, "after-revoke", SITE).await.is_err(),
        "revoking sudo cancels its originating prompt through the normal lifecycle"
    );
}

#[tokio::test]
async fn a06_a_generated_handle_never_takes_a_slot_that_holds_a_value() {
    crate::test_support::isolated_env_test!();
    let f = fixture();
    let turn = elevated(&f);
    unlock(&f);
    let handle = super::super::vault_generate::generated_handle(
        &turn.owner_user_id,
        &turn.session_id,
        &turn.agent_id,
        "signup",
    );
    f.state
        .home_runtime_secret_service()
        .unwrap()
        .set_vault_secret(&handle, "original-value")
        .unwrap();

    assert!(
        generate(&f, "signup", SITE).await.is_err(),
        "a generated password replaced a stored value"
    );
    let value = f
        .state
        .home_runtime_secret_service()
        .unwrap()
        .vault_secret_for_dictation(&handle)
        .unwrap()
        .unwrap();
    assert_eq!(value.as_str(), "original-value");
}

/// Answers the Vault unlock interactions as the owner, after `during` runs.
/// Returns early when `during` already withdrew the prompt.
pub(super) async fn owner_unlocks(f: &Fixture, session: &str, during: impl FnOnce()) {
    let pending = |prefix: &str| {
        f.state
            .owned
            .session_snapshot(session)
            .unwrap()
            .active_interactions()
            .iter()
            .find(|card| card.id().starts_with(prefix))
            .map(|card| card.id().to_string())
    };
    let first = tokio::time::timeout(std::time::Duration::from_secs(10), async {
        loop {
            if let Some(id) = pending("vault-unlock-") {
                return id;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    during();
    // A caller may withdraw its own unlock request.
    if f.state
        .owned
        .pending_interactions
        .write()
        .get(&first)
        .is_none()
    {
        return;
    }
    f.state
        .resolve_terminal_runtime_interaction(
            session,
            &first,
            "passphrase",
            Some(PASSKEY),
            Some("local"),
        )
        .await
        .unwrap();
}

// MP-08 / MP-09 / MP-10 / MP-11: a plain prompt agent (no workflow turn) manages the Vault through the
// credential dispatcher; the same call used to fall into the workflow resolver ("no active workflow turn").
#[tokio::test]
async fn vflag_plain_agent_reads_vault_status_and_opens_the_popup() {
    crate::test_support::isolated_env_test!();
    let f = fixture();
    for tool in ["chariox.manage_credential_vault", "manage_credential_vault"] {
        let status = call(&f, tool, json!({"action": "status"})).await.unwrap();
        assert_eq!(status["action"], "status");
        assert_eq!(status["status"]["unlocked"], false);
    }
    let popup = call(
        &f,
        "chariox.manage_credential_vault",
        json!({"action": "popup"}),
    );
    let unlock = owner_unlocks(&f, &f.request.session_id, || {});
    let (result, ()) = tokio::join!(popup, unlock);
    assert_eq!(result.unwrap()["action"], "unlocked");
    let status = call(
        &f,
        "chariox.manage_credential_vault",
        json!({"action": "status"}),
    )
    .await
    .unwrap();
    assert_eq!(status["status"]["unlocked"], true);
}

// PCD-05: an unlock through another window answers the waiting agent as unlocked and says where it happened.
#[tokio::test]
async fn an_unlock_through_the_vault_window_tells_the_waiting_agent_where() {
    crate::test_support::isolated_env_test!();
    let f = fixture();
    let popup = call(
        &f,
        "chariox.manage_credential_vault",
        json!({"action": "popup"}),
    );
    let window = async {
        let session = &f.request.session_id;
        tokio::time::timeout(std::time::Duration::from_secs(10), async {
            while !f
                .state
                .owned
                .session_snapshot(session)
                .unwrap()
                .active_interactions()
                .iter()
                .any(|card| card.id().starts_with("vault-unlock-"))
            {
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        f.state
            .unlock_credential_vault_for_owner(
                "local",
                crate::local::UnlockCredentialVaultRequest {
                    passphrase: crate::local::VaultPassphrase::new(PASSKEY),
                    duration: crate::local::VaultUnlockDuration::Minutes { minutes: 30 },
                    surface: crate::local::VaultUnlockSurface::VaultWindow,
                },
            )
            .await
            .unwrap();
    };
    let (result, ()) = tokio::join!(popup, window);
    let result = result.unwrap();
    assert_eq!(result["action"], "unlocked");
    assert_eq!(result["via"], "the Vault window");
}

// MP-08 / MP-09 / MP-10 / MP-11: a provider that gives up on a waiting popup call (its tool timeout)
// must not leave a card nobody can answer.
#[tokio::test]
async fn vflag_popup_card_is_withdrawn_when_its_waiting_call_is_gone() {
    crate::test_support::isolated_env_test!();
    let f = fixture();
    let mut waiting = Box::pin(call(
        &f,
        "chariox.request_popup",
        json!({"message": "Open the page", "choices": [{"id": "done", "label": "Done", "reply": "Done"}]}),
    ));
    tokio::select! {
        result = &mut waiting => panic!("popup resolved by itself: {result:?}"),
        () = tokio::time::sleep(std::time::Duration::from_millis(300)) => {}
    }
    let open = || {
        f.state
            .owned
            .session_snapshot(&f.request.session_id)
            .unwrap()
            .active_interactions()
            .len()
    };
    assert_eq!(open(), 1);
    drop(waiting);
    assert_eq!(open(), 0);
}

// MP-08 / MP-09 / MP-10 / MP-11: the provider never tells a card that its waiting call was cut, so a blocking
// card ends itself before the provider's 300 s cut; only wait:false cards may stay open longer.
#[tokio::test]
async fn vflag_blocking_popup_card_ends_before_the_provider_cut() {
    crate::test_support::isolated_env_test!();
    for action in [false, true] {
        for (asked, expected) in [(None, 280), (Some(900), 280), (Some(30), 30)] {
            let f = fixture();
            let mut arguments = json!({"message": "Open the page", "choices": [{"id": "done", "label": "Done", "reply": "Done"}]});
            if action {
                arguments["action"] = json!({"request": {"AliasSession": {"session_id": f.request.session_id, "alias": "after-approval"}}});
            }
            if let Some(seconds) = asked {
                arguments["timeout_sec"] = json!(seconds);
            }
            let mut waiting = Box::pin(call(&f, "chariox.request_popup", arguments));
            tokio::select! {
                result = &mut waiting => panic!("popup resolved by itself: {result:?}"),
                () = tokio::time::sleep(std::time::Duration::from_millis(300)) => {}
            }
            let snapshot = f
                .state
                .owned
                .session_snapshot(&f.request.session_id)
                .unwrap();
            let cards = snapshot.active_interactions();
            assert_eq!(cards.len(), 1);
            assert_eq!(
                cards[0].timeout_sec(),
                Some(expected),
                "action {action}, asked {asked:?}"
            );
            tokio::time::pause();
            tokio::time::advance(std::time::Duration::from_secs(expected)).await;
            let result = tokio::time::timeout(std::time::Duration::from_secs(1), &mut waiting)
                .await
                .expect("card settles before provider timeout")
                .unwrap();
            assert_eq!(result["status"], "timed_out");
            if action {
                assert_eq!(result["ran"], false);
            }
            assert!(f
                .state
                .owned
                .session_snapshot(&f.request.session_id)
                .unwrap()
                .active_interactions()
                .is_empty());
            tokio::time::resume();
        }
    }
}

#[tokio::test]
async fn vflag_unknown_runtime_tool_is_not_a_workflow_error() {
    crate::test_support::isolated_env_test!();
    let f = fixture();
    let error = call(&f, "chariox.no_such_tool", json!({}))
        .await
        .unwrap_err()
        .to_string();
    assert!(
        error.contains("unknown runtime tool `chariox.no_such_tool`"),
        "{error}"
    );
    assert!(!error.contains("workflow turn"), "{error}");
}

#[tokio::test]
async fn vopen_generation_survives_sudo_expiry_during_vault_unlock() {
    crate::test_support::isolated_env_test!();
    let f = fixture();
    let turn = elevated(&f);
    let pending = generate(&f, "signup", SITE);
    let unlock = owner_unlocks(&f, &turn.session_id, || {
        f.state
            .owned
            .sudo_turns
            .lock()
            .unwrap()
            .get_mut(&turn.entry_id)
            .unwrap()
            .deadline = Some(std::time::Instant::now());
    });
    let (result, ()) = tokio::join!(pending, unlock);
    assert!(
        result.is_ok(),
        "sudo expiry does not gate ordinary Vault generation: {result:?}"
    );
    assert_eq!(crate::credential::load_user_credentials().unwrap().len(), 1);
}

const LOGIN_CONTROLLER: &str = r#"set -eu
while IFS= read -r request; do
 id=${request#*:}; id=${id%%,*}
 tabs='{"tab_id":"host-tab-fixture","document_id":"document"}'
 [ -f "$1/opened" ] && tabs="$tabs"',{"tab_id":"host-tab-new","document_id":"doc-login"}'
 case "$request" in
  *'"method":"health"'*) printf '{"id":%s,"ok":true,"result":{"state":"ready","process_id":%s,"diagnostic_code":null}}\n' "$id" "$$" ;;
  *'"method":"host.secret"'*'"op":"capture"'*'"read":"qr"'*) printf '%s' "$request" > "$1/capture"; printf '{"id":%s,"ok":true,"result":{"value":"%s"}}\n' "$id" "$(cat "$1/qr.b64")" ;;
  *'"method":"host.secret"'*'"op":"capture"'*'"read":"node","reveal_node_ref":null'*) printf '%s' "$request" > "$1/capture"; printf '{"id":%s,"ok":true,"result":{"value":"Or enter the secret: jbsw y3dp ehpk 3pxp"}}\n' "$id" ;;
  *'"method":"host.secret"'*'"op":"capture"'*) printf '%s' "$request" > "$1/capture"; printf '{"id":%s,"ok":true,"result":{"value":"%s"}}\n' "$id" "$(cat "$1/value" 2>/dev/null || echo vaudit-captured-fixture-value)" ;;
  *'"method":"host.secret"'*) printf '%s' "$request" | grep -o '"plain_target":true' > "$1/plain" || true; printf '%s' "$request" | sed -n 's/.*"text":"\([^"]*\)".*/\1/p' > "$1/echo"; printf '%s' "$request" | sed -n 's/.*"vault_factor_input":\(true\|false\).*/\1/p' > "$1/factor_input"; printf '%s' "$request" | sed -n 's/.*"_app_view_tab":"\([^"]*\)".*/\1/p' > "$1/app_view_tab"; if printf '%s' "$request" | grep -q '"submit":true'; then printf '{"id":%s,"ok":true,"result":{"inserted":true,"submitted":{"navigated":true,"url":"http://127.0.0.1:8123/welcome","title":"Welcome"}}}\n' "$id"; else printf '{"id":%s,"ok":true,"result":{}}\n' "$id"; fi ;;
  *'"method":"host.'*'"op":"open"'*) : > "$1/opened"; printf '{"id":%s,"ok":true,"result":{"generation":1,"tab_id":"host-tab-new","tabs":[%s,{"tab_id":"host-tab-new","document_id":"doc-login"}]}}\n' "$id" "$tabs" ;;
  *'"op":"stop"'*) printf '{"id":%s,"ok":true,"result":{"state":"stopped","tabs":[]}}\n' "$id" ;;
  *'"op":"state"'*|*'"op":"start"'*|*'"op":"navigate"'*|*'"op":"input"'*) printf '{"id":%s,"ok":true,"result":{"generation":1,"tabs":[%s]}}\n' "$id" "$tabs" ;;
  *'"op":"snapshot"'*) : > "$1/snapshot-observed"; echo=$(cat "$1/echo" 2>/dev/null || true); printf '{"id":%s,"ok":true,"result":{"snapshot":{"browser_generation":1,"target_id":"target-login","document_id":"doc-login","snapshot_revision":1,"accessibility_nodes":[],"dom_documents":[{"document_index":0,"url":"http://127.0.0.1:8123/login","owner_node_ref":null}],"shadow_roots":[],"dom_nodes":[{"node_ref":"backend:7","parent_ref":null,"document_index":0,"node_type":1,"node_name":"INPUT","text":"echo %s","attributes":{"type":"password","id":"password"},"bounds":null}]}}}\n' "$id" "$echo" ;;
  *'"method":"shutdown"'*) printf '{"id":%s,"ok":true,"result":{"state":"stopped","process_id":null,"diagnostic_code":null}}\n' "$id"; exit 0 ;;
  *) printf '{"id":%s,"ok":true,"result":{}}\n' "$id" ;;
 esac
done
"#;

struct Login {
    f: Fixture,
    root: crate::test_support::TestWorktree,
}

/// A retained (unfocused) grant on a tab the agent opened at the login page.
async fn login_room() -> Login {
    login_room_at(SITE).await
}

async fn login_room_at(site: &str) -> Login {
    login_room_with_fixture(fixture(), site).await
}

async fn login_room_with_fixture(f: Fixture, site: &str) -> Login {
    login_room_with_controller(f, site, LOGIN_CONTROLLER).await
}

async fn login_room_with_controller(f: Fixture, site: &str, controller: &str) -> Login {
    let root = crate::test_support::TestWorktree::new("a06-login-controller");
    let script = root.path().join("controller.sh");
    std::fs::write(&script, controller.replace(SITE, site)).unwrap();
    let host = &f.state.owned.kernel_browser_host;
    host.install_fixture_backend("local", &script, root.path());
    host.protected_request(
        "local",
        None,
        "host.browser",
        json!({"op":"start"}),
        json!({"unknown":false,"values":[],"targets":[]}),
    )
    .unwrap();
    let agent = f.request.target_agent_id.clone().unwrap();
    host.request_grant(
        "local",
        &agent,
        "owner-request",
        std::time::Duration::from_secs(3600),
        None,
        &f.request.session_id,
    )
    .unwrap();
    call(
        &f,
        "chariox.kernel_browser",
        json!({"command":{"op":"open","url":format!("{site}/login")}}),
    )
    .await
    .unwrap();
    Login { f, root }
}

fn needs_focus(error: DaemonError) -> bool {
    matches!(
        error,
        DaemonError::UserDomainRefused {
            reason: crate::error::UserDomainRefusalReason::SensitiveRequiresFocus
        }
    )
}

fn paste_args(handle: &str) -> Value {
    json!({"credential_id": handle, "tab_id": "host-tab-new", "generation": 1,
        "document_id": "doc-login", "node_ref": "backend:7"})
}

fn inserted(login: &Login) -> Option<String> {
    std::fs::read_to_string(login.root.path().join("echo"))
        .ok()
        .map(|text| text.trim_end().to_string())
}

/// Registers a Vault credential directly, as the owner's product path would.
fn owner_credential(f: &Fixture, id: &str, hosts: &[&str], generated_in: Option<&str>) {
    let service = f.state.home_runtime_secret_service().unwrap();
    service
        .upsert_vault_backed_credential_with_secret(
            &crate::credential::CharioxCredentialRegistry::user().unwrap(),
            UserCredentialConfig {
                id: id.into(),
                description: None,
                source: crate::config::UserCredentialSourceConfig::Vault { key: id.into() },
                allowed_hosts: hosts.iter().map(|host| host.to_string()).collect(),
                allowed_uses: vec![crate::config::UserCredentialUse::Browser],
                injection: crate::config::UserCredentialInjectionConfig::Browser,
                metadata: generated_in.map(|session| UserCredentialMetadataConfig {
                    payment_card: None,
                    use_binding: Some(crate::config::VaultUseBinding {
                        target: crate::config::VaultUseTarget::site(hosts[0]).unwrap(),
                        value_kind: crate::config::VaultValueKind::Secret,
                    }),
                    created_by_kind: Some("vault_generate".into()),
                    created_by_id: Some("other-agent".into()),
                    session_id: Some(session.into()),
                    provider: None,
                    provider_run_id: None,
                    vault_key: Some(id.into()),
                    created_at_ms: None,
                    updated_at_ms: None,
                }),
            },
            "a06-owner-fixture-value",
            false,
        )
        .unwrap();
}

// A submit that navigates is the submit working: the agent gets the page it landed on, not an error.
#[tokio::test]
async fn vpaste_submit_that_navigates_reports_the_resulting_page() {
    crate::test_support::isolated_env_test!();
    let login = login_room().await;
    let f = &login.f;
    unlock(f);
    let handle = generate(f, "signup", SITE).await.unwrap()["credential_id"]
        .as_str()
        .unwrap()
        .to_string();
    let mut args = paste_args(&handle);
    args["submit"] = json!(true);
    let result = call(f, PASTE, args).await.unwrap();
    assert_eq!(
        result,
        json!({"inserted": true, "submitted": {"navigated": true,
            "url": "http://127.0.0.1:8123/welcome", "title": "Welcome"}})
    );
    let secret = stored(f, &handle);
    assert!(!result.to_string().contains(secret.as_str()));
}

#[tokio::test]
async fn vopen_ordinary_agent_fills_only_the_bound_origin_without_a_prompt() {
    crate::test_support::isolated_env_test!();
    let login = login_room().await;
    let f = &login.f;
    unlock(f);
    // An unfocused holder of the grant is not refused for lack of focus.
    let ordinary = call(f, PASTE, paste_args("gen-unknown")).await.unwrap_err();
    assert!(!needs_focus(ordinary));
    let handle = generate(f, "signup", SITE).await.unwrap()["credential_id"]
        .as_str()
        .unwrap()
        .to_string();
    let elsewhere = generate(f, "elsewhere", "http://127.0.0.1:9999")
        .await
        .unwrap()["credential_id"]
        .as_str()
        .unwrap()
        .to_string();
    owner_credential(f, "unbound-owner", &[], None);
    assert!(call(f, PASTE, paste_args(&elsewhere)).await.is_err());
    assert_eq!(inserted(&login), None, "another origin reached the page");
    // P3-1: a credential without a site binding fills like it does for a focused agent.
    assert!(call(f, PASTE, paste_args("unbound-owner")).await.is_ok());
    assert_eq!(
        std::fs::read_to_string(login.root.path().join("factor_input"))
            .unwrap()
            .trim(),
        "false"
    );
    let _ = std::fs::remove_file(login.root.path().join("echo"));
    let mut stale = paste_args(&handle);
    stale["document_id"] = json!("doc-replaced");
    assert!(call(f, PASTE, stale).await.is_err());
    let result = call(f, PASTE, paste_args(&handle)).await.unwrap();
    assert_eq!(result, json!({"inserted": true}));
    let secret = stored(f, &handle);
    assert_eq!(
        inserted(&login).as_deref(),
        Some(secret.as_str()),
        "A06: the kernel filled the generated value"
    );
    // MP-11: the capture policy records the exact filled field for masking.
    let scope = crate::runtime::kernel_browser_host::KernelBrowserHost::profile_key("local");
    let policy: Value = serde_json::from_str(
        &f.state
            .owned
            .kernel_browser_secret_observations
            .capture_policy(&scope)
            .unwrap(),
    )
    .unwrap();
    assert!(policy["targets"].as_array().unwrap().iter().any(|target| {
        target["kind"] == "browser"
            && target["target_id"] == "target-login"
            && target["document_id"] == "doc-login"
            && target["node_ref"] == "backend:7"
    }));
    // The page echoes the value back: protected observations scrub it.
    let observed = call(
        f,
        "chariox.kernel_browser",
        json!({"command":{"op":"snapshot","tab_id":"host-tab-new","generation":1}}),
    )
    .await
    .unwrap()
    .to_string();
    assert!(!observed.contains(secret.as_str()));
    assert!(observed.contains("[redacted]"));
    std::fs::remove_file(login.root.path().join("echo")).unwrap();
    // The fill rides the browser grant, not the sudo window: it still types after sudo ended.
    assert_eq!(
        call(f, PASTE, paste_args(&handle)).await.unwrap(),
        json!({"inserted": true})
    );
    assert!(inserted(&login).is_some());
    f.state.shutdown_cleanup().await.unwrap();
}

// MP-08/MP-11 #1001 P1: a Vault fill into the owner's App Tab is the agent's
// input to it. It needs the same App view claim as the agent's other input,
// and the host receives that claim, so the App's field handlers run as the agent.
#[cfg(any(target_os = "macos", all(target_os = "linux", target_env = "gnu")))]
#[tokio::test]
async fn a06_vault_fill_into_an_app_tab_is_claimed_agent_input() {
    crate::test_support::isolated_env_test!();
    let login = login_room().await;
    let f = &login.f;
    let views = f.state.app_control().user_views();
    let view = views
        .open(
            "local",
            crate::runtime::app_views::AppViewBinding {
                owner: "local".into(),
                installation: "vault-app".into(),
                generation: 1,
                panel: Default::default(),
                logical_tab: None,
            },
            "vault-app",
        )
        .unwrap();
    views
        .bind_browser(
            "local",
            &view.view_id,
            "target-login".into(),
            crate::local::UserAppViewBrowser {
                tab_id: "host-tab-new".into(),
                generation: 1,
            },
        )
        .unwrap();
    unlock(f);
    let _turn = elevated(f);
    let handle = generate(f, "signup", SITE).await.unwrap()["credential_id"]
        .as_str()
        .unwrap()
        .to_string();
    let app_view_tab = || std::fs::read_to_string(login.root.path().join("app_view_tab")).ok();
    // An unfocused agent has not claimed this App view: nothing reaches it.
    assert!(call(f, PASTE, paste_args(&handle)).await.is_err());
    assert_eq!(inserted(&login), None);
    assert_eq!(app_view_tab(), None);
    // Focused, it claims the view as for any other input; then the sudo fill
    // carries that claim to the host.
    let host = &f.state.owned.kernel_browser_host;
    let agent = f.request.target_agent_id.clone().unwrap();
    host.set_focus("local", Some(&agent));
    call(
        f,
        "chariox.kernel_browser",
        json!({"command":{"op":"snapshot","tab_id":"host-tab-new","generation":1}}),
    )
    .await
    .unwrap();
    host.set_focus("local", None);
    assert_eq!(
        call(f, PASTE, paste_args(&handle)).await.unwrap(),
        json!({"inserted": true})
    );
    assert_eq!(
        app_view_tab().as_deref().map(str::trim_end),
        Some("host-tab-new")
    );
    f.state.shutdown_cleanup().await.unwrap();
}

#[tokio::test]
async fn a06_login_rechecks_the_window_and_protection_after_the_vault_wait() {
    crate::test_support::isolated_env_test!();
    let login = login_room().await;
    let f = &login.f;
    let turn = elevated(f);
    unlock(f);
    let handle = generate(f, "signup", SITE).await.unwrap()["credential_id"]
        .as_str()
        .unwrap()
        .to_string();
    let session = turn.session_id.clone();
    // Protection epoch: another protected value enters the browser mid-wait.
    crate::secret::lock_chariox_encrypted_vault(vault_path(f)).unwrap();
    let scope = crate::runtime::kernel_browser_host::KernelBrowserHost::profile_key("local");
    let (result, ()) = tokio::join!(
        call(f, PASTE, paste_args(&handle)),
        owner_unlocks(f, &session, || {
            f.state
                .owned
                .kernel_browser_secret_observations
                .register(&scope, "a06-other-protected-value")
                .unwrap();
        })
    );
    assert!(result
        .unwrap_err()
        .to_string()
        .contains("observation protection changed"));
    assert_eq!(inserted(&login), None);
    // Window expiry/revocation while the owner is unlocking.
    crate::secret::lock_chariox_encrypted_vault(vault_path(f)).unwrap();
    let entry = turn.entry_id.clone();
    let (result, ()) = tokio::join!(
        call(f, PASTE, paste_args(&handle)),
        owner_unlocks(f, &session, || {
            f.state
                .revoke_sudo(Some("local"), Some(&entry), "explicit_revoke")
                .unwrap();
        })
    );
    assert!(result.is_err());
    assert_eq!(inserted(&login), None);
    f.state.shutdown_cleanup().await.unwrap();
}

// MP-08/MP-10/MP-11 A06 review #937 P1: generated handles bind exact origins.
#[tokio::test]
async fn a06_generated_login_refuses_https_downgrade_and_port_rebinding() {
    crate::test_support::isolated_env_test!();
    for (origin, target) in [
        ("https://127.0.0.1", "http://127.0.0.1"),
        ("https://127.0.0.1", "https://127.0.0.1:8443"),
        ("https://127.0.0.1:8443", "https://127.0.0.1"),
    ] {
        let login = login_room_at(target).await;
        let f = &login.f;
        elevated(f);
        unlock(f);
        let handle = generate(f, "origin-fence", origin).await.unwrap()["credential_id"]
            .as_str()
            .unwrap()
            .to_string();
        assert!(
            call(f, PASTE, paste_args(&handle)).await.is_err(),
            "generated credential for {origin} was inserted at {target}"
        );
        assert_eq!(inserted(&login), None);
        f.state.shutdown_cleanup().await.unwrap();
    }
}

#[tokio::test]
async fn a06_generation_retry_refuses_scheme_and_port_rebinding() {
    crate::test_support::isolated_env_test!();
    let f = fixture();
    elevated(&f);
    unlock(&f);
    let first = generate(&f, "exact-origin", "https://127.0.0.1")
        .await
        .unwrap();
    assert!(generate(&f, "exact-origin", "http://127.0.0.1")
        .await
        .is_err());
    assert!(generate(&f, "exact-origin", "https://127.0.0.1:8443")
        .await
        .is_err());
    let same = generate(&f, "exact-origin", "https://127.0.0.1:443/login")
        .await
        .unwrap();
    assert_eq!(same["credential_id"], first["credential_id"]);
    assert_eq!(same["created"], false);
    let credential = crate::credential::CharioxCredentialRegistry::user()
        .unwrap()
        .get(first["credential_id"].as_str().unwrap())
        .unwrap()
        .unwrap();
    assert_eq!(credential.allowed_hosts, ["https://127.0.0.1"]);
    // Default-port normalization preserves the saved origin through login.
    let service = f.state.home_runtime_secret_service().unwrap();
    assert!(service
        .validate_browser_secret_input_for_target_url(&credential.id, "https://127.0.0.1:443/login")
        .is_ok());
    f.state.shutdown_cleanup().await.unwrap();
}

// MP-08 / MP-09 / MP-10 / MP-11: sudo expiry/revocation never gates ordinary storage.
async fn queued_generation_ignores_sudo_end(expire: bool) {
    let f = fixture();
    let turn = elevated(&f);
    unlock(&f);
    let guard = f.state.vault_observation_mutation_guard().await;
    let pending = generate(&f, "queued-write", SITE);
    tokio::pin!(pending);
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(20), &mut pending)
            .await
            .is_err()
    );
    if expire {
        f.state
            .owned
            .sudo_turns
            .lock()
            .unwrap()
            .get_mut(&turn.entry_id)
            .unwrap()
            .deadline = Some(std::time::Instant::now());
    } else {
        f.state
            .revoke_sudo(Some("local"), Some(&turn.entry_id), "explicit_revoke")
            .unwrap();
    }
    drop(guard);
    let result = pending.await;
    if expire {
        assert_eq!(result.unwrap()["created"], true);
        assert_eq!(crate::credential::load_user_credentials().unwrap().len(), 1);
    } else {
        assert!(
            result.is_err(),
            "a cancelled prompt cannot commit a delayed write"
        );
        assert!(crate::credential::load_user_credentials()
            .unwrap()
            .is_empty());
    }
    f.state.shutdown_cleanup().await.unwrap();
}

#[tokio::test]
async fn vopen_generation_storage_wait_rejects_cancelled_prompt() {
    crate::test_support::isolated_env_test!();
    queued_generation_ignores_sudo_end(false).await;
}

#[tokio::test]
async fn vopen_generation_storage_wait_survives_expired_window() {
    crate::test_support::isolated_env_test!();
    queued_generation_ignores_sudo_end(true).await;
}

// MP-08 / MP-09 / MP-10 / MP-11: cards ask once per actual fill, even under sudo.
#[tokio::test]
async fn vopen_cards_ask_each_time_and_never_resolve_without_approval() {
    crate::test_support::isolated_env_test!();
    let login = card_room(SITE).await;
    let f = &login.f;
    unlock(f);
    let _sudo = elevated(f);
    let binding = crate::config::VaultUseBinding {
        target: crate::config::VaultUseTarget::site(SITE).unwrap(),
        value_kind: crate::config::VaultValueKind::PaymentCard,
    };
    let credential: UserCredentialConfig = serde_json::from_value(json!({
        "id":"test-card", "source":{"type":"vault","key":"test-card"},
        "allowed_hosts":[SITE], "allowed_uses":["browser"], "injection":{"kind":"browser"},
        "metadata":{"use_binding":binding}
    }))
    .unwrap();
    let registry = crate::credential::CharioxCredentialRegistry::user().unwrap();
    f.state
        .home_runtime_secret_service()
        .unwrap()
        .upsert_vault_backed_credential_with_secret(
            &registry,
            credential,
            "card-fixture-value",
            false,
        )
        .unwrap();
    assert!(f
        .state
        .home_runtime_secret_service()
        .unwrap()
        .browser_secret_input_for_target_url("test-card", SITE)
        .is_err());
    for allow in [true, false, true] {
        next_turn(f, &format!("card-turn-{}", rand::random::<u32>()));
        let (asked, result) = card_use(
            f,
            if allow { "allow" } else { "cancel" },
            allow.then_some(PASSKEY),
            || call(f, PASTE, paste_args("test-card")),
        )
        .await;
        assert!(asked.is_some());
        assert_eq!(result.is_ok(), allow);
        assert_eq!(inserted(&login).is_some(), allow);
        let _ = std::fs::remove_file(login.root.path().join("echo"));
    }
}

// Published payment-provider test card, never a real one.
const CARD_NUMBER: &str = "4242424242424242";
const CARD_CODE: &str = "7316";

async fn answer_card(
    f: &Fixture,
    id: &str,
    choice: &str,
    passkey: Option<&str>,
) -> Result<(), DaemonError> {
    f.state
        .answer_terminal_runtime_interaction(
            &f.request.session_id,
            id,
            choice,
            None,
            Some("local"),
            passkey.map(crate::local::ApprovalPasskey::new).as_ref(),
            None,
            Some(crate::local::KernelConnectionClass::Terminal),
        )
        .await
}

/// The owner's pending payment decision in the asking agent's Room, once a fill raised it.
fn pending_card(f: &Fixture) -> Option<crate::session::RuntimeInteraction> {
    f.state
        .owned
        .session_store
        .get_session(&f.request.session_id)
        .unwrap()
        .active_interactions()
        .iter()
        .find(|interaction| interaction.id().starts_with("vault-card-"))
        .cloned()
}

async fn card_decision(f: &Fixture) -> crate::session::RuntimeInteraction {
    pending_card(f).expect("the fill asks the owner")
}

fn card_pending(f: &Fixture) -> bool {
    pending_card(f).is_some()
}

/// A Room with event tools, so a card payment can ask its owner without blocking the turn.
async fn card_room(site: &str) -> Login {
    login_room_with_fixture(fixture_with_options(None, true), site).await
}

/// The agent's current turn runs as a task, the way the kernel starts a turn.
fn asking_task(f: &Fixture) -> String {
    use crate::durable_state::agent_lifecycle::{Operation, Outcome};
    let agent = f.request.target_agent_id.as_deref().unwrap();
    let session = f
        .state
        .owned
        .session_store
        .get_session(&f.request.session_id)
        .unwrap();
    let prompts = &f.state.owned.prompt_state_owner;
    let active = prompts
        .active_prompt_for_agent_snapshot(&session, agent)
        .unwrap();
    if active.status() != PromptStatus::Running {
        let mut started = active.clone();
        started.set_status(PromptStatus::Running);
        assert!(prompts.replace_active_prompt_if_matches(&session, agent, &active, started));
    }
    let store = &f.state.owned.durable_state_store;
    if let Some(task) = store
        .agent_tasks(Some(&f.request.session_id), Some(agent))
        .unwrap()
        .into_iter()
        .find(|task| task.prompt_id == active.id())
    {
        return task.task_id;
    }
    let Outcome::Task(task) = store
        .agent_lifecycle(Operation::Begin {
            owner: "local".into(),
            room: f.request.session_id.clone(),
            agent: agent.into(),
            prompt: active.id().into(),
            run: Some(f.run.id().into()),
            now: crate::session::unix_epoch_ms(),
        })
        .unwrap()
    else {
        panic!("begin returns the task")
    };
    task.task_id
}

/// A structured card saved the way the owner's Create saves it: for any site.
fn saved_card(f: &Fixture) {
    let card = crate::config::PaymentCard::parse(
        &json!({"number": CARD_NUMBER, "expiry_month": 12, "expiry_year": 2099,
            "security_code": CARD_CODE, "name": "Ada Lovelace"})
        .to_string(),
        crate::session::unix_epoch_ms(),
    )
    .unwrap();
    let credential: UserCredentialConfig = serde_json::from_value(json!({
        "id": "saved-card", "description": "Travel card",
        "source": {"type": "vault", "key": "saved-card"},
        "allowed_hosts": [], "allowed_uses": ["browser"], "injection": {"kind": "browser"},
        "metadata": {"use_binding": {"target": {"kind": "any_site"},
            "value_kind": "payment_card"}, "payment_card": card.summary()}
    }))
    .unwrap();
    f.state
        .home_runtime_secret_service()
        .unwrap()
        .upsert_vault_backed_credential_with_secret(
            &crate::credential::CharioxCredentialRegistry::user().unwrap(),
            credential,
            &card.canonical(),
            false,
        )
        .unwrap();
}

fn card_fill(field: &str, payment: Option<&Value>) -> Value {
    let mut args = paste_args("saved-card");
    args["card_field"] = json!(field);
    if let Some(payment) = payment {
        args["payment"] = payment.clone();
    }
    args
}

fn no_card_data(text: &str) {
    assert!(
        !text.contains(CARD_NUMBER) && !text.contains(CARD_CODE),
        "card data in an output"
    );
}

/// Runs a card use; when it asks the owner, answers with `choice` and, after an Allow, runs it again.
async fn card_use<T, Fut>(
    f: &Fixture,
    choice: &str,
    passkey: Option<&str>,
    work: impl Fn() -> Fut,
) -> (
    Option<crate::session::RuntimeInteraction>,
    Result<T, DaemonError>,
)
where
    Fut: std::future::Future<Output = Result<T, DaemonError>>,
{
    asking_task(f);
    assert!(!card_pending(f), "a decision was already open");
    let first = work().await;
    let Some(decision) = pending_card(f) else {
        return (None, first);
    };
    assert!(first.is_err(), "a fill that asks fills nothing");
    answer_card(f, decision.id(), choice, passkey)
        .await
        .unwrap();
    assert!(!card_pending(f), "the answer closes the card");
    if choice == "allow" {
        (Some(decision), work().await)
    } else {
        (Some(decision), first)
    }
}

/// Ends the agent's current turn and starts its next one.
fn next_turn(f: &Fixture, prompt_id: &str) {
    let agent = f.request.target_agent_id.as_deref().unwrap();
    let session = f
        .state
        .owned
        .session_store
        .get_session(&f.request.session_id)
        .unwrap();
    let prompts = &f.state.owned.prompt_state_owner;
    prompts.cancel_active_prompt_only(&session, agent).unwrap();
    prompts
        .submit_prepared_prompt_with_queue_policy(
            &session,
            PromptQueueItem::new(
                prompt_id,
                &f.request.attachment_id,
                agent,
                "next task",
                PromptStatus::Queued,
            ),
            false,
            false,
        )
        .unwrap();
}

const PAYMENT: &str = r#"{"merchant": "Example Shop", "amount": "12.50", "currency": "EUR"}"#;

fn payment() -> Value {
    serde_json::from_str(PAYMENT).unwrap()
}

// MP-08 / MP-09 / MP-10 / MP-11: the owner approves one payment with the passkey on a card that
// shows the observed origin and the agent's claim. During that turn the agent fills every field
// of that card for that payment as often as the checkout needs, and learns only that the fill
// happened. Another claim or the next turn asks again. Sudo answers none of it.
#[tokio::test]
async fn vopen_one_approved_payment_fills_every_card_field_while_the_turn_lasts() {
    crate::test_support::isolated_env_test!();
    let login = card_room(SITE).await;
    let f = &login.f;
    unlock(f);
    let _sudo = elevated(f);
    saved_card(f);
    let echo = login.root.path().join("echo");
    let agent = f.run.agent_instance_id().unwrap().to_owned();
    let payment = payment();
    // A saved card fills one named field of one described payment; a bad claim is refused.
    // None of these asks the owner.
    let mut bad = payment.clone();
    bad["amount"] = json!("12,50 EUR");
    for args in [
        paste_args("saved-card"),
        card_fill("number", Some(&bad)),
        card_fill("number", None),
        card_fill("number", Some(&json!({"merchant": "Example Shop"}))),
    ] {
        assert!(call(f, PASTE, args).await.is_err());
    }
    assert!(!card_pending(f) && inserted(&login).is_none());

    asking_task(f);
    let asked = call(f, PASTE, card_fill("number", Some(&payment))).await;
    // The agent is not held: it learns at once that the owner was asked, and nothing is filled.
    let refusal = asked.unwrap_err().to_string();
    assert!(refusal.contains("Keep working"), "{refusal}");
    assert!(inserted(&login).is_none());
    let decision = card_decision(f).await;
    assert_eq!(decision.agent_id(), Some(agent.as_str()));
    assert_eq!(
        decision.timeout_sec(),
        Some(24 * 60 * 60),
        "the card waits for the owner"
    );
    // What every client of the Room reads: the shape the web's unified card and its fixtures use.
    let listed = serde_json::to_value(&decision).unwrap();
    assert_eq!(
        (
            &listed["agent_id"],
            &listed["kind"],
            &listed["level"],
            &listed["title"]
        ),
        (
            &json!(agent),
            &json!("choice"),
            &json!("critical"),
            &json!("Use payment card")
        )
    );
    assert_eq!(
        listed["choices"],
        json!([{"id": "allow", "label": "Allow once", "reply": "allow", "requires_passkey": true},
            {"id": "cancel", "label": "Cancel", "reply": "cancel", "style": "danger"}])
    );
    {
        assert_eq!(
            serde_json::to_value(decision.payment().unwrap()).unwrap(),
            json!({"origin": SITE, "entry": "Travel card", "card_last4": "4242",
                "card_brand": "Visa", "agent_claim": payment, "requested_by_agent_id": agent,
                "session_id": f.request.session_id})
        );
        let allow = decision.choice("allow").unwrap();
        assert!(allow.requires_passkey());
        assert_eq!(
            (allow.label(), decision.choice("cancel").unwrap().label()),
            ("Allow once", "Cancel")
        );
        assert!(!decision.choice("cancel").unwrap().requires_passkey());
        // Without the owner's passkey the decision stays open and nothing is filled.
        assert!(answer_card(f, decision.id(), "allow", None).await.is_err());
        assert!(card_pending(f) && inserted(&login).is_none());
        answer_card(f, decision.id(), "allow", Some(PASSKEY))
            .await
            .unwrap();
    }
    let result = call(f, PASTE, card_fill("number", Some(&payment)))
        .await
        .unwrap();
    assert_eq!(inserted(&login).as_deref(), Some(CARD_NUMBER));
    no_card_data(&result.to_string());
    no_card_data(&serde_json::to_string(&decision).unwrap());

    // Every field of this payment fills without another decision, again after a reload.
    for (field, text) in [
        ("security_code", CARD_CODE),
        ("expiry", "12/99"),
        ("name", "Ada Lovelace"),
        ("number", CARD_NUMBER),
        ("security_code", CARD_CODE),
        ("expiry_month", "12"),
        ("expiry_year", "2099"),
    ] {
        let (asked, result) = card_use(f, "cancel", None, || {
            call(f, PASTE, card_fill(field, Some(&payment)))
        })
        .await;
        assert!(asked.is_none(), "{field} asked again");
        no_card_data(&result.unwrap().to_string());
        assert_eq!(inserted(&login).as_deref(), Some(text));
    }
    // Another merchant, amount or currency is another payment: it asks, and a cancel fills nothing.
    for (key, other) in [
        ("merchant", "Other Shop"),
        ("amount", "12.51"),
        ("currency", "USD"),
    ] {
        let mut changed = payment.clone();
        changed[key] = json!(other);
        let _ = std::fs::remove_file(&echo);
        let (asked, result) = card_use(f, "cancel", None, || {
            call(f, PASTE, card_fill("number", Some(&changed)))
        })
        .await;
        let asked = serde_json::to_value(asked.unwrap().payment().unwrap()).unwrap();
        assert_eq!(asked["agent_claim"], changed);
        assert!(result.is_err() && inserted(&login).is_none(), "{key}");
    }
    // The approval ends with the agent's task.
    next_turn(f, "after-payment-turn");
    let (asked, result) = card_use(f, "cancel", None, || {
        call(f, PASTE, card_fill("number", Some(&payment)))
    })
    .await;
    assert!(asked.is_some() && result.is_err() && inserted(&login).is_none());
    let activity = f
        .state
        .home_runtime_secret_service()
        .unwrap()
        .vault_activity("saved-card")
        .unwrap();
    no_card_data(&serde_json::to_string(&activity).unwrap());
}

// MP-08 / MP-09 / MP-10 / MP-11: one card pays at any site. The decision names the origin the
// kernel observed, and a payment approved at one site fills nothing at another.
#[tokio::test]
async fn vopen_a_card_pays_at_any_site_and_another_site_asks_again() {
    crate::test_support::isolated_env_test!();
    const SHOP: &str = "http://127.0.0.1:8124";
    let login = card_room(SHOP).await;
    let f = &login.f;
    unlock(f);
    let _turn = running(f);
    saved_card(f);
    let payment = payment();
    let (asked, result) = card_use(f, "allow", Some(PASSKEY), || {
        call(f, PASTE, card_fill("number", Some(&payment)))
    })
    .await;
    result.unwrap();
    assert_eq!(asked.unwrap().payment().unwrap().origin, SHOP);
    assert_eq!(inserted(&login).as_deref(), Some(CARD_NUMBER));
    let credential = f
        .state
        .home_runtime_secret_service()
        .unwrap()
        .credential("saved-card")
        .unwrap()
        .clone();
    let agent = f.request.target_agent_id.clone().unwrap();
    let claim: crate::local::VaultPaymentClaim = serde_json::from_str(PAYMENT).unwrap();
    let use_at = |origin: &str| {
        let target = crate::config::VaultUseTarget::site(origin).unwrap();
        let credential = credential.clone();
        let agent = agent.clone();
        let claim = claim.clone();
        async move {
            f.state
                .authorize_vault_use(
                    &agent,
                    &credential,
                    &target,
                    super::super::vault_use::VaultUseAsk {
                        agent_id: Some(&agent),
                        session_id: Some(&f.request.session_id),
                        card_field: Some(crate::config::PaymentCardField::Number),
                        claim: Some(&claim),
                    },
                )
                .await
                .map(|_| ())
        }
    };
    let (asked, result) = card_use(f, "cancel", None, || use_at(SHOP)).await;
    assert!(asked.is_none() && result.is_ok());
    let (asked, result) = card_use(f, "cancel", None, || use_at(SITE)).await;
    assert_eq!(asked.unwrap().payment().unwrap().origin, SITE);
    assert!(result.is_err());
}

const OTHER_CARD_NUMBER: &str = "5555555555554444";
const OTHER_CARD_CODE: &str = "9081";

fn other_card() -> String {
    json!({"number": OTHER_CARD_NUMBER, "expiry_month": 11, "expiry_year": 2098,
        "security_code": OTHER_CARD_CODE, "name": "Grace Hopper"})
    .to_string()
}

/// The owner replaces the saved card through their Vault window.
async fn owner_replaces_card(f: &Fixture) {
    use crate::local::{LocalDaemonRequest, OwnerVaultRequest, VaultPassphrase};
    use crate::runtime::command::{KernelCaller, KernelCommand, KernelCommandSource};
    let request = LocalDaemonRequest::OwnerVault(OwnerVaultRequest::Replace {
        id: "saved-card".into(),
        value: VaultPassphrase::new(other_card()),
    });
    let caller = KernelCaller::for_source(&KernelCommandSource::LocalCli)
        .with_connection_class(crate::local::KernelConnectionClass::Terminal);
    let command = KernelCommand::from_local_request_with_caller(
        "owner-replace",
        KernelCommandSource::LocalCli,
        caller,
        None,
        None,
        &request,
    );
    f.router.dispatch(command, request).await.unwrap();
}

fn no_card_number(login: &Login) {
    let filled = inserted(login);
    assert!(
        !matches!(
            filled.as_deref(),
            Some(CARD_NUMBER | OTHER_CARD_NUMBER | OTHER_CARD_CODE)
        ),
        "a card value was filled"
    );
}

// MP-08 / MP-09 / MP-10 / MP-11: the owner's decision is about the card it shows. When that card
// is replaced while the decision is open, Allow fills nothing (neither card) and keeps no
// approval for the replacement: its next fill asks again.
#[tokio::test]
async fn vopen_a_card_replaced_during_its_decision_fills_nothing() {
    crate::test_support::isolated_env_test!();
    let login = card_room(SITE).await;
    let f = &login.f;
    unlock(f);
    let _turn = running(f);
    saved_card(f);
    let payment = payment();
    asking_task(f);
    let result = call(f, PASTE, card_fill("number", Some(&payment))).await;
    let decision = card_decision(f).await;
    assert_eq!(
        decision.payment().unwrap().card_last4.as_deref(),
        Some("4242")
    );
    owner_replaces_card(f).await;
    answer_card(f, decision.id(), "allow", Some(PASSKEY))
        .await
        .unwrap();
    assert!(result.is_err(), "the replaced card was filled");
    no_card_number(&login);
    for field in ["number", "security_code"] {
        let (asked, result) = card_use(f, "cancel", None, || {
            call(f, PASTE, card_fill(field, Some(&payment)))
        })
        .await;
        let asked = asked.expect("the replacement needs its own decision");
        assert_eq!(asked.payment().unwrap().card_last4.as_deref(), Some("4444"));
        assert!(result.is_err(), "{field}");
        no_card_number(&login);
    }
}

// MP-08 / MP-09 / MP-10 / MP-11: an approved payment belongs to the saved card it was approved
// for. Once that value is replaced, by the owner or by any other Vault write, the payment fills
// nothing more and the next fill asks again.
#[tokio::test]
async fn vopen_a_payment_approved_before_a_card_replacement_fills_nothing_after_it() {
    crate::test_support::isolated_env_test!();
    let login = card_room(SITE).await;
    let f = &login.f;
    unlock(f);
    let _turn = running(f);
    saved_card(f);
    let payment = payment();
    let echo = login.root.path().join("echo");
    let (asked, result) = card_use(f, "allow", Some(PASSKEY), || {
        call(f, PASTE, card_fill("number", Some(&payment)))
    })
    .await;
    assert!(asked.is_some());
    result.unwrap();
    assert_eq!(inserted(&login).as_deref(), Some(CARD_NUMBER));
    // A Vault write that is not the owner's Replace: the same entry, another card.
    let service = f.state.home_runtime_secret_service().unwrap();
    let mut credential = service.credential("saved-card").unwrap().clone();
    let card =
        crate::config::PaymentCard::parse(&other_card(), crate::session::unix_epoch_ms()).unwrap();
    credential.metadata.as_mut().unwrap().payment_card = Some(card.summary());
    service
        .upsert_vault_backed_credential_with_secret(
            &crate::credential::CharioxCredentialRegistry::user().unwrap(),
            credential,
            &card.canonical(),
            true,
        )
        .unwrap();
    for field in ["security_code", "number"] {
        let _ = std::fs::remove_file(&echo);
        let (asked, result) = card_use(f, "cancel", None, || {
            call(f, PASTE, card_fill(field, Some(&payment)))
        })
        .await;
        assert!(
            asked.is_some(),
            "{field} filled under the old card's approval"
        );
        assert!(result.is_err() && inserted(&login).is_none(), "{field}");
    }
    // The owner's Replace ends it too.
    let (_, result) = card_use(f, "allow", Some(PASSKEY), || {
        call(f, PASTE, card_fill("number", Some(&payment)))
    })
    .await;
    result.unwrap();
    assert_eq!(inserted(&login).as_deref(), Some(OTHER_CARD_NUMBER));
    owner_replaces_card(f).await;
    let _ = std::fs::remove_file(&echo);
    let (asked, result) = card_use(f, "cancel", None, || {
        call(f, PASTE, card_fill("security_code", Some(&payment)))
    })
    .await;
    assert!(asked.is_some() && result.is_err() && inserted(&login).is_none());
}

// MP-08 / MP-09 / MP-10 / MP-11: an approval lasts at most 15 minutes; one that ran out, or a
// decision that timed out, fills nothing.
#[tokio::test]
async fn vopen_an_expired_card_payment_fills_nothing() {
    crate::test_support::isolated_env_test!();
    let login = card_room(SITE).await;
    let f = &login.f;
    unlock(f);
    let _turn = running(f);
    saved_card(f);
    let echo = login.root.path().join("echo");
    let payment = payment();
    let (_, first) = card_use(f, "allow", Some(PASSKEY), || {
        call(f, PASTE, card_fill("name", Some(&payment)))
    })
    .await;
    first.unwrap();
    assert_eq!(inserted(&login).as_deref(), Some("Ada Lovelace"));
    {
        let mut grants = f.state.owned.vault_payment_grants.lock().unwrap();
        let [grant] = grants.as_mut_slice() else {
            panic!("one approved payment")
        };
        let left = grant
            .expires_at
            .saturating_duration_since(std::time::Instant::now());
        assert!(left > Duration::from_secs(14 * 60) && left <= Duration::from_secs(15 * 60));
        grant.expires_at = std::time::Instant::now();
    }
    let _ = std::fs::remove_file(&echo);
    let late = call(f, PASTE, card_fill("name", Some(&payment))).await;
    assert!(late.is_err());
    assert!(
        inserted(&login).is_none(),
        "an expired payment fills nothing"
    );
    // It asks again, and the new card waits for the owner.
    assert!(card_pending(f));
}

/// The owner's Allow on a card decision, as a client answers it.
async fn allow_card(
    f: &Fixture,
    id: &str,
    passkey: Option<&str>,
    remember_minutes: Option<u32>,
    class: crate::local::KernelConnectionClass,
) -> Result<(), DaemonError> {
    f.state
        .answer_terminal_runtime_interaction(
            &f.request.session_id,
            id,
            "allow",
            None,
            Some("local"),
            passkey.map(crate::local::ApprovalPasskey::new).as_ref(),
            remember_minutes,
            Some(class),
        )
        .await
}

// MP-08 / MP-09 / MP-10 / MP-11 PAYMENT PASSPHRASE RULE (owner, 11-09): a card payment is made only
// under a passphrase-confirmed action. Every payment's Allow needs the typed passkey from a Chariox
// terminal: an open remember window, an unlocked Vault or a sudo window never stands in for it,
// remember minutes are refused (so a payment never starts a window), and no host, agent or
// outside connection answers it.
#[tokio::test]
async fn vopen_every_card_payment_needs_the_typed_passkey() {
    use crate::local::KernelConnectionClass::{ExternalAgent, Host, KernelAgent, Terminal};
    crate::test_support::isolated_env_test!();
    let login = card_room(SITE).await;
    let f = &login.f;
    unlock(f);
    let _sudo = elevated(f);
    saved_card(f);
    let echo = login.root.path().join("echo");
    // An earlier typed passkey left the owner's remember window open.
    f.state.owned.critical_approval_passkeys.record_success(
        "local",
        std::time::Instant::now(),
        Some(15),
    );
    for amount in ["12.50", "99.00"] {
        let mut this = payment();
        this["amount"] = json!(amount);
        asking_task(f);
        assert!(call(f, PASTE, card_fill("number", Some(&this)))
            .await
            .is_err());
        let decision = card_decision(f).await;
        let refusal = |result: Result<(), DaemonError>| result.unwrap_err().to_string();
        // (a) The open window, unlocked Vault and sudo answer nothing.
        let missing = refusal(allow_card(f, decision.id(), None, None, Terminal).await);
        assert!(missing.contains("PASSKEY_REQUIRED"), "{missing}");
        // (c) Remember minutes are refused, even with the right passkey.
        let remember =
            refusal(allow_card(f, decision.id(), Some(PASSKEY), Some(15), Terminal).await);
        assert!(remember.contains("PASSKEY_NOT_ACCEPTED"), "{remember}");
        // (e) Only a Chariox terminal answers it.
        for class in [Host, KernelAgent, ExternalAgent] {
            assert!(
                allow_card(f, decision.id(), Some(PASSKEY), None, class)
                    .await
                    .is_err(),
                "{class:?} answered a payment"
            );
        }
        // (b) A wrong passkey is refused.
        let wrong =
            refusal(allow_card(f, decision.id(), Some("not-the-passkey"), None, Terminal).await);
        assert!(wrong.contains("PASSKEY_REJECTED"), "{wrong}");
        assert!(card_pending(f) && inserted(&login).is_none());
        // Filling again while the card waits fills nothing and shows no second card.
        let again = call(f, PASTE, card_fill("number", Some(&this)))
            .await
            .unwrap_err()
            .to_string();
        assert!(again.contains(decision.id()), "{again}");
        assert_eq!(
            f.state
                .owned
                .session_store
                .get_session(&f.request.session_id)
                .unwrap()
                .active_interactions()
                .iter()
                .filter(|interaction| interaction.id().starts_with("vault-card-"))
                .count(),
            1
        );
        // (b) The typed passkey approves this one payment.
        allow_card(f, decision.id(), Some(PASSKEY), None, Terminal)
            .await
            .unwrap();
        call(f, PASTE, card_fill("number", Some(&this)))
            .await
            .unwrap();
        assert_eq!(inserted(&login).as_deref(), Some(CARD_NUMBER));
        // (f) The audit shows a typed passkey, never a remembered one.
        let outcomes: Vec<String> = f
            .state
            .owned
            .durable_state_store
            .load_subject_events_by_kind(decision.id(), "critical_approval.passkey", 50)
            .unwrap()
            .into_iter()
            .map(|event| event.payload["outcome"].as_str().unwrap().to_owned())
            .collect();
        assert!(
            outcomes.iter().all(|outcome| outcome != "remembered"),
            "{outcomes:?}"
        );
        assert_eq!(outcomes.last().map(String::as_str), Some("verified"));
        // (g) The next payment asks again and needs its own typed passkey.
        let _ = std::fs::remove_file(&echo);
    }
}

// MP-08 / MP-09 / MP-10 / MP-11: two payments at once are two cards in the agent's pane, and each
// Allow approves only its own payment; the other card keeps waiting.
#[tokio::test]
async fn vopen_two_payments_at_once_are_two_cards() {
    crate::test_support::isolated_env_test!();
    let login = card_room(SITE).await;
    let f = &login.f;
    unlock(f);
    let _turn = running(f);
    saved_card(f);
    asking_task(f);
    let first = payment();
    let mut second = payment();
    second["amount"] = json!("99.00");
    for claim in [&first, &second] {
        assert!(call(f, PASTE, card_fill("number", Some(claim)))
            .await
            .is_err());
    }
    let cards: Vec<_> = f
        .state
        .owned
        .session_store
        .get_session(&f.request.session_id)
        .unwrap()
        .active_interactions()
        .iter()
        .filter(|interaction| interaction.id().starts_with("vault-card-"))
        .cloned()
        .collect();
    assert_eq!(cards.len(), 2);
    let amount = |card: &crate::session::RuntimeInteraction| {
        card.payment()
            .unwrap()
            .agent_claim
            .as_ref()
            .unwrap()
            .amount
            .clone()
    };
    let later = cards.iter().find(|card| amount(card) == "99.00").unwrap();
    answer_card(f, later.id(), "allow", Some(PASSKEY))
        .await
        .unwrap();
    call(f, PASTE, card_fill("number", Some(&second)))
        .await
        .unwrap();
    assert_eq!(inserted(&login).as_deref(), Some(CARD_NUMBER));
    let _ = std::fs::remove_file(login.root.path().join("echo"));
    assert!(call(f, PASTE, card_fill("number", Some(&first)))
        .await
        .is_err());
    assert!(inserted(&login).is_none());
    let waiting = card_decision(f).await;
    assert_eq!(amount(&waiting), "12.50");
}

#[tokio::test]
async fn vopen_overwrite_and_alias_cannot_rebind_or_downgrade_an_entry() {
    crate::test_support::isolated_env_test!();
    let f = fixture();
    unlock(&f);
    let handle = generate(&f, "bound-entry", SITE).await.unwrap()["credential_id"]
        .as_str()
        .unwrap()
        .to_owned();
    let registry = crate::credential::CharioxCredentialRegistry::user().unwrap();
    let original = registry.get(&handle).unwrap().unwrap();
    let mut changed = original.clone();
    changed
        .metadata
        .as_mut()
        .unwrap()
        .use_binding
        .as_mut()
        .unwrap()
        .target = crate::config::VaultUseTarget::site("https://example.com").unwrap();
    assert!(registry.upsert(changed).is_err());
    let mut alias = original.clone();
    alias.id = "alias".into();
    alias.metadata = None;
    assert!(registry.upsert(alias).is_err());
    let mut stripped = original.clone();
    stripped.metadata = None;
    assert!(registry.upsert(stripped).is_err());
    assert_eq!(registry.get(&handle).unwrap().unwrap(), original);
    assert!(f
        .state
        .home_runtime_secret_service()
        .unwrap()
        .browser_secret_input(&handle)
        .is_err());
}

#[tokio::test]
async fn vopen_agent_is_rechecked_after_the_final_storage_barrier() {
    crate::test_support::isolated_env_test!();
    let f = fixture();
    unlock(&f);
    let guard = f.state.vault_observation_mutation_guard().await;
    let pending = generate(&f, "delayed-write", SITE);
    tokio::pin!(pending);
    tokio::select! {
        result = &mut pending => panic!("generation passed its held storage barrier: {result:?}"),
        _ = tokio::time::sleep(Duration::from_millis(30)) => {}
    }
    f.state
        .owned
        .provider_store
        .mark_run_ended_provider_only(&f.request.session_id, f.run.id())
        .unwrap();
    drop(guard);
    assert!(pending.await.is_err());
    assert!(crate::credential::load_user_credentials()
        .unwrap()
        .is_empty());
}

#[tokio::test]
async fn vopen_http_card_wait_rechecks_the_live_actor_before_sending() {
    use tokio::io::AsyncWriteExt;
    crate::test_support::isolated_env_test!();
    let f = fixture_with_options(None, true);
    unlock(&f);
    let _turn = running(&f);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let credential: UserCredentialConfig = serde_json::from_value(json!({
        "id":"http-card", "source":{"type":"vault","key":"http-card"},
        "allowed_hosts":[origin], "allowed_uses":["http"],
        "injection":{"kind":"header","name":"Authorization","value":"Bearer ${secret}"},
        "metadata":{"use_binding": {
            "target":{"kind":"site", "origin":origin}, "value_kind":"payment_card"
        }}
    }))
    .unwrap();
    let registry = crate::credential::CharioxCredentialRegistry::user().unwrap();
    f.state
        .home_runtime_secret_service()
        .unwrap()
        .upsert_vault_backed_credential_with_secret(&registry, credential, "fixture-card", false)
        .unwrap();
    asking_task(&f);
    let request = || {
        call(
            &f,
            "chariox.http_request_with_credential",
            json!({"credential_id":"http-card", "url":origin}),
        )
    };
    let receive = async {
        match tokio::time::timeout(Duration::from_secs(2), listener.accept()).await {
            Ok(Ok((mut stream, _))) => {
                let _ = stream
                    .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
                    .await;
                true
            }
            _ => false,
        }
    };
    let ask_then_end = async {
        // The use asks the owner and returns at once; the run then ends before the owner allows.
        assert!(request().await.is_err());
        let decision = card_decision(&f).await;
        f.state
            .owned
            .provider_store
            .mark_run_ended_provider_only(&f.request.session_id, f.run.id())
            .unwrap();
        answer_card(&f, decision.id(), "allow", Some(PASSKEY))
            .await
            .unwrap();
        request().await
    };
    let (result, sent) = tokio::join!(ask_then_end, receive);
    assert!(
        !sent,
        "ended provider run sent a credential-authenticated HTTP request"
    );
    assert!(result.is_err());
}

// MP-08 / MP-09 / MP-10 / MP-11: an HTTP use that fails after "Use attempted" closes its attempt as failed.
#[tokio::test]
async fn an_http_use_that_fails_closes_its_attempt_as_failed() {
    crate::test_support::isolated_env_test!();
    let f = fixture();
    unlock(&f);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    drop(listener);
    let credential: UserCredentialConfig = serde_json::from_value(json!({
        "id":"http-secret", "source":{"type":"vault","key":"http-secret"},
        "allowed_hosts":[origin], "allowed_uses":["http"],
        "injection":{"kind":"header","name":"Authorization","value":"Bearer ${secret}"},
        "metadata":{"use_binding": {"target":{"kind":"site", "origin":origin}}}
    }))
    .unwrap();
    let registry = crate::credential::CharioxCredentialRegistry::user().unwrap();
    f.state
        .home_runtime_secret_service()
        .unwrap()
        .upsert_vault_backed_credential_with_secret(
            &registry,
            credential,
            "fixture-http-secret",
            false,
        )
        .unwrap();
    let result = call(
        &f,
        "chariox.http_request_with_credential",
        json!({"credential_id":"http-secret", "url":origin, "timeout_ms":2000}),
    )
    .await;
    assert!(result.is_err(), "nothing listens, so the request must fail");
    let events = f
        .state
        .home_runtime_secret_service()
        .unwrap()
        .vault_activity("http-secret")
        .unwrap();
    let uses: Vec<&str> = events
        .iter()
        .map(|row| row.event.as_str())
        .filter(|event| event.starts_with("Use"))
        .collect();
    assert_eq!(uses.len(), 2, "{uses:?}");
    assert!(uses[0].starts_with("Use attempted over HTTP by "));
    assert_eq!(uses[1], "Use failed: the action did not finish");
    assert!(!serde_json::to_string(&events)
        .unwrap()
        .contains("fixture-http-secret"));
    f.state.shutdown_cleanup().await.unwrap();
}

// MP-08 / MP-09 / MP-10 / MP-11: owner New secret and ordinary agent use share the actual routers.
#[tokio::test]
async fn vrec_owner_created_entry_fills_and_audits_refusal_without_sudo() {
    crate::test_support::isolated_env_test!();
    let login = login_room().await;
    let f = &login.f;
    unlock(f);
    let owner = |request| {
        let request = LocalDaemonRequest::OwnerVault(request);
        let mut command = crate::runtime::command::KernelCommand::from_local_request(
            "owner-new-secret",
            None,
            None,
            &request,
        );
        command.caller.connection_class = Some(KernelConnectionClass::Terminal);
        f.router.dispatch(command, request)
    };
    let created = owner(crate::local::OwnerVaultRequest::Create {
        name: "Owner login".into(),
        kind: "Password".into(),
        use_binding: crate::config::VaultUseBinding {
            target: crate::config::VaultUseTarget::site(SITE).unwrap(),
            value_kind: Default::default(),
        },
        value: crate::local::VaultPassphrase::new("vrec-owner-only-value"),
    })
    .await
    .unwrap();
    let crate::local::LocalDaemonResponse::OwnerVault {
        result: crate::local::OwnerVaultResult::Saved { id },
    } = created
    else {
        panic!("owner creation failed")
    };
    assert!(f.state.owned.sudo_turns.lock().unwrap().is_empty());
    assert_eq!(
        call(f, PASTE, paste_args(&id)).await.unwrap(),
        json!({"inserted":true})
    );
    assert!(inserted(&login).as_deref() == Some("vrec-owner-only-value"));
    let _ = std::fs::remove_file(login.root.path().join("echo"));
    let registry = crate::credential::CharioxCredentialRegistry::user().unwrap();
    let mut elsewhere = registry.get(&id).unwrap().unwrap();
    // A second entry, never a rebind of the first, exercises exact-origin refusal at the MCP boundary.
    elsewhere.id = "owner-other-site".into();
    elsewhere.source = crate::config::UserCredentialSourceConfig::Vault {
        key: elsewhere.id.clone(),
    };
    elsewhere
        .metadata
        .as_mut()
        .unwrap()
        .use_binding
        .as_mut()
        .unwrap()
        .target = crate::config::VaultUseTarget::site("https://other.example").unwrap();
    f.state
        .home_runtime_secret_service()
        .unwrap()
        .upsert_vault_backed_credential_with_secret(
            &registry,
            elsewhere,
            "other-owner-only-value",
            false,
        )
        .unwrap();
    assert!(call(f, PASTE, paste_args("owner-other-site"))
        .await
        .is_err());
    assert!(inserted(&login).is_none());
    let refused = f
        .state
        .home_runtime_secret_service()
        .unwrap()
        .vault_activity("owner-other-site")
        .unwrap();
    assert!(refused.iter().any(|event| event
        .event
        .starts_with("Use refused for http://127.0.0.1:8123 by ")));
    owner(crate::local::OwnerVaultRequest::Reveal { id: id.clone() })
        .await
        .unwrap();
    let events = f
        .state
        .home_runtime_secret_service()
        .unwrap()
        .vault_activity(&id)
        .unwrap();
    assert!(events
        .iter()
        .any(|event| event.event.contains("Used in browser by")));
    assert!(events.iter().any(|event| event.event == "Viewed by owner"));
    assert!(!serde_json::to_string(&events)
        .unwrap()
        .contains("vrec-owner-only-value"));
    let request = LocalDaemonRequest::OwnerVault(crate::local::OwnerVaultRequest::Reveal { id });
    let mut agent = crate::runtime::command::KernelCommand::from_local_request(
        "agent-reveal",
        None,
        None,
        &request,
    );
    agent.caller.connection_class = Some(KernelConnectionClass::KernelAgent);
    assert!(f.router.dispatch(agent, request).await.is_err());
    assert!(f
        .state
        .owned
        .session_snapshot(&f.request.session_id)
        .unwrap()
        .active_interactions()
        .is_empty());
    f.state.shutdown_cleanup().await.unwrap();
}

fn owner_factor(f: &Fixture) {
    f.state
        .home_runtime_secret_service()
        .unwrap()
        .upsert_vault_backed_credential_with_secret(
            &crate::credential::CharioxCredentialRegistry::user().unwrap(),
            UserCredentialConfig {
                id: "service-totp".into(),
                description: None,
                source: crate::config::UserCredentialSourceConfig::Vault {
                    key: "service-totp".into(),
                },
                allowed_hosts: vec!["127.0.0.1:8123".into()],
                allowed_uses: vec![crate::config::UserCredentialUse::Browser],
                injection: crate::config::UserCredentialInjectionConfig::Totp(
                    crate::config::TotpConfig::default(),
                ),
                metadata: None,
            },
            "GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ",
            false,
        )
        .unwrap();
}

// MP-08 / MP-09 / MP-10 / MP-11: a fill that errors after "Use attempted" closes its attempt in a running kernel.
#[tokio::test]
async fn a_browser_fill_that_errors_closes_its_attempt_as_failed() {
    crate::test_support::isolated_env_test!();
    let controller = LOGIN_CONTROLLER.replace(
        " case \"$request\" in",
        r#" case "$request" in
  *'"method":"host.secret"'*'"text":'*) printf '{"id":%s,"ok":false,"error":{"code":"browser_action_failed","message":"the tab closed"}}\n' "$id" ;;"#,
    );
    let login = login_room_with_controller(fixture(), SITE, &controller).await;
    let f = &login.f;
    unlock(f);
    let handle = generate(f, "signup", SITE).await.unwrap()["credential_id"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(call(f, PASTE, paste_args(&handle)).await.is_err());
    let events: Vec<String> = f
        .state
        .home_runtime_secret_service()
        .unwrap()
        .vault_activity(&handle)
        .unwrap()
        .into_iter()
        .map(|row| row.event)
        .filter(|event| event.starts_with("Use"))
        .collect();
    assert_eq!(events.len(), 2, "{events:?}");
    assert!(events[0].starts_with("Use attempted in browser by "));
    assert_eq!(events[1], "Use failed: the action did not finish");
    assert!(!events.iter().any(|event| event.starts_with("Used")));
    let activity = serde_json::to_string(
        &f.state
            .home_runtime_secret_service()
            .unwrap()
            .vault_activity(&handle)
            .unwrap(),
    )
    .unwrap();
    assert!(!activity.contains(stored(f, &handle).as_str()));
    f.state.shutdown_cleanup().await.unwrap();
}

#[tokio::test]
async fn ordinary_totp_browser_fill_masks_the_computed_code() {
    crate::test_support::isolated_env_test!();
    let login = login_room().await;
    let f = &login.f;
    unlock(f);
    owner_factor(f);
    assert!(f.state.owned.sudo_turns.lock().unwrap().is_empty());
    let result = call(f, PASTE, paste_args("service-totp")).await.unwrap();
    assert_eq!(
        std::fs::read_to_string(login.root.path().join("factor_input"))
            .unwrap()
            .trim(),
        "true"
    );
    let code = zeroize::Zeroizing::new(inserted(&login).unwrap());
    assert!(code.len() == 6 && code.bytes().all(|c| c.is_ascii_digit()));
    assert!(!result.to_string().contains(code.as_str()));
    let snapshot = call(
        f,
        "chariox.kernel_browser",
        json!({"command":{"op":"snapshot","tab_id":"host-tab-new","generation":1}}),
    )
    .await
    .unwrap();
    assert!(!snapshot.to_string().contains(code.as_str()));
    assert!(!snapshot
        .to_string()
        .contains("GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ"));
    assert!(f.state.owned.sudo_turns.lock().unwrap().is_empty());
    f.state.shutdown_cleanup().await.unwrap();
}

#[tokio::test]
async fn totp_browser_fill_rechecks_prompt_lifetime_after_vault_unlock() {
    crate::test_support::isolated_env_test!();
    let login = login_room().await;
    let f = &login.f;
    unlock(f);
    owner_factor(f);
    crate::secret::lock_chariox_encrypted_vault(vault_path(f)).unwrap();
    let turn = elevated(f);
    let fill = call(f, PASTE, paste_args("service-totp"));
    let unlock = owner_unlocks(f, &turn.session_id, || {
        f.state
            .revoke_sudo(Some("local"), Some(&turn.entry_id), "explicit_revoke")
            .unwrap();
    });
    let (result, ()) = tokio::join!(fill, unlock);
    assert!(result.is_err());
    assert!(inserted(&login).is_none());
}

#[tokio::test]
async fn terminal_secret_rechecks_credential_kind_after_vault_unlock() {
    crate::test_support::isolated_env_test!();
    let f = fixture();
    unlock(&f);
    let registry = crate::credential::CharioxCredentialRegistry::user().unwrap();
    let mut credential = UserCredentialConfig {
        id: "changing-factor".into(),
        description: None,
        source: crate::config::UserCredentialSourceConfig::Vault {
            key: "changing-factor".into(),
        },
        allowed_hosts: vec![],
        allowed_uses: vec![crate::config::UserCredentialUse::Pty],
        injection: crate::config::UserCredentialInjectionConfig::Pty,
        metadata: None,
    };
    f.state
        .home_runtime_secret_service()
        .unwrap()
        .upsert_vault_backed_credential_with_secret(
            &registry,
            credential.clone(),
            "GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ",
            false,
        )
        .unwrap();
    crate::secret::lock_chariox_encrypted_vault(vault_path(&f)).unwrap();
    let send = call(
        &f,
        "chariox.send_secret_to_terminal",
        json!({"credential_id":"changing-factor","append_newline":false}),
    );
    let unlock = owner_unlocks(&f, &f.request.session_id, || {
        credential.injection = crate::config::UserCredentialInjectionConfig::Browser;
        registry.upsert(credential).unwrap();
    });
    let (result, ()) = tokio::join!(send, unlock);
    assert!(
        matches!(
            result,
            Err(DaemonError::LocalTransport {
                operation: "credential_policy",
                ..
            })
        ),
        "the current credential definition must be validated before terminal delivery"
    );
    f.state.shutdown_cleanup().await.unwrap();
}

const CAPTURE: &str = "chariox.vault.capture_browser";
const REMOVE: &str = "chariox.vault.remove_capture";
const FILL: &str = "chariox.vault.fill_field";
const CAPTURED: &str = "vaudit-captured-fixture-value";

fn capture_args(request_id: &str) -> Value {
    json!({"request_id": request_id, "tab_id": "host-tab-new", "generation": 1,
        "document_id": "doc-login", "node_ref": "backend:7", "reveal_node_ref": "backend:7",
        "label": "Stripe secret key"})
}

// VAUDIT S3: the kernel reveals, reads and protects a page value; the model gets a handle.
#[tokio::test]
async fn vaudit_s3_capture_browser_stores_and_protects_a_page_value() {
    crate::test_support::isolated_env_test!();
    let login = login_room().await;
    let f = &login.f;
    unlock(f);
    assert!(
        names(f).iter().any(|name| name == CAPTURE) && names(f).iter().any(|name| name == FILL)
    );
    // An authenticated ordinary agent captures without a sudo window.
    let result = call(f, CAPTURE, capture_args("sk")).await.unwrap();
    assert!(
        !result.to_string().contains(CAPTURED),
        "the model never gets the value"
    );
    let handle = result["credential_id"].as_str().unwrap().to_string();
    assert!(handle.starts_with("cap-"));
    assert_eq!(result["label"], "Stripe secret key");
    assert_eq!(result["use_on"], json!([SITE]));
    let mut multiple = capture_args("multiple-sites");
    multiple["use_on"] = json!([SITE, "https://other.example.com"]);
    assert!(call(f, CAPTURE, multiple).await.is_err());

    let request = std::fs::read_to_string(login.root.path().join("capture")).unwrap();
    for field in [
        r#""reveal_node_ref":"backend:7""#,
        r#""read":"node""#,
        r#""expected_document_url":"http://127.0.0.1:8123/login""#,
    ] {
        assert!(request.contains(field), "capture request lacks {field}");
    }
    assert_eq!(stored(f, &handle).as_str(), CAPTURED);
    let credential = crate::credential::CharioxCredentialRegistry::user()
        .unwrap()
        .get(&handle)
        .unwrap()
        .unwrap();
    assert_eq!(
        credential
            .metadata
            .as_ref()
            .unwrap()
            .use_binding
            .as_ref()
            .unwrap()
            .target,
        crate::config::VaultUseTarget::Site {
            origin: SITE.into()
        }
    );
    assert_eq!(
        credential.metadata.unwrap().created_by_kind.as_deref(),
        Some("vault_capture")
    );
    // Registered for masking before the barrier opened: value and page node.
    let scope = crate::runtime::kernel_browser_host::KernelBrowserHost::profile_key("local");
    let protection = &f.state.owned.kernel_browser_secret_observations;
    let policy: Value = serde_json::from_str(&protection.capture_policy(&scope).unwrap()).unwrap();
    assert!(policy["targets"]
        .as_array()
        .unwrap()
        .iter()
        .any(|target| target["node_ref"] == "backend:7"
            && target["value_hash"]
                == format!(
                    "{:x}",
                    <sha2::Sha256 as sha2::Digest>::digest(CAPTURED.as_bytes())
                )));
    assert_eq!(
        protection.scrub_text_or_withhold(&scope, &format!("key {CAPTURED}")),
        "key [redacted]"
    );
    // A retry returns the same handle without a second read.
    std::fs::remove_file(login.root.path().join("capture")).unwrap();
    let again = call(f, CAPTURE, capture_args("sk")).await.unwrap();
    assert_eq!(
        (again["credential_id"].as_str(), again["created"].as_bool()),
        (Some(handle.as_str()), Some(false))
    );
    assert!(!login.root.path().join("capture").exists());
}

// A value already in the Vault is protected in observations; capturing it again for another use
// must still store the page's value, not the redaction marker.
#[tokio::test]
async fn vflag_capture_of_an_already_protected_value_stores_the_value() {
    crate::test_support::isolated_env_test!();
    let login = login_room().await;
    let f = &login.f;
    unlock(f);
    let first = call(f, CAPTURE, capture_args("first")).await.unwrap();
    let second = call(f, CAPTURE, capture_args("second")).await.unwrap();
    let (first, second) = (
        first["credential_id"].as_str().unwrap().to_string(),
        second["credential_id"].as_str().unwrap().to_string(),
    );
    assert_ne!(first, second);
    assert_eq!(stored(f, &first).as_str(), CAPTURED);
    assert_eq!(stored(f, &second).as_str(), CAPTURED);
}

// The outer runtime-tool scrub must preserve the snapshot schema as well as the browser boundary.
#[tokio::test]
async fn vflag_capture_schema_label_preserves_the_next_snapshot_through_mcp() {
    crate::test_support::isolated_env_test!();
    let login = login_room().await;
    let f = &login.f;
    unlock(f);
    std::fs::write(login.root.path().join("value"), "text").unwrap();
    call(f, CAPTURE, capture_args("schema-label"))
        .await
        .unwrap();
    std::fs::write(login.root.path().join("echo"), "text").unwrap();
    let observed = call(
        f,
        "chariox.kernel_browser",
        json!({"command":{
        "op":"snapshot","tab_id":"host-tab-new","generation":1}}),
    )
    .await
    .unwrap();
    let snapshot: crate::runtime::browser_controller_snapshot::BrowserControllerStructuredSnapshot =
        serde_json::from_value(observed["snapshot"].clone()).expect("next snapshot remains usable");
    assert_eq!(snapshot.dom_nodes[0].text, "echo [redacted]");
    assert_eq!(snapshot.dom_nodes[0].node_ref, "backend:7");
}

// MP-08 / MP-09 / MP-10 / MP-11 review P1: distinct keys may own identical bytes.
#[tokio::test]
async fn vflag_remove_capture_preserves_the_same_value_owned_by_a_distinct_key() {
    crate::test_support::isolated_env_test!();
    // A plain field cannot rely on the browser's password-field masking.
    let controller = LOGIN_CONTROLLER
        .replace(r#""type":"password""#, r#""type":"text""#)
        .replace(
            " case \"$request\" in",
            r#" case "$request" in
  *'"method":"host.protect"'*) printf '%s' "$request" > "$1/protection"; printf '{"id":%s,"ok":true,"result":{}}\n' "$id" ;;"#,
        );
    let login = login_room_with_controller(fixture(), SITE, &controller).await;
    let f = &login.f;
    unlock(f);
    let first = call(f, CAPTURE, capture_args("equal-first")).await.unwrap();
    let second = call(f, CAPTURE, capture_args("equal-second"))
        .await
        .unwrap();
    let first = first["credential_id"].as_str().unwrap();
    let second = second["credential_id"].as_str().unwrap();
    let registry = crate::credential::CharioxCredentialRegistry::user().unwrap();
    assert_ne!(
        registry.get(first).unwrap().unwrap().source,
        registry.get(second).unwrap().unwrap().source
    );
    call(f, FILL, paste_args(second)).await.unwrap();
    assert_eq!(inserted(&login).as_deref(), Some(CAPTURED));
    assert!(std::fs::read_to_string(login.root.path().join("plain"))
        .unwrap()
        .contains(r#""plain_target":true"#));
    call(f, REMOVE, json!({"credential_id": first}))
        .await
        .unwrap();
    assert_eq!(stored(f, second).as_str(), CAPTURED);
    assert!(registry.get(first).unwrap().is_none());
    let observed = call(
        f,
        "chariox.kernel_browser",
        json!({"command": {
            "op": "snapshot", "tab_id": "host-tab-new", "generation": 1
        }}),
    )
    .await
    .unwrap();
    assert_eq!(
        observed["snapshot"]["dom_nodes"][0]["attributes"]["type"],
        "text"
    );
    assert_eq!(
        observed["snapshot"]["dom_nodes"][0]["text"], "echo [redacted]",
        "removing another key must not expose the retained entry through the outer MCP response"
    );
    let scope = crate::runtime::kernel_browser_host::KernelBrowserHost::profile_key("local");
    let policy: Value = serde_json::from_str(
        &f.state
            .owned
            .kernel_browser_secret_observations
            .capture_policy(&scope)
            .unwrap(),
    )
    .unwrap();
    let expected_hash = format!(
        "{:x}",
        <sha2::Sha256 as sha2::Digest>::digest(CAPTURED.as_bytes())
    );
    assert!(
        policy["targets"].as_array().unwrap().iter().any(|target| {
            target["kind"] == "browser"
                && target["target_id"] == "target-login"
                && target["document_id"] == "doc-login"
                && target["node_ref"] == "backend:7"
                && target["value_hash"] == expected_hash
        }),
        "the retained plain field still needs its exact model-frame mask"
    );
    // The next model observation must send those masks to the host, where
    // snapshots and frames share the protection policy.
    let delivered: Value = serde_json::from_str(
        &std::fs::read_to_string(login.root.path().join("protection")).unwrap(),
    )
    .unwrap();
    assert_eq!(delivered["params"]["targets"], policy["targets"]);
    assert_eq!(delivered["params"]["values"], policy["values"]);
    assert_eq!(
        f.state
            .owned
            .room_secret_observations
            .scrub_text_or_withhold(&f.request.session_id, CAPTURED),
        "[redacted]"
    );
    call(f, REMOVE, json!({"credential_id": second}))
        .await
        .unwrap();
    assert_eq!(
        f.state
            .owned
            .kernel_browser_secret_observations
            .scrub_text_or_withhold(&scope, CAPTURED),
        CAPTURED,
        "removing the last owner still undoes mistaken protection"
    );
}

// MP-08 / MP-09 / MP-10 / MP-11 review P2: the outer MCP scrub preserves control replies.
#[tokio::test]
async fn vflag_capture_schema_label_preserves_state_and_control_through_mcp() {
    crate::test_support::isolated_env_test!();
    let login = login_room().await;
    let f = &login.f;
    unlock(f);
    std::fs::write(login.root.path().join("value"), "Tabs").unwrap();
    call(f, CAPTURE, capture_args("state-schema-label"))
        .await
        .unwrap();
    for command in [
        json!({"op":"state"}),
        json!({"op":"open","url":SITE}),
        json!({"op":"navigate","tab_id":"host-tab-new","generation":1,"url":SITE}),
        json!({"op":"input","tab_id":"host-tab-new","generation":1,"input":{"kind":"key","key":"ArrowDown"}}),
    ] {
        let command_name = command["op"].as_str().unwrap().to_string();
        let arguments = if command["op"] == "input" {
            json!({"document_id":"doc-login", "command":command})
        } else {
            json!({"command":command})
        };
        let result = call(f, "chariox.kernel_browser", arguments).await.unwrap();
        assert_eq!(result["generation"], 1, "{} keeps generation", command_name);
        assert!(
            result["tabs"].is_array(),
            "capturing Tabs must preserve browser inventory through every boundary"
        );
        assert_eq!(result["tabs"][0]["tab_id"], "host-tab-new");
    }
    let agent = f.request.target_agent_id.as_deref().unwrap();
    f.state
        .owned
        .kernel_browser_host
        .set_focus("local", Some(agent));
    let stopped = call(
        f,
        "chariox.kernel_browser",
        json!({"command":{"op":"stop"}}),
    )
    .await
    .unwrap();
    assert_eq!(stopped["state"], "stopped");
    assert_eq!(stopped["tabs"], json!([]));
}

#[tokio::test]
async fn vflag_capture_inventory_schema_collisions_preserve_outer_runtime_results() {
    crate::test_support::isolated_env_test!();
    let controller = LOGIN_CONTROLLER
        .replace(
            r#""result":{"generation":1"#,
            r#""result":{"state":"ready","generation":1"#,
        )
        .replace(
            r#""tab_id":"host-tab-new","document_id":"doc-login""#,
            r#""tab_id":"host-tab-new","document_id":"doc-login","title":"Tabs generation ready host-tab-new""#,
        );
    let login = login_room_with_controller(fixture(), SITE, &controller).await;
    let f = &login.f;
    unlock(f);
    for (index, label) in ["Tabs", "generation", "ready", "host-tab-new"]
        .into_iter()
        .enumerate()
    {
        std::fs::write(login.root.path().join("value"), label).unwrap();
        call(
            f,
            CAPTURE,
            capture_args(&format!("inventory-schema-{index}")),
        )
        .await
        .unwrap();
    }
    for command in [
        json!({"op":"state"}),
        json!({"op":"open","url":SITE}),
        json!({"op":"navigate","tab_id":"host-tab-new","generation":1,"url":SITE}),
        json!({"op":"input","tab_id":"host-tab-new","generation":1,"input":{"kind":"key","key":"ArrowDown"}}),
    ] {
        let operation = command["op"].as_str().unwrap().to_string();
        let arguments = if operation == "input" {
            json!({"document_id":"doc-login", "command":command})
        } else {
            json!({"command":command})
        };
        let result = call(f, "chariox.kernel_browser", arguments).await.unwrap();
        assert_eq!(result["state"], "ready", "{operation} keeps its state enum");
        assert_eq!(result["generation"], 1, "{operation} keeps generation");
        let tabs = result["tabs"]
            .as_array()
            .expect("capturing Tabs preserves the fixed inventory key");
        let tab = tabs
            .iter()
            .find(|tab| tab["tab_id"] == "host-tab-new")
            .expect("the filled tab keeps its opaque identity");
        assert_eq!(tab["document_id"], "doc-login");
        assert_eq!(
            tab["title"], "[redacted] [redacted] [redacted] [redacted]",
            "{operation} still scrubs page-derived title text"
        );
    }
}

#[tokio::test]
async fn vflag_remove_capture_preserves_a_slot_referenced_by_an_alias() {
    crate::test_support::isolated_env_test!();
    let login = login_room().await;
    let f = &login.f;
    unlock(f);
    let captured = call(f, CAPTURE, capture_args("shared-slot")).await.unwrap();
    let id = captured["credential_id"].as_str().unwrap();
    let registry = crate::credential::CharioxCredentialRegistry::user().unwrap();
    let mut alias = registry.get(id).unwrap().unwrap();
    alias.id = "same-binding-alias".into();
    registry.upsert(alias).unwrap();

    let result = call(f, REMOVE, json!({"credential_id": id})).await;
    assert_eq!(stored(f, "same-binding-alias").as_str(), CAPTURED);
    assert!(result
        .unwrap_err()
        .to_string()
        .contains("Another entry uses this value"));
    assert!(registry.get(id).unwrap().is_some());
    let scope = crate::runtime::kernel_browser_host::KernelBrowserHost::profile_key("local");
    assert_eq!(
        f.state
            .owned
            .kernel_browser_secret_observations
            .scrub_text_or_withhold(&scope, CAPTURED),
        "[redacted]"
    );
}

// A label captured by mistake ("Token") is undone by the agent that made the capture: the entry goes,
// and the word is no longer masked in what the agent observes.
#[tokio::test]
async fn vflag_agent_removes_its_own_capture_and_the_value_is_no_longer_masked() {
    crate::test_support::isolated_env_test!();
    let login = login_room().await;
    let f = &login.f;
    unlock(f);
    std::fs::write(login.root.path().join("value"), "Token").unwrap();
    let wrong = call(f, CAPTURE, capture_args("label")).await.unwrap();
    let wrong = wrong["credential_id"].as_str().unwrap().to_string();
    let scope = crate::runtime::kernel_browser_host::KernelBrowserHost::profile_key("local");
    let protection = &f.state.owned.kernel_browser_secret_observations;
    assert_eq!(
        protection.scrub_text_or_withhold(&scope, "Token page"),
        "[redacted] page"
    );
    std::fs::write(login.root.path().join("value"), CAPTURED).unwrap();
    let kept = call(f, CAPTURE, capture_args("value")).await.unwrap();
    let kept = kept["credential_id"].as_str().unwrap().to_string();

    let removed = call(f, REMOVE, json!({"credential_id": wrong}))
        .await
        .unwrap();
    assert_eq!(removed["removed"][0]["credential_id"], wrong.as_str());
    assert_eq!(removed["removed"][0]["label"], "Stripe secret key");
    assert_eq!(removed["removed"][0]["length"], 5);
    assert!(
        !removed.to_string().contains("Token"),
        "the value is never returned"
    );
    assert_eq!(
        protection.scrub_text_or_withhold(&scope, "Token page"),
        "Token page"
    );
    assert_eq!(
        protection.scrub_text_or_withhold(&scope, &format!("key {CAPTURED}")),
        "key [redacted]",
        "the other capture stays masked"
    );
    let registry = crate::credential::CharioxCredentialRegistry::user().unwrap();
    assert!(registry.get(&wrong).unwrap().is_none());
    assert!(registry.get(&kept).unwrap().is_some());
    assert!(
        call(f, REMOVE, json!({"credential_id": wrong}))
            .await
            .is_err(),
        "already removed"
    );

    // Not a capture of this agent: an owner entry and a generated entry are refused.
    owner_credential(f, "owner-entry", &[SITE], None);
    let refused = call(f, REMOVE, json!({"credential_id": "owner-entry"}))
        .await
        .unwrap_err();
    assert!(
        refused
            .to_string()
            .contains("only a capture that this agent made"),
        "{refused}"
    );
    owner_credential(
        f,
        "other-agents-capture",
        &[SITE],
        Some(&f.request.session_id),
    );
    assert!(
        call(f, REMOVE, json!({"credential_id": "other-agents-capture"}))
            .await
            .is_err()
    );
}

// A page's manual setup key or QR code becomes a TOTP seed bound to the one site.
#[tokio::test]
async fn capture_browser_stores_a_page_totp_key_or_qr_as_a_site_bound_seed() {
    crate::test_support::isolated_env_test!();
    let login = login_room().await;
    let f = &login.f;
    unlock(f);
    std::fs::write(
        login.root.path().join("qr.b64"),
        crate::runtime::state::kernel_browser_qr::FIXTURE_QR_PNG,
    )
    .unwrap();
    let registry = crate::credential::CharioxCredentialRegistry::user().unwrap();
    let totp_args = |request_id: &str, node_ref: &str, read: &str| {
        let mut args = capture_args(request_id);
        args["node_ref"] = json!(node_ref);
        args["read"] = json!(read);
        args["totp"] = json!(true);
        args.as_object_mut().unwrap().remove("reveal_node_ref");
        args
    };
    let key = call(f, CAPTURE, totp_args("key", "backend:7", "node"))
        .await
        .unwrap();
    assert_eq!(key["totp"], true);
    assert!(
        !key.to_string().to_lowercase().contains("jbsw"),
        "the model never gets the seed"
    );
    let handle = key["credential_id"].as_str().unwrap().to_string();
    assert_code(f, &handle, 6, 30);
    let credential = registry.get(&handle).unwrap().unwrap();
    assert_eq!(
        credential.injection,
        crate::config::UserCredentialInjectionConfig::Totp(crate::config::TotpConfig::default())
    );
    assert_eq!(
        credential.metadata.unwrap().use_binding.unwrap().target,
        crate::config::VaultUseTarget::Totp {
            origin: SITE.into()
        }
    );
    let again = call(f, CAPTURE, totp_args("key", "backend:7", "node"))
        .await
        .unwrap();
    assert_eq!(
        (again["credential_id"].as_str(), again["created"].as_bool()),
        (Some(handle.as_str()), Some(false))
    );
    let mut plain = totp_args("key", "backend:7", "node");
    plain["totp"] = json!(false);
    assert!(
        call(f, CAPTURE, plain).await.is_err(),
        "a retry cannot turn a seed into a password"
    );

    let qr = call(f, CAPTURE, totp_args("qr", "backend:7", "qr"))
        .await
        .unwrap();
    let request = std::fs::read_to_string(login.root.path().join("capture")).unwrap();
    assert!(request.contains(r#""read":"qr""#), "{request}");
    let handle = qr["credential_id"].as_str().unwrap().to_string();
    assert_code(f, &handle, 8, 45);
    match registry.get(&handle).unwrap().unwrap().injection {
        crate::config::UserCredentialInjectionConfig::Totp(config) => {
            assert_eq!(
                (config.digits, config.period_s),
                (8, 45),
                "URI parameters are kept"
            );
        }
        other => panic!("{other:?}"),
    }

    // A value that is not an authenticator key stores nothing and is not echoed.
    let stored_before = crate::credential::load_user_credentials().unwrap().len();
    let mut wrong = totp_args("wrong", "backend:7", "node");
    wrong["reveal_node_ref"] = json!("backend:7");
    let error = call(f, CAPTURE, wrong).await.unwrap_err().to_string();
    assert!(
        error.contains("authenticator setup key") && !error.contains(CAPTURED),
        "{error}"
    );
    assert_eq!(
        crate::credential::load_user_credentials().unwrap().len(),
        stored_before
    );
}

#[tokio::test]
async fn reviewer_capture_browser_accepts_standalone_lowercase_and_padded_totp_keys() {
    crate::test_support::isolated_env_test!();
    let login = login_room().await;
    let f = &login.f;
    unlock(f);
    for (id, seed) in [
        ("lowercase-key", "abcdefghijklmnop".to_string()),
        ("padded-key", data_encoding::BASE32.encode(b"abcdefghijk")),
    ] {
        std::fs::write(login.root.path().join("value"), &seed).unwrap();
        let mut args = capture_args(id);
        args["totp"] = json!(true);
        args["read"] = json!("node");
        let captured = call(f, CAPTURE, args).await.unwrap();
        assert_eq!(captured["totp"], true);
        assert_eq!(captured["created"], true);
        assert!(!captured.to_string().contains(&seed));
        let handle = captured["credential_id"].as_str().unwrap();
        assert_eq!(stored(f, handle).len(), 6);
    }
}

/// The injected code is the RFC 6238 code of the fictional page seed (SHA-1, as the fixture URI says).
fn assert_code(f: &Fixture, handle: &str, digits: u32, period: u64) {
    use hmac::{Hmac, Mac};
    let seed = data_encoding::BASE32_NOPAD
        .decode(b"JBSWY3DPEHPK3PXP")
        .unwrap();
    let code_at = |seconds: u64| {
        let mut mac = Hmac::<sha1::Sha1>::new_from_slice(&seed).unwrap();
        mac.update(&(seconds / period).to_be_bytes());
        let digest = mac.finalize().into_bytes();
        let offset = usize::from(digest[19] & 15);
        let number =
            u32::from_be_bytes(digest[offset..offset + 4].try_into().unwrap()) & 0x7fff_ffff;
        format!(
            "{:0width$}",
            number % 10u32.pow(digits),
            width = digits as usize
        )
    };
    let now = || {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs()
    };
    let (before, code, after) = (now(), stored(f, handle), now());
    assert!(
        [code_at(before), code_at(after)].contains(&*code),
        "the Vault computes a code from the captured seed"
    );
}

// Browser capture can bind a page value to one command environment variable instead of a site.
#[tokio::test]
async fn capture_browser_binds_a_page_value_to_one_command_environment() {
    crate::test_support::isolated_env_test!();
    let login = login_room().await;
    let f = &login.f;
    unlock(f);
    let binding =
        json!({"target":{"kind":"command_environment","name":"RESEND_API_KEY","origin":SITE}});
    let mut args = capture_args("env");
    args["use_binding"] = binding.clone();
    let result = call(f, CAPTURE, args.clone()).await.unwrap();
    assert!(
        !result.to_string().contains(CAPTURED),
        "the model never gets the value"
    );
    let handle = result["credential_id"].as_str().unwrap().to_string();
    let service = f.state.home_runtime_secret_service().unwrap();
    let target = crate::config::VaultUseTarget::CommandEnvironment {
        name: "RESEND_API_KEY".into(),
        origin: SITE.into(),
    };
    assert_eq!(
        service
            .workspace_secret_input_for_target(&handle, &target)
            .unwrap(),
        CAPTURED
    );
    let other = crate::config::VaultUseTarget::CommandEnvironment {
        name: "OTHER_KEY".into(),
        origin: SITE.into(),
    };
    assert!(
        service
            .workspace_secret_input_for_target(&handle, &other)
            .is_err(),
        "one entry, one use"
    );
    assert!(
        service
            .browser_secret_input_for_target_url(&handle, &format!("{SITE}/login"))
            .is_err(),
        "a command-bound entry is not a site password"
    );
    let again = call(f, CAPTURE, args).await.unwrap();
    assert_eq!(
        (again["credential_id"].as_str(), again["created"].as_bool()),
        (Some(handle.as_str()), Some(false))
    );

    std::fs::remove_file(login.root.path().join("capture")).unwrap();
    let mut both = capture_args("both");
    both["use_binding"] = binding;
    both["use_on"] = json!([SITE]);
    assert!(
        call(f, CAPTURE, both).await.is_err(),
        "a site and a command use are exclusive"
    );
    let mut outside = capture_args("outside");
    outside["use_binding"] = json!({"target":{"kind":"file","path":"/etc/capture-target"}});
    assert!(
        call(f, CAPTURE, outside).await.is_err(),
        "a file outside the workspace is refused"
    );
    assert!(
        !login.root.path().join("capture").exists(),
        "a refused destination reveals nothing"
    );
}

// VAUDIT S3: fill_field puts a captured value into a plain field on an allowed origin only.
#[tokio::test]
async fn vaudit_s3_fill_field_fills_plain_targets_on_the_exact_origin() {
    crate::test_support::isolated_env_test!();
    let login = login_room().await;
    let f = &login.f;
    unlock(f);
    elevated(f);
    let handle = call(f, CAPTURE, capture_args("sk")).await.unwrap()["credential_id"]
        .as_str()
        .unwrap()
        .to_string();
    let mut https = capture_args("https-only");
    https["use_on"] = json!(["https://127.0.0.1:8123"]);
    let https = call(f, CAPTURE, https).await.unwrap()["credential_id"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(
        call(f, FILL, paste_args(&https)).await.is_err(),
        "a scheme downgrade is refused"
    );
    assert_eq!(inserted(&login), None);
    assert_eq!(
        call(f, FILL, paste_args(&handle)).await.unwrap(),
        json!({"inserted": true})
    );
    assert_eq!(inserted(&login).as_deref(), Some(CAPTURED));
    assert_eq!(
        std::fs::read_to_string(login.root.path().join("plain"))
            .unwrap()
            .trim(),
        r#""plain_target":true"#
    );
    // The ordinary paste keeps its password-only host contract.
    std::fs::remove_file(login.root.path().join("plain")).unwrap();
    call(f, PASTE, paste_args(&handle)).await.unwrap();
    assert!(std::fs::read_to_string(login.root.path().join("plain"))
        .unwrap()
        .is_empty());
}

#[tokio::test]
async fn vault_tools_and_resource_register_do_not_depend_on_the_sudo_window() {
    crate::test_support::isolated_env_test!();
    let f = fixture();
    unlock(&f);
    let turn = elevated(&f);
    f.state
        .owned
        .sudo_turns
        .lock()
        .unwrap()
        .remove(&turn.entry_id);
    generate(&f, "ordinary-after-revoke", SITE).await.unwrap();
    call(
        &f,
        crate::runtime::state::resource_register::LIST,
        json!({}),
    )
    .await
    .unwrap();
    assert_eq!(crate::credential::load_user_credentials().unwrap().len(), 1);
    f.state.shutdown_cleanup().await.unwrap();
}

#[tokio::test]
async fn vflag_capture_cursor_and_resize_schema_preserves_outer_runtime_results() {
    crate::test_support::isolated_env_test!();
    let controller = LOGIN_CONTROLLER.replace(
        r#"  *'"op":"stop"'*)"#,
        r#"  *'"op":"pointer_cursor"'*) printf '{"id":%s,"ok":true,"result":{"generation":1,"cursor":"pointer"}}\n' "$id" ;;
  *'"op":"resize"'*) printf '{"id":%s,"ok":true,"result":{"width":1280,"height":800}}\n' "$id" ;;
  *'"op":"stop"'*)"#,
    );
    let login = login_room_with_controller(fixture(), SITE, &controller).await;
    let f = &login.f;
    unlock(f);
    for (index, label) in ["generation", "cursor", "pointer", "width", "height"]
        .into_iter()
        .enumerate()
    {
        std::fs::write(login.root.path().join("value"), label).unwrap();
        call(
            f,
            CAPTURE,
            capture_args(&format!("cursor-resize-schema-{index}")),
        )
        .await
        .unwrap();
    }
    for (command, expected) in [
        (
            json!({"op":"pointer_cursor","tab_id":"host-tab-new","generation":1,"x":10,"y":10}),
            json!({"generation":1,"cursor":"pointer"}),
        ),
        (
            json!({"op":"resize","tab_id":"host-tab-new","generation":1,"width":1280,"height":800}),
            json!({"width":1280,"height":800}),
        ),
    ] {
        let operation = command["op"].as_str().unwrap().to_string();
        let result = call(f, "chariox.kernel_browser", json!({"command":command}))
            .await
            .unwrap();
        assert_eq!(result, expected, "{operation} preserves fixed metadata");
    }
    f.state.shutdown_cleanup().await.unwrap();
}

// MP-08 / MP-09 / MP-10 / MP-11: the owner types the passkey for a payment in the passkey popup
// every owner terminal gets. The card asked in the agent's Room is in its Room owner's popup feed,
// for as long as the card waits, and in no other owner's.
#[tokio::test]
async fn vopen_a_payment_card_is_its_room_owners_passkey_popup_while_it_waits() {
    use crate::local::KernelConnectionClass::Terminal;
    crate::test_support::isolated_env_test!();
    let login = card_room(SITE).await;
    let f = &login.f;
    unlock(f);
    let _turn = running(f);
    saved_card(f);
    asking_task(f);
    assert!(call(f, PASTE, card_fill("number", Some(&payment())))
        .await
        .is_err());
    let decision = card_decision(f).await;
    let prompts = f.state.passkey_prompts_for("local");
    let prompt = prompts
        .iter()
        .find(|prompt| prompt.interaction_id == decision.id())
        .expect("the owner's passkey popup shows the payment");
    assert_eq!(prompt.session_id, f.request.session_id);
    assert_eq!(prompt.approve_choice_id, "allow");
    let day_ms = 24 * 60 * 60 * 1000;
    let now_ms = crate::session::unix_epoch_ms();
    assert!(
        prompt.expires_at_ms > now_ms + day_ms - 60_000 && prompt.expires_at_ms <= now_ms + day_ms,
        "the popup lives as long as the card"
    );
    assert!(f.state.passkey_prompts_for("guest").is_empty());
    // The popup's typed passkey approves it, and the popup closes.
    allow_card(f, &prompt.interaction_id, Some(PASSKEY), None, Terminal)
        .await
        .unwrap();
    assert!(f.state.passkey_prompts_for("local").is_empty());
    call(f, PASTE, card_fill("number", Some(&payment())))
        .await
        .unwrap();
    assert_eq!(inserted(&login).as_deref(), Some(CARD_NUMBER));
}

// MP-08 / MP-09 / MP-10 / MP-11: what a payment card approves lives only in kernel memory. A card
// still waiting when the kernel restarts is withdrawn, so no Allow can approve nothing, and the
// agent's next fill asks again on one new card.
#[tokio::test]
async fn vopen_a_payment_card_waiting_at_a_restart_is_withdrawn_and_asked_again() {
    use super::super::wake_driver::Recovery;
    use crate::local::KernelConnectionClass::Terminal;
    crate::test_support::isolated_env_test!();
    let login = card_room(SITE).await;
    let f = &login.f;
    unlock(f);
    let _turn = running(f);
    saved_card(f);
    asking_task(f);
    assert!(call(f, PASTE, card_fill("number", Some(&payment())))
        .await
        .is_err());
    let decision = card_decision(f).await;
    let wake = f
        .state
        .owned
        .durable_state_store
        .agent_wake(decision.id())
        .unwrap()
        .unwrap();
    // The restarted kernel holds no responder and no payment ask; the wake ledger and card remain.
    f.state
        .owned
        .pending_interactions
        .write()
        .remove(decision.id());
    f.state.owned.vault_payment_asks.lock().unwrap().clear();
    let driver = f
        .state
        .owned
        .agent_wakes
        .drivers
        .get("interaction")
        .unwrap();
    let Recovery::Lost(reason) = driver.recover(&wake) else {
        panic!("a payment card reattached without what it approves");
    };
    assert!(reason.contains("fill the card again"), "{reason}");
    let now = crate::session::unix_epoch_ms();
    f.state
        .report_lost_wake(&wake, now, &format!("the kernel restarted: {reason}"))
        .unwrap();
    f.state.tick_interaction_wakes(now).unwrap();
    assert!(!card_pending(f));
    assert!(allow_card(f, decision.id(), Some(PASSKEY), None, Terminal)
        .await
        .is_err());
    assert!(inserted(&login).is_none());
    // Filling again asks again, on one new card.
    assert!(call(f, PASTE, card_fill("number", Some(&payment())))
        .await
        .is_err());
    let again = card_decision(f).await;
    assert_ne!(again.id(), decision.id());
    allow_card(f, again.id(), Some(PASSKEY), None, Terminal)
        .await
        .unwrap();
    call(f, PASTE, card_fill("number", Some(&payment())))
        .await
        .unwrap();
    assert_eq!(inserted(&login).as_deref(), Some(CARD_NUMBER));
}

/// How a payment card can be withdrawn before anyone answers it.
#[derive(Clone, Copy, Debug)]
enum PaymentWithdrawal {
    WakeCancelled,
    KernelRestarted,
    AgentStopped,
}

// MP-08 / MP-09 / MP-10 / MP-11: a payment card withdrawn before anyone answers it closes its
// passkey popup on every owner terminal at once, on each withdrawal path. Nothing else is asked,
// another open prompt of the owner stays, and another owner's feed hears nothing.
#[tokio::test]
async fn vopen_a_payment_card_whose_wake_is_cancelled_closes_its_passkey_popup() {
    crate::test_support::isolated_env_test!();
    a_withdrawn_payment_card_closes_the_popup(PaymentWithdrawal::WakeCancelled).await;
}

#[tokio::test]
async fn vopen_a_payment_card_withdrawn_at_a_restart_closes_its_passkey_popup() {
    crate::test_support::isolated_env_test!();
    a_withdrawn_payment_card_closes_the_popup(PaymentWithdrawal::KernelRestarted).await;
}

#[tokio::test]
async fn vopen_a_payment_card_of_a_stopped_agent_closes_its_passkey_popup() {
    crate::test_support::isolated_env_test!();
    a_withdrawn_payment_card_closes_the_popup(PaymentWithdrawal::AgentStopped).await;
}

async fn a_withdrawn_payment_card_closes_the_popup(withdrawal: PaymentWithdrawal) {
    use super::super::wake_driver::Recovery;
    use crate::local::KernelConnectionClass::Terminal;
    use crate::local::PasskeyPromptClosureReason;
    use crate::session::{RuntimeInteraction, RuntimeInteractionChoice};
    use crate::transport::kernel_protocol::KernelEvent;
    use crate::transport::passkey_prompt_feed::PasskeyPromptFeed;
    let login = card_room(SITE).await;
    let f = &login.f;
    unlock(f);
    let _turn = running(f);
    saved_card(f);
    asking_task(f);
    let mut owner = PasskeyPromptFeed::new(Terminal, "local");
    let mut stranger = PasskeyPromptFeed::new(Terminal, "guest");
    let _other = f
        .state
        .create_kernel_operation_interaction(
            &f.request.session_id,
            "local",
            RuntimeInteraction::for_kernel_operation(
                "other-critical",
                "validation:other-critical",
                "Approve App action",
                "An App asks to perform a protected action.",
                vec![
                    RuntimeInteractionChoice::new("deny", "Deny", "deny", None),
                    RuntimeInteractionChoice::new("approve", "Approve", "allow", None)
                        .requiring_passkey(),
                ],
            )
            .with_timeout_sec(300),
        )
        .await
        .unwrap();
    assert!(call(f, PASTE, card_fill("number", Some(&payment())))
        .await
        .is_err());
    let decision = card_decision(f).await;
    let ids = |event: Option<KernelEvent>| match event {
        Some(KernelEvent::PasskeyPromptsChanged {
            prompts, closed, ..
        }) => (
            prompts
                .into_iter()
                .map(|prompt| prompt.interaction_id)
                .collect::<Vec<_>>(),
            closed
                .into_iter()
                .map(|closure| (closure.interaction_id, closure.reason))
                .collect::<Vec<_>>(),
        ),
        other => panic!("{withdrawal:?}: expected a popup change, got {other:?}"),
    };
    let (shown, _) = ids(owner.next_event(&f.router));
    assert_eq!(shown, ["other-critical", decision.id()], "{withdrawal:?}");
    assert_eq!(ids(stranger.next_event(&f.router)), (vec![], vec![]));
    let wake = f
        .state
        .owned
        .durable_state_store
        .agent_wake(decision.id())
        .unwrap()
        .unwrap();
    match withdrawal {
        PaymentWithdrawal::WakeCancelled => {
            // The asking turn cancels it, the way chariox.events.cancel_wake does.
            let task = f
                .state
                .owned
                .durable_state_store
                .agent_tasks(Some(&f.request.session_id), None)
                .unwrap()
                .into_iter()
                .find(|task| task.task_id == wake.task_id)
                .unwrap();
            f.state
                .cancel_agent_wake(&wake, &task.task_id, Some(&task.prompt_id))
                .await
                .unwrap();
        }
        PaymentWithdrawal::KernelRestarted => {
            f.state
                .owned
                .pending_interactions
                .write()
                .remove(decision.id());
            f.state.owned.vault_payment_asks.lock().unwrap().clear();
            let driver = f
                .state
                .owned
                .agent_wakes
                .drivers
                .get("interaction")
                .unwrap();
            let Recovery::Lost(reason) = driver.recover(&wake) else {
                panic!("a payment card reattached without what it approves");
            };
            let now = crate::session::unix_epoch_ms();
            f.state.report_lost_wake(&wake, now, &reason).unwrap();
            f.state.tick_interaction_wakes(now).unwrap();
        }
        PaymentWithdrawal::AgentStopped => {
            f.state
                .owned
                .withdraw_agent_interactions(
                    &f.request.session_id,
                    f.request.target_agent_id.as_deref(),
                )
                .unwrap();
        }
    }
    assert!(!card_pending(f), "{withdrawal:?}");
    assert_eq!(
        ids(owner.next_event(&f.router)),
        (
            vec!["other-critical".to_owned()],
            vec![(
                decision.id().to_owned(),
                PasskeyPromptClosureReason::Revoked
            )]
        ),
        "{withdrawal:?}: the popup closes and nothing else is asked"
    );
    assert!(owner.next_event(&f.router).is_none(), "{withdrawal:?}");
    assert!(stranger.next_event(&f.router).is_none(), "{withdrawal:?}");
    assert!(allow_card(f, decision.id(), Some(PASSKEY), None, Terminal)
        .await
        .is_err());
    assert!(inserted(&login).is_none());
}

// A fill must not bypass the checkout commit gate by submitting its native form.
#[tokio::test]
async fn vopen_a_card_fill_cannot_submit_a_payment() {
    crate::test_support::isolated_env_test!();
    let login = card_room(SITE).await;
    let f = &login.f;
    unlock(f);
    let _turn = running(f);
    saved_card(f);
    let mut args = card_fill("number", Some(&payment()));
    args["submit"] = json!(true);
    let result = call(f, PASTE, args).await;
    assert!(result.unwrap_err().to_string().contains("checkout button"));
    assert!(
        !card_pending(f),
        "submitting a fill must not request approval"
    );
    assert!(inserted(&login).is_none());
}
