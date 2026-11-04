//! MP-08 / MP-09 / MP-10 / MP-11 AU-15: use the authenticated MCP entry point.
use super::tests::{fixture, running, Fixture, PASSKEY};
use super::*;
use serde_json::{json, Value};

const TOOL: &str = "chariox.agent_auth.register";
const TOKEN: &str = "sudo-fixture-bearer";

#[tokio::test]
async fn au15_agent_auth_is_discoverable_before_elevation() {
    crate::test_support::isolated_env_test!();
    let f = fixture();
    let specs = f.router.runtime_tool_specs_for_auth_token(TOKEN);
    let spec = specs.iter().find(|spec| spec.name == TOOL).expect(
        "auth.md registration must be discoverable by official harnesses before sudo starts",
    );
    assert!(spec.input_schema["properties"]["resource_url"].is_object());
    assert!(spec.input_schema["properties"]["email"].is_object());
}

async fn call(f: &Fixture, args: Value) -> Result<Value, DaemonError> {
    Box::pin(
        f.router
            .dispatch_authenticated_runtime_tool_call(TOKEN, TOOL, args),
    )
    .await
    .map(|result| result.payload)
}

#[tokio::test]
async fn au15_ended_agent_is_refused_before_any_network_or_vault_write() {
    crate::test_support::isolated_env_test!();
    let f = fixture();
    f.state
        .owned
        .provider_store
        .mark_run_ended_provider_only(f.run.session_id(), f.run.id())
        .unwrap();
    let result = call(
        &f,
        json!({"request_id":"denied", "resource_url":"http://127.0.0.1:1"}),
    )
    .await;
    assert!(result
        .unwrap_err()
        .to_string()
        .contains("authenticated provider run"));
    assert!(crate::credential::load_user_credentials()
        .unwrap()
        .is_empty());
}
// The service below speaks real HTTP, including discovery and form-encoded grants.
// It supplements the live drill; it does not claim public-service acceptance.
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

