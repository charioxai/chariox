//! MP-08 / MP-09 / MP-10 / MP-11: ordinary Vault generation and protected login through
//! the real MCP router. The browser is a fixture controller that records what
//! the kernel physically inserted; live Chromium/provider acceptance is separate.
use super::tests::{fixture, running, Fixture, PASSKEY};
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
async fn owner_unlocks(f: &Fixture, session: &str, during: impl FnOnce()) {
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
    for (asked, expected) in [(None, 280), (Some(900), 280), (Some(30), 30)] {
        let f = fixture();
        let mut arguments = json!({"message": "Open the page", "choices": [{"id": "done", "label": "Done", "reply": "Done"}]});
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
        assert_eq!(cards[0].timeout_sec(), Some(expected), "asked {asked:?}");
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
  *'"op":"state"'*|*'"op":"start"'*) printf '{"id":%s,"ok":true,"result":{"generation":1,"tabs":[%s]}}\n' "$id" "$tabs" ;;
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
    let root = crate::test_support::TestWorktree::new("a06-login-controller");
    let script = root.path().join("controller.sh");
    std::fs::write(&script, LOGIN_CONTROLLER.replace(SITE, site)).unwrap();
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
    let login = login_room().await;
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
        let answer = async {
            let id = tokio::time::timeout(Duration::from_secs(2), async {
                loop {
                    let id = f
                        .state
                        .owned
                        .pending_interactions
                        .write()
                        .values()
                        .filter_map(|pending| pending.user_domain_interaction.as_ref())
                        .find(|interaction| interaction.id().starts_with("vault-card-"))
                        .map(|interaction| interaction.id().to_owned());
                    if let Some(id) = id {
                        break id;
                    }
                    tokio::task::yield_now().await;
                }
            })
            .await
            .unwrap();
            f.state
                .resolve_terminal_runtime_interaction(
                    "",
                    &id,
                    if allow { "allow" } else { "cancel" },
                    None,
                    Some("local"),
                )
                .await
                .unwrap();
        };
        let fill = call(f, PASTE, paste_args("test-card"));
        let (result, ()) = tokio::join!(fill, answer);
        assert_eq!(result.is_ok(), allow);
        assert_eq!(inserted(&login).is_some(), allow);
        let _ = std::fs::remove_file(login.root.path().join("echo"));
    }
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
    let f = fixture();
    unlock(&f);
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
    let answer = async {
        let id = tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                let id = f
                    .state
                    .owned
                    .pending_interactions
                    .write()
                    .values()
                    .filter_map(|pending| pending.user_domain_interaction.as_ref())
                    .find(|interaction| interaction.id().starts_with("vault-card-"))
                    .map(|interaction| interaction.id().to_owned());
                if let Some(id) = id {
                    break id;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        f.state
            .owned
            .provider_store
            .mark_run_ended_provider_only(&f.request.session_id, f.run.id())
            .unwrap();
        f.state
            .resolve_terminal_runtime_interaction("", &id, "allow", None, Some("local"))
            .await
            .unwrap();
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
    let request = call(
        &f,
        "chariox.http_request_with_credential",
        json!({"credential_id":"http-card", "url":origin}),
    );
    let (result, (), sent) = tokio::join!(request, answer, receive);
    assert!(
        !sent,
        "ended provider run sent a credential-authenticated HTTP request"
    );
    assert!(result.is_err());
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
