use super::*;
use crate::config::{
    UserCredentialConfig, UserCredentialInjectionConfig, UserCredentialSourceConfig,
};
use crate::local::{KernelConnectionClass, LocalDaemonRequest};

const CANARY: &str = "MP-11-generated-test-only-secret";

fn credential() -> UserCredentialConfig {
    UserCredentialConfig {
        id: format!("response-canary-{:016x}", rand::random::<u64>()),
        description: None,
        source: UserCredentialSourceConfig::Env {
            name: "UNUSED_TEST_ONLY".into(),
        },
        allowed_hosts: vec!["example.com".into()],
        allowed_uses: vec![crate::config::UserCredentialUse::Http],
        injection: UserCredentialInjectionConfig::Header {
            name: "Authorization".into(),
            value: CANARY.into(),
        },
        metadata: None,
    }
}

fn command(class: KernelConnectionClass) -> KernelCommand {
    let request = LocalDaemonRequest::ListSessions(crate::local::ListSessionsRequest);
    let mut command = KernelCommand::from_local_request("response-test", None, None, &request);
    command.caller.connection_class = Some(class);
    command.caller.caller_id = "sudo:test".into();
    command
}

fn secret_responses() -> Vec<LocalDaemonResponse> {
    let credential = credential();
    let path = std::path::PathBuf::from("test-only.yaml");
    let mut mcp = crate::mcp::CharioxMcpServerConfig::stdio("response-secret", "true", vec![]);
    if let crate::mcp::CharioxMcpTransportConfig::Stdio { env, .. } = &mut mcp.transport {
        env.insert("API_KEY".into(), CANARY.into());
    }
    let mut agent = crate::agent::AgentInstance::new(
        "reply-agent",
        "a1",
        "reply-session",
        None,
        "codex",
        None,
        None,
        None,
        crate::agent::GridPosition::new(0, 0, 1, 1),
    );
    agent.set_remote_execution(Some(crate::agent::RemoteAgentBinding {
        worker_kernel_id: "worker".into(),
        worker_machine_id: "machine".into(),
        execution_lease_id: "lease".into(),
        leased_agent_id: "leased".into(),
        active_worker_provider_run_id: None,
        relay_url: None,
        relay_token: Some(CANARY.into()),
        relay_peer_protocol_version: None,
    }));
    let mut http = crate::mcp::CharioxMcpServerConfig::streamable_http(
        "response-header",
        "https://example.com/mcp",
    );
    if let crate::mcp::CharioxMcpTransportConfig::StreamableHttp { http_headers, .. } =
        &mut http.transport
    {
        http_headers.insert("Authorization".into(), CANARY.into());
    }
    vec![
        LocalDaemonResponse::AgentSpawned { agent },
        LocalDaemonResponse::McpServer { mcp: http },
        LocalDaemonResponse::CredentialRegistered {
            credential: credential.clone(),
            path: path.clone(),
        },
        LocalDaemonResponse::CredentialUpserted {
            credential: credential.clone(),
            path: path.clone(),
        },
        LocalDaemonResponse::CredentialRemoved {
            credential: credential.clone(),
            path: path.clone(),
        },
        LocalDaemonResponse::Credential {
            credential: credential.clone(),
        },
        LocalDaemonResponse::CredentialsListed {
            credentials: vec![credential],
        },
        LocalDaemonResponse::McpServerInstalled {
            mcp: mcp.clone(),
            path: path.clone(),
        },
        LocalDaemonResponse::McpServerUpdated {
            mcp: mcp.clone(),
            path,
        },
        LocalDaemonResponse::McpServer { mcp: mcp.clone() },
        LocalDaemonResponse::McpServersListed { mcps: vec![mcp] },
        LocalDaemonResponse::ProviderLoginStarted {
            login: crate::provider::ProviderLoginStart {
                provider: "codex".into(),
                account_profile: "test".into(),
                login_kind: "device_code".into(),
                login_id: None,
                auth_url: Some(CANARY.into()),
                verification_url: None,
                user_code: Some(CANARY.into()),
            },
        },
    ]
}