struct Service {
    origin: String,
    secret: String,
    assertion: String,
    claim_token: String,
    claimed: Arc<AtomicBool>,
    registrations: Arc<AtomicUsize>,
    polls: Arc<AtomicUsize>,
    mode: Arc<std::sync::Mutex<&'static str>>,
    task: tokio::task::JoinHandle<()>,
}
impl Drop for Service {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl Service {
    async fn start() -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        let secret = format!("local-access-{:032x}", rand::random::<u128>());
        let assertion = format!("local-assertion-{:032x}", rand::random::<u128>());
        let claim_token = format!("local-claim-{:032x}", rand::random::<u128>());
        let claimed = Arc::new(AtomicBool::new(false));
        let registrations = Arc::new(AtomicUsize::new(0));
        let polls = Arc::new(AtomicUsize::new(0));
        let mode = Arc::new(std::sync::Mutex::new("standard"));
        let (host, key, assertion_value, claim_value, is_claimed, counts, poll_count, setting) = (
            origin.clone(),
            secret.clone(),
            assertion.clone(),
            claim_token.clone(),
            claimed.clone(),
            registrations.clone(),
            polls.clone(),
            mode.clone(),
        );
        let task = tokio::spawn(async move {
            while let Ok((mut stream, _)) = listener.accept().await {
                let mut bytes = Vec::new();
                let mut chunk = [0; 4096];
                let headers_end;
                loop {
                    let length = stream.read(&mut chunk).await.unwrap();
                    if length == 0 {
                        return;
                    }
                    bytes.extend_from_slice(&chunk[..length]);
                    if let Some(end) = bytes.windows(4).position(|value| value == b"\r\n\r\n") {
                        headers_end = end + 4;
                        break;
                    }
                    assert!(bytes.len() < 65536);
                }
                let headers = String::from_utf8(bytes[..headers_end].to_vec()).unwrap();
                let length: usize = headers
                    .lines()
                    .find_map(|line| {
                        line.to_ascii_lowercase()
                            .strip_prefix("content-length:")
                            .map(|value| value.trim().parse().unwrap())
                    })
                    .unwrap_or(0);
                while bytes.len() < headers_end + length {
                    let count = stream.read(&mut chunk).await.unwrap();
                    if count == 0 {
                        break;
                    }
                    bytes.extend_from_slice(&chunk[..count]);
                }
                let body = String::from_utf8(bytes[headers_end..].to_vec()).unwrap();
                let path = headers
                    .lines()
                    .next()
                    .unwrap()
                    .split_whitespace()
                    .nth(1)
                    .unwrap();
                let mode = *setting.lock().unwrap();
                let mut status = 200;
                let mut extra = String::new();
                let response = match path {
                    "/.well-known/oauth-protected-resource" => {
                        json!({"resource":if mode=="wrong_resource" {"https://other.invalid"} else {&host},"authorization_servers":[host],"scopes_supported":["api.read"],"bearer_methods_supported":["header"]})
                    }
                    "/.well-known/oauth-authorization-server" => {
                        json!({"issuer":host,"token_endpoint":format!("{host}/oauth2/token"),"agent_auth":{"skill":format!("{host}/auth.md"),"identity_endpoint":format!("{host}/agent/identity"),"claim_endpoint":format!("{host}/agent/identity/claim"),"identity_types_supported": if mode=="identity_only" {vec!["identity_assertion"]} else {vec!["anonymous","service_auth"]}}})
                    }
                    "/auth.md" => json!("# auth.md: use the discovered identity endpoint"),
                    "/agent/identity" => {
                        counts.fetch_add(1, Ordering::SeqCst);
                        let request: Value = serde_json::from_str(&body).unwrap();
                        if request["type"] == "service_auth" {
                            if mode == "missing_claim" {
                                json!({"registration_id":"reg-local","claim_token":claim_value})
                            } else {
                                json!({"registration_id":"reg-local","claim_token":claim_value,"claim":{"user_code":"123456","verification_uri":format!("{host}/claim"),"expires_in":if mode=="expiry" {1} else {60},"interval":if mode=="ended_long" {30} else {1}}})
                            }
                        } else if mode == "direct" {
                            json!({"registration_id":"reg-local","credential_type":"api_key","credential":key})
                        } else {
                            json!({"registration_id":"reg-local","identity_assertion":assertion_value,"claim_token":claim_value,"pre_claim_scopes":["api.read"],"project":{"id":"proj-1","expires_at":"2099-01-01T00:00:00Z","branch":{"id":"br-1"}}})
                        }
                    }
                    "/agent/identity/claim" => {
                        json!({"claim_attempt":{"user_code":"123456","verification_uri":format!("{host}/claim"),"expires_in":30,"interval":1}})
                    }
                    "/oauth2/token" => {
                        let form: BTreeMap<_, _> = url::form_urlencoded::parse(body.as_bytes())
                            .into_owned()
                            .collect();
                        if mode == "redirect" {
                            status = 307;
                            extra = format!("Location: {host}/must-not-forward\r\n");
                            json!({})
                        } else if mode == "error_echo" {
                            status = 400;
                            json!({"error":key,"error_description":assertion_value})
                        } else if form
                            .get("grant_type")
                            .is_some_and(|value| value.ends_with(":claim"))
                        {
                            assert!(form.get("claim_token") == Some(&claim_value));
                            poll_count.fetch_add(1, Ordering::SeqCst);
                            if is_claimed.load(Ordering::SeqCst) {
                                json!({"access_token":key,"token_type":"Bearer","scope":"api.read"})
                            } else {
                                status = 400;
                                json!({"error":"authorization_pending"})
                            }
                        } else {
                            assert!(form.get("assertion") == Some(&assertion_value));
                            assert!(form
                                .get("resource")
                                .is_some_and(|resource| resource.trim_end_matches('/') == host));
                            json!({"access_token":key,"token_type":"Bearer","scope":"api.read"})
                        }
                    }
                    "/api/check" => {
                        let accepted = headers.lines().any(|line| {
                            line.strip_prefix("authorization: ")
                                .or_else(|| line.strip_prefix("Authorization: "))
                                .is_some_and(|value| value == format!("Bearer {key}"))
                        });
                        if !accepted {
                            status = 401;
                        }
                        json!({"accepted":accepted,"echo":key})
                    }
                    _ => {
                        status = 404;
                        json!({"error":"not_found"})
                    }
                };
                let body = serde_json::to_vec(&response).unwrap();
                let header = format!("HTTP/1.1 {status} Test\r\nContent-Type: application/json\r\n{extra}Content-Length: {}\r\nConnection: close\r\n\r\n", body.len());
                stream.write_all(header.as_bytes()).await.unwrap();
                stream.write_all(&body).await.unwrap();
            }
        });
        Self {
            origin,
            secret,
            assertion,
            claim_token,
            claimed,
            registrations,
            polls,
            mode,
            task,
        }
    }
    fn mode(&self, mode: &'static str) {
        *self.mode.lock().unwrap() = mode;
    }
    fn absent(&self, value: &str) {
        assert!(
            !value.contains(&self.secret),
            "access credential crossed the model boundary"
        );
        assert!(
            !value.contains(&self.assertion),
            "identity assertion crossed the model boundary"
        );
        assert!(
            !value.contains(&self.claim_token),
            "claim token crossed the model boundary"
        );
    }
}

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
    let vault = f
        .state
        .owned
        .config_projection
        .snapshot()
        .user_config
        .credential_vault
        .path;
    crate::secret::unlock_chariox_encrypted_vault(
        vault,
        PASSKEY,
        crate::secret::VaultUnlockLease::KernelShutdown,
    )
    .unwrap();
    turn
}

