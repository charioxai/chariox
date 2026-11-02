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

fn names(f: &Fixture) -> Vec<String> {
    f.router
        .runtime_tool_specs_for_auth_token(TOKEN)
        .into_iter()
        .map(|spec| spec.name)
        .collect()
}

async fn call(f: &Fixture, tool: &str, arguments: Value) -> Result<Value, DaemonError> {
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

fn unlock(f: &Fixture) {
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
fn elevated(f: &Fixture) -> KernelSudoTurn {
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
  *'"method":"host.secret"'*) printf '%s' "$request" | sed -n 's/.*"text":"\([^"]*\)".*/\1/p' > "$1/echo"; printf '%s' "$request" | sed -n 's/.*"_app_view_tab":"\([^"]*\)".*/\1/p' > "$1/app_view_tab"; printf '{"id":%s,"ok":true,"result":{}}\n' "$id" ;;
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