// Generate the complete request-name inventory from the actual enum. Exercise
// the final router boundary for every command kind, independently of which
// executor produced a response. This is not a claim to execute all operations.
#[test]
fn external_response_all_request_variants_protect_secret_replies() {
    let source = include_str!("../../local/api/types/request.rs");
    let names: Vec<_> = source
        .lines()
        .filter_map(|line| {
            let declaration = line.strip_prefix("    ")?;
            let name = declaration.split(['(', '{', ',']).next()?.trim();
            (name.chars().next()?.is_ascii_uppercase()
                && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_'))
            .then_some(name)
        })
        .collect();
    assert!(names.len() > 300);
    for class in [
        KernelConnectionClass::ExternalAgent,
        KernelConnectionClass::KernelAgent,
    ] {
        for name in &names {
            let mut command = command(class);
            command.command_type = (*name).into();
            for response in secret_responses() {
                if let Ok(projected) = finish_response(&command, Ok(response)) {
                    assert!(
                        !serde_json::to_string(&projected).unwrap().contains(CANARY),
                        "secret response escaped for {name}"
                    );
                }
            }
            let failure = DaemonError::LocalTransport {
                operation: "test-secret-parse",
                message: CANARY.into(),
            };
            let error = finish_response(&command, Err(failure)).unwrap_err();
            assert!(
                !error.to_string().contains(CANARY),
                "secret error escaped for {name}"
            );
        }
    }
}

#[test]
fn external_response_terminal_retains_owner_replies() {
    for response in secret_responses() {
        let projected =
            finish_response(&command(KernelConnectionClass::Terminal), Ok(response)).unwrap();
        assert!(serde_json::to_string(&projected).unwrap().contains(CANARY));
    }
}

#[test]
fn external_response_retains_only_exact_public_authority_errors() {
    let caller = command(KernelConnectionClass::ExternalAgent);
    for message in [
        "grant revoked or expired",
        "sudo request refused",
        "sudo request expired; answer the popup in a Chariox terminal",
    ] {
        let error = finish_response(&caller, Err(crate::runtime::kernel_access::error(message)))
            .unwrap_err();
        assert!(error.to_string().contains(message));
        let tainted = finish_response(
            &caller,
            Err(crate::runtime::kernel_access::error(format!(
                "{message}: {CANARY}"
            ))),
        )
        .unwrap_err();
        assert!(!tainted.to_string().contains(CANARY));
    }
}

#[test]
fn external_response_redacts_nested_remote_bindings_without_scanning_history() {
    let mut value = serde_json::json!({"sessions":[{"agents":[{"remote_execution":{
        "worker_kernel_id":"worker", "relay_token":CANARY
    }}]}],"nested_credential":{"injection":{"kind":"header","name":"Authorization","value":CANARY}},
    "history_text":CANARY});
    // MP-11: exercise the common structured-secret projection used at delivery.
    redact_secret_values(&mut value);
    assert!(value["sessions"][0]["agents"][0]["remote_execution"]
        .get("relay_token")
        .is_none());
    assert_eq!(value["history_text"], CANARY);
    assert_eq!(
        value["nested_credential"]["injection"]["value"],
        "[REDACTED]"
    );
}

#[tokio::test]
async fn external_response_register_and_remove_existing_literal_credentials() {
    use std::sync::Arc;
    use tokio::sync::Mutex;
    let worktree = crate::test_support::TestWorktree::new("response-credential");
    let mut app =
        crate::test_support::bootstrap_authenticated_app(crate::config::DaemonConfig::for_tests())
            .unwrap();
    let (session, _) = crate::app::KernelSessionService::new(&mut app)
        .create_session(worktree.session_request())
        .unwrap();
    let router = crate::runtime::router::CommandRouter::with_interactive_capacity_from_app(
        Arc::new(Mutex::new(app)),
        32,
    );
    let grant = router
        .runtime_state()
        .insert_access_grant_for_test(session.id());
    let credential = credential();
    let registry = crate::credential::CharioxCredentialRegistry::user().unwrap();
    let (_, path) = registry.upsert(credential.clone()).unwrap();
    for request in [
        LocalDaemonRequest::RegisterCredential(crate::local::RegisterCredentialRequest {
            source_path: path.clone(),
        }),
        LocalDaemonRequest::RemoveCredential(crate::local::RemoveCredentialRequest {
            id: credential.id.clone(),
        }),
    ] {
        let mut caller = command(KernelConnectionClass::ExternalAgent);
        caller.caller.caller_id = grant.clone();
        let response = router.dispatch(caller, request).await.unwrap();
        // Both mutations succeed and retain their public acknowledgement.
        assert!(!serde_json::to_string(&response).unwrap().contains(CANARY));
    }
    assert!(!path.exists());
}
