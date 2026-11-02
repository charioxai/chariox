//! MP-08/MP-10/MP-11 A06: elevated Vault generation and protected login through
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

#[tokio::test]
async fn a06_ordinary_agents_cannot_generate_credentials() {
    crate::test_support::isolated_env_test!();
    let f = fixture();
    unlock(&f);
    let catalog = names(&f);
    // MP-08/MP-10/MP-11: stable discovery is not a Vault generation grant.
    assert!(catalog.iter().any(|name| name == VAULT_GENERATE));
    assert!(
        !catalog
            .iter()
            .any(|name| name.contains("generated_credential")),
        "A06: generation is not an ordinary agent tool"
    );
    assert!(generate(&f, "signup", SITE).await.is_err());
    assert!(call(
        &f,
        "chariox.create_generated_credential",
        json!({"credential":{"id":"ordinary","allowed_hosts":["127.0.0.1:8123"],
            "allowed_uses":["browser"],"injection":{"kind":"browser"}}}),
    )
    .await
    .is_err());
    assert!(crate::credential::load_user_credentials()
        .unwrap()
        .is_empty());
}

#[tokio::test]
async fn a06_an_enrolled_kernel_owner_generates_and_other_users_cannot() {
    crate::test_support::isolated_env_test!();
    let f = fixture();
    let turn = elevated(&f);
    unlock(&f);
    // A Cloud-enrolled kernel's local owner is its Cloud user.
    let mut config = f.state.owned.config_projection.snapshot();
    config.cloud_relay = Some(crate::config::PersistedCloudRelayProfile {
        user_id: "cloud-owner".into(),
        ..Default::default()
    });
    f.state.owned.config_projection.update(config);
    let set_owner = |owner: &str| {
        let mut turns = f.state.owned.sudo_turns.lock().unwrap();
        turns.get_mut(&turn.entry_id).unwrap().owner_user_id = owner.into();
    };
    set_owner("collaborator");
    assert!(generate(&f, "other", SITE).await.is_err());
    set_owner("cloud-owner");
    let created = generate(&f, "signup", SITE).await.unwrap();
    assert_eq!(created["created"], true);
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
    assert!(generate(&f, "after-revoke", SITE).await.is_err());
}