async fn claim_card(f: &Fixture) -> crate::session::RuntimeInteraction {
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            if let Some(card) = f
                .state
                .owned
                .session_store
                .get_session(&f.request.session_id)
                .unwrap()
                .active_interactions()
                .iter()
                .find(|card| card.id().starts_with("agent-auth-claim-"))
                .cloned()
            {
                return card;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("one kernel-owned claim interaction")
}

#[tokio::test]
async fn au15_anonymous_vault_credential_works_and_retry_does_not_register_twice() {
    crate::test_support::isolated_env_test!();
    let f = fixture();
    elevated(&f);
    let service = Service::start().await;
    let args =
        json!({"request_id":"anonymous", "resource_url":service.origin,"scopes":["api.read"]});
    let created = call(&f, args.clone()).await.unwrap();
    assert_eq!(created.as_object().unwrap().len(), 2);
    service.absent(&created.to_string());
    let handle = created["credential_id"].as_str().unwrap();
    let http = Box::pin(f.router.dispatch_authenticated_runtime_tool_call(
        TOKEN,
        "http_request_with_credential",
        json!({"credential_id":handle,"url":format!("{}/api/check",service.origin),"method":"GET"}),
    ))
    .await
    .unwrap()
    .payload;
    assert_eq!(http["status"], 200);
    service.absent(&http.to_string());
    for other in [
        format!(
            "{}/api/check",
            service.origin.replace("127.0.0.1", "localhost")
        ),
        "http://127.0.0.1:1/api/check".into(),
        format!("{}/api/check", service.origin.replace("http:", "https:")),
    ] {
        assert!(
            Box::pin(f.router.dispatch_authenticated_runtime_tool_call(
                TOKEN,
                "http_request_with_credential",
                json!({"credential_id":handle,"url":other,"method":"GET"})
            ))
            .await
            .is_err(),
            "auth.md handles must authorize only the canonical resource origin"
        );
    }
    assert_eq!(created, call(&f, args).await.unwrap());
    assert_eq!(service.registrations.load(Ordering::SeqCst), 1);
    let credentials = crate::credential::load_user_credentials().unwrap();
    assert_eq!(credentials.len(), 1);
    assert_eq!(
        credentials[0]
            .metadata
            .as_ref()
            .unwrap()
            .created_by_kind
            .as_deref(),
        Some("agent_auth")
    );
    assert_eq!(
        credentials[0].allowed_hosts,
        std::slice::from_ref(&service.origin)
    );
    assert_eq!(
        credentials[0]
            .metadata
            .as_ref()
            .unwrap()
            .use_binding
            .as_ref()
            .unwrap()
            .target,
        crate::config::VaultUseTarget::Site {
            origin: service.origin.clone()
        }
    );

    assert_eq!(
        credentials[0].allowed_uses,
        [crate::config::UserCredentialUse::Http]
    );
}

#[tokio::test]
async fn au15_service_auth_has_one_owner_card_and_polls_until_claimed() {
    crate::test_support::isolated_env_test!();
    let f = fixture();
    elevated(&f);
    let service = Service::start().await;
    let args =
        json!({"request_id":"claim", "resource_url":service.origin,"email":"owner@example.test"});
    let work = call(&f, args.clone());
    tokio::pin!(work);
    let card = tokio::select! { card=claim_card(&f)=>card, _=&mut work=>panic!("Service Auth completed without a claim") };
    assert!(card.kernel_operation_id().is_some());
    assert!(card.message().contains("123456"));
    service.absent(&serde_json::to_string(&card).unwrap());
    let duplicate = call(&f, args).await.unwrap_err();
    assert!(duplicate.to_string().contains("already running"));
    assert!(crate::credential::load_user_credentials()
        .unwrap()
        .is_empty());
    // The service must confirm ownership; an agent cannot answer the owner card.
    assert!(f
        .state
        .resolve_runtime_interaction(&f.request.session_id, card.id(), "cancel", None)
        .await
        .is_err());
    tokio::select! { _=tokio::time::sleep(Duration::from_millis(1200))=>{}, _=&mut work=>panic!("claim completed before service confirmation") };
    assert!(service.polls.load(Ordering::SeqCst) >= 1);
    service.claimed.store(true, Ordering::SeqCst);
    let value = work.await.unwrap();
    service.absent(&value.to_string());
    assert_eq!(service.registrations.load(Ordering::SeqCst), 1);
    assert!(f
        .state
        .owned
        .session_store
        .get_session(&f.request.session_id)
        .unwrap()
        .active_interactions()
        .is_empty());
}

#[tokio::test]
async fn au15_claim_cancel_expiry_and_lost_authority_never_store() {
    crate::test_support::isolated_env_test!();
    for mode in ["cancel", "expiry", "ended", "ended_long"] {
        let f = fixture();
        let turn = elevated(&f);
        let service = Service::start().await;
        service.mode(mode);
        let args =
            json!({"request_id":mode,"resource_url":service.origin,"email":"owner@example.test"});
        let work = call(&f, args);
        tokio::pin!(work);
        let card = tokio::select! {card=claim_card(&f)=>card, _=&mut work=>panic!("no claim card")};
        if mode == "cancel" {
            f.state
                .answer_terminal_runtime_interaction(
                    &f.request.session_id,
                    card.id(),
                    "cancel",
                    None,
                    Some("local"),
                    None,
                    None,
                    Some(KernelConnectionClass::Terminal),
                )
                .await
                .unwrap();
        }
        if mode.starts_with("ended") {
            f.state
                .owned
                .provider_store
                .mark_run_ended_provider_only(&turn.session_id, f.run.id())
                .unwrap();
        }
        let failure = tokio::time::timeout(Duration::from_secs(5), work)
            .await
            .unwrap()
            .unwrap_err();
        service.absent(&failure.to_string());
        assert!(crate::credential::load_user_credentials()
            .unwrap()
            .is_empty());
        assert!(f
            .state
            .owned
            .session_store
            .get_session(&f.request.session_id)
            .unwrap()
            .active_interactions()
            .is_empty());
    }
}

#[tokio::test]
async fn au15_discovery_mismatch_unsupported_flow_and_hostile_errors_are_safe() {
    crate::test_support::isolated_env_test!();
    for mode in [
        "wrong_resource",
        "identity_only",
        "error_echo",
        "redirect",
        "missing_claim",
    ] {
        let f = fixture();
        elevated(&f);
        let service = Service::start().await;
        service.mode(mode);
        let mut args = json!({"request_id":mode,"resource_url":service.origin});
        if mode == "missing_claim" {
            args["email"] = json!("owner@example.test");
        }
        let failure = call(&f, args).await.unwrap_err();
        service.absent(&failure.to_string());
        assert!(crate::credential::load_user_credentials()
            .unwrap()
            .is_empty());
        if mode == "wrong_resource" || mode == "identity_only" {
            assert_eq!(service.registrations.load(Ordering::SeqCst), 0);
        }
    }
}

#[tokio::test]
async fn au15_anonymous_can_claim_and_direct_api_keys_go_to_vault() {
    crate::test_support::isolated_env_test!();
    let f = fixture();
    elevated(&f);
    let service = Service::start().await;
    service.mode("direct");
    let direct = call(
        &f,
        json!({"request_id":"direct","resource_url":service.origin}),
    )
    .await
    .unwrap();
    service.absent(&direct.to_string());
    service.mode("standard");
    service.claimed.store(true, Ordering::SeqCst);
    let claimed=call(&f,json!({"request_id":"anonymous-claim","resource_url":service.origin,"flow":"anonymous","claim":true,"email":"owner@example.test"})).await.unwrap();
    service.absent(&claimed.to_string());
    assert_eq!(crate::credential::load_user_credentials().unwrap().len(), 2);
}

#[tokio::test]
async fn au15_return_fields_gives_only_named_non_secret_registration_fields() {
    crate::test_support::isolated_env_test!();
    let f = fixture();
    elevated(&f);
    let service = Service::start().await;
    let created = call(
        &f,
        json!({"request_id":"fields","resource_url":service.origin,"scopes":["api.read"],
            "return_fields":["/project/id","/project/branch/id"]}),
    )
    .await
    .unwrap();
    assert_eq!(
        created["fields"],
        json!({"/project/id":"proj-1","/project/branch/id":"br-1"})
    );
    service.absent(&created.to_string());
    // A pointer that names a secret is refused before any network call, so no account is left behind.
    let before = service.registrations.load(Ordering::SeqCst);
    for refused in [
        json!(["/identity_assertion"]),
        json!(["/claim_token"]),
        json!(["project/id"]),
    ] {
        assert!(
            call(
                &f,
                json!({"request_id":format!("refused-{refused}"),"resource_url":service.origin,
                    "return_fields":refused})
            )
            .await
            .is_err(),
            "{refused} must be refused"
        );
    }
    assert_eq!(service.registrations.load(Ordering::SeqCst), before);
    // Absent or non-scalar fields are reported; the credential is still stored.
    let partial = call(
        &f,
        json!({"request_id":"partial","resource_url":service.origin,"scopes":["api.read"],
            "return_fields":["/project/id","/project","/missing"]}),
    )
    .await
    .unwrap();
    assert_eq!(partial["fields"], json!({"/project/id":"proj-1"}));
    assert_eq!(
        partial["unavailable_fields"],
        json!(["/project", "/missing"])
    );
    assert!(partial["credential_id"].as_str().is_some());
}
