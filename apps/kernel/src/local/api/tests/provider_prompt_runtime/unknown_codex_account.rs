//! MP-08 / MP-10 / MP-11: missing optional usage must not repeat first-use auth.
use super::*;
use std::io::{Read, Write};

#[test]
#[ignore = "fake official app-server child for the first-prompt regression"]
fn first_prompt_codex_app_server() {
    let endpoint = std::env::var("UIQA_CODEX_LISTEN").unwrap();
    let listener = std::net::TcpListener::bind(endpoint.trim_start_matches("ws://")).unwrap();
    for stream in listener.incoming() {
        let mut stream = stream.unwrap();
        std::thread::spawn(move || {
            stream
                .set_read_timeout(Some(std::time::Duration::from_secs(5)))
                .unwrap();
            let mut header = [0; 64];
            if stream
                .peek(&mut header)
                .is_ok_and(|n| header[..n].starts_with(b"GET /readyz "))
            {
                let _ = stream.read(&mut header);
                let _ = stream.write_all(
                    b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                );
                return;
            }
            let Ok(mut socket) = tokio_tungstenite::tungstenite::accept(stream) else {
                return;
            };
            while let Ok(message) = socket.read() {
                let Ok(text) = message.to_text() else {
                    continue;
                };
                let Ok(request) = serde_json::from_str::<serde_json::Value>(text) else {
                    continue;
                };
                let Some(id) = request.get("id") else {
                    continue;
                };
                let method = request["method"].as_str().unwrap_or_default();
                let response = if matches!(method, "account/rateLimits/read" | "account/usage/read")
                {
                    serde_json::json!({"id":id,"error":{"code":-32601,"message":"optional usage unavailable"}})
                } else {
                    let result = match method {
                        "account/read" => {
                            let mut log = fs::OpenOptions::new()
                                .create(true)
                                .append(true)
                                .open(std::env::var("UIQA_CODEX_CHECKS").unwrap())
                                .unwrap();
                            writeln!(log, "check").unwrap();
                            serde_json::json!({"account":{},"requiresOpenaiAuth":true})
                        }
                        "thread/start" | "thread/resume" => {
                            serde_json::json!({"thread":{"id":"fixture-thread"},"model":"gpt-6-luna"})
                        }
                        "turn/start" => {
                            serde_json::json!({"turn":{"id":"fixture-turn","status":"inProgress"}})
                        }
                        _ => serde_json::json!({}),
                    };
                    serde_json::json!({"id":id,"result":result})
                };
                if socket
                    .send(tokio_tungstenite::tungstenite::Message::Text(
                        response.to_string().into(),
                    ))
                    .is_err()
                {
                    break;
                }
            }
        });
    }
}

#[test]
fn unknown_codex_account_checks_once_without_optional_usage() {
    crate::test_support::isolated_env_test!();
    let _env = crate::env_lock::lock();
    let worktree = crate::test_support::TestWorktree::new("unknown-codex-no-usage");
    let root = worktree.path();
    std::env::set_var("CHARIOX_HOME", root.join("state"));
    std::env::set_var("HOME", root.join("home"));
    let source = root.join("codex-source");
    let executable = root.join("codex");
    let checks = root.join("checks");
    let quote = |s: &str| format!("'{}'", s.replace('\'', "'\\''"));
    fs::write(&source, format!(
        "#!/bin/sh\nif [ \"$1\" = --version ]; then echo fixture; exit; fi\nwhile [ \"$#\" -gt 0 ]; do if [ \"$1\" = --listen ]; then shift; export UIQA_CODEX_LISTEN=\"$1\"; break; fi; shift; done\nexport UIQA_CODEX_CHECKS={}\nexec {} --exact {} --ignored --nocapture\n",
        quote(checks.to_str().unwrap()), quote(std::env::current_exe().unwrap().to_str().unwrap()),
        quote("local::api::tests::provider_prompt_runtime::unknown_codex_account::first_prompt_codex_app_server"),
    )).unwrap();
    assert!(std::process::Command::new("install")
        .args(["-m", "700"])
        .arg(&source)
        .arg(&executable)
        .status()
        .unwrap()
        .success());
    std::env::set_var("CHARIOX_CODEX_BIN", &executable);
    let harness = LocalRouterTestHarness::new();
    let profile = harness.with_app(|app| {
        app.provider_account_profile_registry()
            .create_managed("local", "codex", "missing-usage")
            .unwrap()
    });
    struct EndpointCleanup(String);
    impl Drop for EndpointCleanup {
        fn drop(&mut self) {
            crate::provider::invalidate_codex_account_endpoint("local", &self.0);
        }
    }
    let _cleanup = EndpointCleanup(profile.profile_id.clone());
    let (session, agent) = match harness
        .dispatch(LocalDaemonRequest::CreateSession(
            worktree.session_request(),
        ))
        .unwrap()
    {
        LocalDaemonResponse::SessionCreated { session, agent, .. } => (session, agent),
        _ => panic!("expected session"),
    };
    harness
        .dispatch(LocalDaemonRequest::UpdateAgentProfile(
            UpdateAgentProfileRequest {
                session_id: session.id().into(),
                agent_id: agent.id().into(),
                provider: Some("codex".into()),
                account_profile: Some(profile.profile_id.clone()),
                model: Some("gpt-6-luna".into()),
                effort: Some("low".into()),
                clear_effort: false,
            },
        ))
        .unwrap();
    let attachment = match harness
        .dispatch(LocalDaemonRequest::AttachToSession(
            AttachToSessionRequest {
                session_id: session.id().into(),
                client_id: "first-prompt".into(),
                capability_level: ClientCapabilityLevel::FullTerminal,
            },
        ))
        .unwrap()
    {
        LocalDaemonResponse::SessionAttached { attachment } => attachment,
        _ => panic!("expected attachment"),
    };
    for _ in 0..2 {
        assert!(matches!(
            harness
                .dispatch(LocalDaemonRequest::SubmitPrompt(SubmitPromptRequest {
                    session_id: session.id().into(),
                    attachment_id: attachment.id().into(),
                    target_agent_id: Some(agent.id().into()),
                    prompt: "reply briefly".into(),
                    attachments: Vec::new(),
                }))
                .unwrap(),
            LocalDaemonResponse::PromptSubmitted { .. }
        ));
    }
    assert_eq!(fs::read_to_string(checks).unwrap().lines().count(), 1,
        "MP-08: optional usage unavailability must not run account/read again during the cold launch");
    harness
        .dispatch(LocalDaemonRequest::EndSession(EndSessionRequest {
            session_id: session.id().into(),
        }))
        .unwrap();
}