#[tokio::test]
async fn a06_a_generated_handle_never_takes_a_slot_that_holds_a_value() {
    crate::test_support::isolated_env_test!();
    let f = fixture();
    let turn = elevated(&f);
    unlock(&f);
    let handle = super::vault::generated_handle(&turn, "signup");
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
    // Ending the window withdraws the prompt itself; nothing is left to answer.
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
async fn a06_generation_refuses_when_the_window_ends_during_the_vault_wait() {
    crate::test_support::isolated_env_test!();
    let f = fixture();
    let turn = elevated(&f);
    let pending = generate(&f, "signup", SITE);
    let unlock = owner_unlocks(&f, &turn.session_id, || {
        f.state
            .revoke_sudo(Some("local"), Some(&turn.entry_id), "explicit_revoke")
            .unwrap();
    });
    let (result, ()) = tokio::join!(pending, unlock);
    assert!(
        result.is_err(),
        "A06: no value may be minted after the window ends"
    );
    assert!(crate::credential::load_user_credentials()
        .unwrap()
        .is_empty());
}

const LOGIN_CONTROLLER: &str = r#"set -eu
while IFS= read -r request; do
 id=${request#*:}; id=${id%%,*}
 tabs='{"tab_id":"host-tab-fixture","document_id":"document"}'
 [ -f "$1/opened" ] && tabs="$tabs"',{"tab_id":"host-tab-new","document_id":"doc-login"}'
 case "$request" in
  *'"method":"health"'*) printf '{"id":%s,"ok":true,"result":{"state":"ready","process_id":%s,"diagnostic_code":null}}\n' "$id" "$$" ;;
  *'"method":"host.secret"'*'"op":"capture"'*) printf '%s' "$request" > "$1/capture"; printf '{"id":%s,"ok":true,"result":{"value":"vaudit-captured-fixture-value"}}\n' "$id" ;;
  *'"method":"host.secret"'*) printf '%s' "$request" | grep -o '"plain_target":true' > "$1/plain" || true; printf '%s' "$request" | sed -n 's/.*"text":"\([^"]*\)".*/\1/p' > "$1/echo"; printf '%s' "$request" | sed -n 's/.*"vault_factor_input":\(true\|false\).*/\1/p' > "$1/factor_input"; printf '%s' "$request" | sed -n 's/.*"_app_view_tab":"\([^"]*\)".*/\1/p' > "$1/app_view_tab"; printf '{"id":%s,"ok":true,"result":{}}\n' "$id" ;;
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
async fn a06_sudo_window_fills_a_retained_target_and_ordinary_agents_are_refused() {
    crate::test_support::isolated_env_test!();
    let login = login_room().await;
    let f = &login.f;
    unlock(f);
    // An unfocused holder of the grant is not refused for lack of focus.
    let ordinary = call(f, PASTE, paste_args("gen-unknown")).await.unwrap_err();
    assert!(!needs_focus(ordinary));
    let turn = elevated(f);
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
    owner_credential(
        f,
        "foreign-generated",
        &["127.0.0.1:8123"],
        Some("other-session"),
    );
    owner_credential(f, "unbound-owner", &[], None);
    for (refused, reason) in [
        (elsewhere.as_str(), "another site"),
        ("foreign-generated", "another session"),
    ] {
        assert!(
            call(f, PASTE, paste_args(refused)).await.is_err(),
            "A06: {reason} must be refused"
        );
        assert_eq!(inserted(&login), None, "A06: {reason} reached the page");
    }
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
    f.state
        .revoke_sudo(Some("local"), Some(&turn.entry_id), "explicit_revoke")
        .unwrap();
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

// Poll the real generation future into its contended storage wait before ending sudo.
async fn queued_generation_loses_authority(expire: bool) {
    let f = fixture();
    let turn = elevated(&f);
    unlock(&f);
    let before = std::fs::read(vault_path(&f)).unwrap();
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
    let error = pending
        .await
        .expect_err("generation committed after original sudo authority ended");
    assert!(
        error.to_string().contains("sudo"),
        "storage must reject the ended sudo authority"
    );
    assert!(crate::credential::load_user_credentials()
        .unwrap()
        .is_empty());
    assert!(
        std::fs::read(vault_path(&f)).unwrap() == before,
        "Vault storage changed after authority ended"
    );
    f.state.shutdown_cleanup().await.unwrap();
}

#[tokio::test]
async fn a06_generation_storage_wait_rechecks_revoked_window() {
    crate::test_support::isolated_env_test!();
    queued_generation_loses_authority(false).await;
}

#[tokio::test]
async fn a06_generation_storage_wait_rechecks_expired_window() {
    crate::test_support::isolated_env_test!();
    queued_generation_loses_authority(true).await;
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
async fn totp_browser_fill_requires_live_elevation_and_masks_the_computed_code() {
    crate::test_support::isolated_env_test!();
    let login = login_room().await;
    let f = &login.f;
    unlock(f);
    owner_factor(f);
    assert!(call(f, PASTE, paste_args("service-totp")).await.is_err());
    assert!(inserted(&login).is_none());
    let turn = elevated(f);
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
    f.state
        .revoke_sudo(Some("local"), Some(&turn.entry_id), "explicit_revoke")
        .unwrap();
    assert!(call(f, PASTE, paste_args("service-totp")).await.is_err());
}

#[tokio::test]
async fn totp_browser_fill_rechecks_elevation_after_vault_unlock() {
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
async fn totp_terminal_rechecks_credential_kind_after_vault_unlock() {
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
        credential.injection = crate::config::UserCredentialInjectionConfig::Totp(
            crate::config::TotpConfig::default(),
        );
        registry.upsert(credential).unwrap();
    });
    let (result, ()) = tokio::join!(send, unlock);
    assert!(
        matches!(
            result,
            Err(DaemonError::LocalTransport {
                operation: "vault_elevation",
                ..
            })
        ),
        "the current factor definition must require elevation before terminal delivery"
    );
    f.state.shutdown_cleanup().await.unwrap();
}

const CAPTURE: &str = "chariox.vault.capture_browser";
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
    let ordinary = call(f, CAPTURE, capture_args("sk")).await.unwrap_err();
    assert!(ordinary.to_string().contains("requires a live sudo window"));
    assert!(
        !login.root.path().join("capture").exists(),
        "no reveal without elevation"
    );
    elevated(f);
    let result = call(f, CAPTURE, capture_args("sk")).await.unwrap();
    assert!(
        !result.to_string().contains(CAPTURED),
        "the model never gets the value"
    );
    let handle = result["credential_id"].as_str().unwrap().to_string();
    assert!(handle.starts_with("cap-"));
    assert_eq!(result["label"], "Stripe secret key");
    assert_eq!(result["use_on"], json!([SITE]));
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
