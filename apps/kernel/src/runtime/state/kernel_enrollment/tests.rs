//! `chariox.kernels.enroll` against a stand-in Cloud: one owner card, the owner's answer, the owner's kernel decides.
use super::*;
use crate::local::KernelConnectionClass;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

/// What the stand-in Cloud knows about the waiting login and what it was asked to decide.
struct Cloud {
    api_url: String,
    status: Arc<std::sync::Mutex<String>>,
    expires_in_s: Arc<std::sync::Mutex<i64>>,
    decisions: Arc<std::sync::Mutex<Vec<(String, serde_json::Value)>>>,
}

async fn cloud(status: &str) -> Cloud {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let api_url = format!("http://{}", listener.local_addr().unwrap());
    let status = Arc::new(std::sync::Mutex::new(status.to_owned()));
    let expires_in_s = Arc::new(std::sync::Mutex::new(600_i64));
    let decisions = Arc::new(std::sync::Mutex::new(Vec::new()));
    let (state, expires, seen) = (status.clone(), expires_in_s.clone(), decisions.clone());
    tokio::spawn(async move {
        loop {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut bytes = Vec::new();
            let header_end = loop {
                let mut chunk = [0; 4096];
                let n = socket.read(&mut chunk).await.unwrap();
                assert!(n > 0);
                bytes.extend_from_slice(&chunk[..n]);
                if let Some(end) = bytes.windows(4).position(|part| part == b"\r\n\r\n") {
                    break end + 4;
                }
            };
            let head = String::from_utf8_lossy(&bytes[..header_end]).to_string();
            let length: usize = head
                .lines()
                .find_map(|line| {
                    let (key, value) = line.split_once(':')?;
                    key.eq_ignore_ascii_case("content-length").then(|| value.trim().parse().unwrap())
                })
                .unwrap_or(0);
            while bytes.len() < header_end + length {
                let mut chunk = [0; 4096];
                let n = socket.read(&mut chunk).await.unwrap();
                assert!(n > 0);
                bytes.extend_from_slice(&chunk[..n]);
            }
            let credential_ok = head
                .to_ascii_lowercase()
                .contains("x-chariox-kernel-credential: synthetic-test-credential");
            let first = head.lines().next().unwrap().to_owned();
            let (code, body) = if !credential_ok {
                ("403 Forbidden", serde_json::json!({"error":{"code":"session_invalid","message":"no"}}))
            } else if first.starts_with("GET /v1/kernel-device-approvals?userCode=ABCD-EFGH") {
                let expires_at = (chrono::Utc::now() + chrono::Duration::seconds(*expires.lock().unwrap())).to_rfc3339();
                (
                    "200 OK",
                    serde_json::json!({"userCode":"ABCD-EFGH","status":state.lock().unwrap().clone(),"machineName":"Drill box","expiresAt":expires_at}),
                )
            } else if first.starts_with("GET /v1/kernel-device-approvals") {
                ("404 Not Found", serde_json::json!({"error":{"code":"not_found","message":"not found"}}))
            } else {
                let body: serde_json::Value = serde_json::from_slice(&bytes[header_end..]).unwrap();
                let decision = body["decision"].as_str().unwrap().to_owned();
                seen.lock().unwrap().push((decision.clone(), body));
                *state.lock().unwrap() = if decision == "approve" { "APPROVED".into() } else { "DENIED".into() };
                ("200 OK", serde_json::json!({"decision":decision}))
            };
            let body = body.to_string();
            socket
                .write_all(format!("HTTP/1.1 {code}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).as_bytes())
                .await
                .unwrap();
        }
    });
    Cloud { api_url, status, expires_in_s, decisions }
}

async fn cards(state: &KernelRuntimeState, room: &str, want: usize) -> Vec<(String, String)> {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let session = state.owned.session_store.get_session(room).unwrap();
            let cards: Vec<_> = session.active_interactions().iter().map(|c| (c.id().to_owned(), c.message().to_owned())).collect();
            if cards.len() >= want {
                return cards;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap()
}

fn room_of(state: &KernelRuntimeState, token: &str) -> String {
    let run = state.owned.provider_store.get_runs_by_runtime_mcp_auth_token(token).remove(0);
    run.session_id().to_owned()
}

fn args() -> serde_json::Value {
    serde_json::json!({"user_code":"abcd efgh"})
}

#[tokio::test]
async fn the_owner_gets_one_card_and_approving_it_lets_the_kernel_join() {
    let cloud = cloud("PENDING").await;
    let (state, token, _workspace) = super::super::machine_acquisition::tool_tests::agent_kernel(cloud.api_url.clone()).await;
    let room = room_of(&state, &token);
    let tool = {
        let (state, token) = (state.clone(), token.clone());
        tokio::spawn(async move { state.kernel_enrollment_tool(&token, args()).await })
    };
    let listed = cards(&state, &room, 1).await;
    assert_eq!(listed.len(), 1, "one card, not one per client");
    assert!(listed[0].0.starts_with("kernel-enrollment-"));
    assert!(listed[0].1.contains("Drill box") && listed[0].1.contains("ABCD-EFGH"), "{}", listed[0].1);
    assert!(!tool.is_finished(), "the kernel does not join before the owner answers");
    let answer = crate::local::RespondToInteractionRequest {
        session_id: room.clone(),
        interaction_id: listed[0].0.clone(),
        choice_id: "approve".into(),
        custom_reply: None,
        passkey: None,
        passkey_remember_minutes: None,
    };
    assert!(
        !state.interaction_person_only(&answer),
        "any client the owner answers from, including an agent acting for them through a grant, may settle it"
    );
    assert!(cloud.decisions.lock().unwrap().is_empty());
    let owner = state.owned.session_store.get_session(&room).unwrap().owner_user_id().to_owned();
    assert!(
        state
            .answer_terminal_runtime_interaction(&room, &listed[0].0, "approve", None, Some(&owner), None, None, Some(KernelConnectionClass::KernelAgent))
            .await
            .is_err(),
        "the agent cannot approve its own request"
    );
    state.resolve_terminal_runtime_interaction(&room, &listed[0].0, "approve", None, Some(&owner)).await.unwrap();
    let result = tool.await.unwrap().unwrap();
    assert_eq!(result.payload["status"], "approved");
    let decisions = cloud.decisions.lock().unwrap();
    assert_eq!(decisions.len(), 1);
    assert_eq!(decisions[0].0, "approve");
    assert_eq!(decisions[0].1["userCode"], "ABCD-EFGH");
    drop(decisions);
    // Asking again after approval is answered without a second card.
    let again = state.kernel_enrollment_tool(&token, args()).await.unwrap();
    assert_eq!(again.payload["status"], "approved");
    assert!(state.owned.session_store.get_session(&room).unwrap().active_interactions().is_empty());
}

#[tokio::test]
async fn denying_the_card_tells_cloud_and_the_agent_plainly() {
    let cloud = cloud("PENDING").await;
    let (state, token, _workspace) = super::super::machine_acquisition::tool_tests::agent_kernel(cloud.api_url.clone()).await;
    let room = room_of(&state, &token);
    let tool = {
        let (state, token) = (state.clone(), token.clone());
        tokio::spawn(async move { state.kernel_enrollment_tool(&token, args()).await })
    };
    let listed = cards(&state, &room, 1).await;
    let owner = state.owned.session_store.get_session(&room).unwrap().owner_user_id().to_owned();
    state.resolve_terminal_runtime_interaction(&room, &listed[0].0, "deny", None, Some(&owner)).await.unwrap();
    let error = tool.await.unwrap().unwrap_err().to_string();
    assert!(error.contains("The owner denied this kernel"), "{error}");
    assert_eq!(cloud.decisions.lock().unwrap()[0].0, "deny");
}

#[tokio::test]
async fn an_expired_code_raises_no_card_and_says_so() {
    let cloud = cloud("EXPIRED").await;
    let (state, token, _workspace) = super::super::machine_acquisition::tool_tests::agent_kernel(cloud.api_url.clone()).await;
    let room = room_of(&state, &token);
    let error = state.kernel_enrollment_tool(&token, args()).await.unwrap_err().to_string();
    assert!(error.contains("expired"), "{error}");
    assert!(state.owned.session_store.get_session(&room).unwrap().active_interactions().is_empty());
    assert!(cloud.decisions.lock().unwrap().is_empty());
    *cloud.status.lock().unwrap() = "PENDING".into();
    *cloud.expires_in_s.lock().unwrap() = -5;
    let error = state.kernel_enrollment_tool(&token, args()).await.unwrap_err().to_string();
    assert!(error.contains("expired"), "{error}");
    let unknown = state.kernel_enrollment_tool(&token, serde_json::json!({"user_code":"ZZZZ-ZZZZ"})).await.unwrap_err().to_string();
    assert!(unknown.contains("not found"), "{unknown}");
    let malformed = state.kernel_enrollment_tool(&token, serde_json::json!({"user_code":"x"})).await.unwrap_err().to_string();
    assert!(malformed.contains("ABCD-EFGH"), "{malformed}");
}

#[tokio::test]
async fn another_owners_agent_cannot_enroll_into_this_kernels_account() {
    let cloud = cloud("PENDING").await;
    let (state, token, _workspace) = super::super::machine_acquisition::tool_tests::agent_kernel(cloud.api_url.clone()).await;
    let room = room_of(&state, &token);
    let mut config = state.owned.config_projection.snapshot();
    config.cloud_relay.as_mut().unwrap().user_id = "someone-else".into();
    state.owned.config_projection.update(config);
    let error = state.kernel_enrollment_tool(&token, args()).await.unwrap_err().to_string();
    assert!(error.contains("Only the owner of this kernel"), "{error}");
    assert!(state.owned.session_store.get_session(&room).unwrap().active_interactions().is_empty());
    assert!(cloud.decisions.lock().unwrap().is_empty());
}

#[tokio::test]
async fn only_the_owners_terminal_answers_the_card() {
    let cloud = cloud("PENDING").await;
    let (state, token, _workspace) = super::super::machine_acquisition::tool_tests::agent_kernel(cloud.api_url.clone()).await;
    let room = room_of(&state, &token);
    let tool = {
        let (state, token) = (state.clone(), token.clone());
        tokio::spawn(async move { state.kernel_enrollment_tool(&token, args()).await })
    };
    let listed = cards(&state, &room, 1).await;
    for class in [KernelConnectionClass::KernelAgent, KernelConnectionClass::RelayPeer] {
        assert!(
            state
                .answer_terminal_runtime_interaction(&room, &listed[0].0, "approve", None, Some("someone-else"), None, None, Some(class))
                .await
                .is_err()
        );
    }
    assert!(!tool.is_finished());
    assert!(cloud.decisions.lock().unwrap().is_empty());
    let owner = state.owned.session_store.get_session(&room).unwrap().owner_user_id().to_owned();
    assert!(
        state.resolve_terminal_runtime_interaction(&room, &listed[0].0, "approve", None, Some("someone-else")).await.is_err(),
        "a different user cannot approve"
    );
    state.resolve_terminal_runtime_interaction(&room, &listed[0].0, "deny", None, Some(&owner)).await.unwrap();
    assert!(tool.await.unwrap().is_err());
}

#[tokio::test]
async fn a_call_never_outlives_the_wait_window_and_asking_again_raises_no_second_card() {
    let cloud = cloud("PENDING").await;
    let (state, token, _workspace) = super::super::machine_acquisition::tool_tests::agent_kernel(cloud.api_url.clone()).await;
    let room = room_of(&state, &token);
    let first = state.kernel_enrollment_tool(&token, args()).await.unwrap();
    assert_eq!(first.payload["status"], "waiting_for_owner");
    let second = state.kernel_enrollment_tool(&token, args()).await.unwrap();
    assert_eq!(second.payload["status"], "waiting_for_owner");
    let listed = cards(&state, &room, 1).await;
    assert_eq!(listed.len(), 1, "one card however often the agent asks");
    assert!(cloud.decisions.lock().unwrap().is_empty());
    let owner = state.owned.session_store.get_session(&room).unwrap().owner_user_id().to_owned();
    state.resolve_terminal_runtime_interaction(&room, &listed[0].0, "approve", None, Some(&owner)).await.unwrap();
    let third = state.kernel_enrollment_tool(&token, args()).await.unwrap();
    assert_eq!(third.payload["status"], "approved");
    assert_eq!(cloud.decisions.lock().unwrap().len(), 1, "Cloud is told once");
}

#[tokio::test]
async fn a_card_nobody_answers_is_withdrawn_when_the_code_expires_and_the_agent_is_told() {
    let cloud = cloud("PENDING").await;
    *cloud.expires_in_s.lock().unwrap() = 2;
    let (state, token, _workspace) = super::super::machine_acquisition::tool_tests::agent_kernel(cloud.api_url.clone()).await;
    let room = room_of(&state, &token);
    let first = state.kernel_enrollment_tool(&token, args()).await.unwrap();
    assert_eq!(first.payload["status"], "waiting_for_owner");
    tokio::time::timeout(Duration::from_secs(10), async {
        while !state.owned.session_store.get_session(&room).unwrap().active_interactions().is_empty() {
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .expect("the unanswered card is withdrawn");
    *cloud.status.lock().unwrap() = "EXPIRED".into();
    let error = state.kernel_enrollment_tool(&token, args()).await.unwrap_err().to_string();
    assert!(error.contains("expired"), "{error}");
    assert!(cloud.decisions.lock().unwrap().is_empty());
}

#[tokio::test]
async fn a_card_the_kernel_withdraws_on_its_own_timeout_tells_cloud_nothing_and_can_be_raised_again() {
    let cloud = cloud("PENDING").await;
    let (state, token, _workspace) = super::super::machine_acquisition::tool_tests::agent_kernel(cloud.api_url.clone()).await;
    let room = room_of(&state, &token);
    let first = state.kernel_enrollment_tool(&token, args()).await.unwrap();
    assert_eq!(first.payload["status"], "waiting_for_owner");
    let listed = cards(&state, &room, 1).await;
    // The kernel's own timeout withdraws the unanswered card.
    state.timeout_runtime_interaction(&room, &listed[0].0).await.unwrap();
    tokio::time::timeout(Duration::from_secs(5), async {
        while !state.owned.session_store.get_session(&room).unwrap().active_interactions().is_empty() {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("the withdrawn card leaves");
    assert!(cloud.decisions.lock().unwrap().is_empty(), "a timeout is not a denial: {:?}", cloud.decisions.lock().unwrap());
    // The code is still open in Cloud, so asking again puts up one fresh card.
    let again = state.kernel_enrollment_tool(&token, args()).await.unwrap();
    assert_eq!(again.payload["status"], "waiting_for_owner");
    assert_eq!(cards(&state, &room, 1).await.len(), 1);
}
