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

#[tokio::test]
async fn a06_ordinary_agents_cannot_generate_credentials() {
    crate::test_support::isolated_env_test!();
    let f = fixture();
    unlock(&f);
    let catalog = names(&f);
    assert!(!catalog.iter().any(|name| name == VAULT_GENERATE));
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
    assert!(!names(&f).iter().any(|name| name == VAULT_GENERATE));
    assert!(generate(&f, "after-revoke", SITE).await.is_err());
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
    // The default TTL policy then asks how long to stay unlocked.
    let lease = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            if let Some(id) = pending("vault-unlock-lease-") {
                return id;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await;
    if let Ok(lease) = lease {
        f.state
            .resolve_terminal_runtime_interaction(
                session,
                &lease,
                "unlock_operation",
                None,
                Some("local"),
            )
            .await
            .unwrap();
    }
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
  *'"method":"host.secret"'*) printf '%s' "$request" | sed -n 's/.*"text":"\([^"]*\)".*/\1/p' > "$1/echo"; printf '{"id":%s,"ok":true,"result":{}}\n' "$id" ;;
  *'"method":"host.'*'"op":"open"'*) : > "$1/opened"; printf '{"id":%s,"ok":true,"result":{"generation":1,"tab_id":"host-tab-new","tabs":[%s,{"tab_id":"host-tab-new","document_id":"doc-login"}]}}\n' "$id" "$tabs" ;;
  *'"op":"state"'*|*'"op":"start"'*) printf '{"id":%s,"ok":true,"result":{"generation":1,"tabs":[%s]}}\n' "$id" "$tabs" ;;
  *'"op":"snapshot"'*) echo=$(cat "$1/echo" 2>/dev/null || true); printf '{"id":%s,"ok":true,"result":{"snapshot":{"browser_generation":1,"target_id":"target-login","document_id":"doc-login","snapshot_revision":1,"accessibility_nodes":[],"dom_documents":[{"document_index":0,"url":"http://127.0.0.1:8123/login","owner_node_ref":null}],"shadow_roots":[],"dom_nodes":[{"node_ref":"backend:7","parent_ref":null,"document_index":0,"node_type":1,"node_name":"INPUT","text":"echo %s","attributes":{"type":"password","id":"password"},"bounds":null}]}}}\n' "$id" "$echo" ;;
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
    let f = fixture();
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
    // Ordinary, unfocused holder of the same grant: refused before any lookup.
    let ordinary = call(f, PASTE, paste_args("gen-unknown")).await.unwrap_err();
    assert!(needs_focus(ordinary));
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
        ("unbound-owner", "unbound credential"),
    ] {
        assert!(
            call(f, PASTE, paste_args(refused)).await.is_err(),
            "A06: {reason} must be refused"
        );
        assert_eq!(inserted(&login), None, "A06: {reason} reached the page");
    }
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
    let after = call(f, PASTE, paste_args(&handle)).await.unwrap_err();
    assert!(needs_focus(after));
    assert_eq!(inserted(&login), None);
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
    assert!(
        pending.await.is_err(),
        "generation committed after original sudo authority ended"
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
