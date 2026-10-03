//! Exercise the runtime tool route, a real lease and the encrypted relay reply.
use super::*;

impl RemoteQueueFixture {
    async fn popup_caller(&self) -> (Arc<CommandRouter>, String) {
        self.submit(&self.attachment_id, "LEASED_POPUP_TURN\n")
            .await;
        self.wait_for_delivered_prompt("LEASED_POPUP_TURN\n").await;
        let mut worker = self.app_worker.lock().await;
        let leased = RemoteLeaseRuntime::new(&mut worker)
            .leased_agent_snapshot_for_test(&self.leased_agent_id)
            .unwrap();
        let run = worker
            .providers()
            .get_run_for_agent(&leased.backing_session_id, &leased.backing_agent_id)
            .unwrap();
        let token = run
            .runtime_mcp_auth_token()
            .expect("worker runtime MCP token")
            .to_string();
        drop(worker);
        (
            Arc::new(CommandRouter::with_interactive_capacity(
                self.app_worker.clone(),
                8,
            )),
            token,
        )
    }

    async fn popup(&self) -> crate::session::RuntimeInteraction {
        tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                if let Some(interaction) = self
                    .app_home
                    .lock()
                    .await
                    .sessions()
                    .get_session(&self.session_id)
                    .unwrap()
                    .active_interaction_for_agent(&self.agent_id)
                    .cloned()
                {
                    return interaction;
                }
                sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("popup must reach home")
    }

    async fn assert_no_popup(&self) {
        assert!(self
            .app_home
            .lock()
            .await
            .sessions()
            .get_session(&self.session_id)
            .unwrap()
            .active_interaction_for_agent(&self.agent_id)
            .is_none());
    }

    async fn cleanup_popup(self) {
        self.dispatch(
            "destroy-popup-agent",
            LocalDaemonRequest::DestroyAgent(crate::local::DestroyAgentRequest {
                session_id: self.session_id.clone(),
                agent_id: self.agent_id.clone(),
            }),
        )
        .await;
        self.shutdown().await;
    }
}

fn call_popup(
    router: Arc<CommandRouter>,
    token: String,
    timeout_sec: Option<u64>,
    default: Option<&str>,
) -> tokio::task::JoinHandle<
    Result<crate::transport::runtime_tools::RuntimeToolResult, crate::error::DaemonError>,
> {
    let arguments = serde_json::json!({
        "message": "Ordinary leased App consent?",
        "timeout_sec": timeout_sec,
        "default_on_timeout": default,
        "choices": [
            {"id": "yes", "label": "Yes", "reply": "answered once"},
            {"id": "no", "label": "No", "reply": "declined"}
        ]
    });
    tokio::spawn(async move {
        router
            .dispatch_authenticated_runtime_tool_call(
                &token,
                crate::transport::runtime_tools::REQUEST_POPUP_TOOL,
                arguments,
            )
            .await
    })
}

#[test]
fn leased_popup_answers_once_after_sixty_seconds() {
    crate::test_support::isolated_env_test!();
    run_async_with_large_test_stack("leased-popup-long", || async {
        let _relay_guard = relay_client_test_guard().await;
        let _home = RelayTestHome::new();
        let fixture = RemoteQueueFixture::start("leased-popup-long").await;
        let (router, token) = fixture.popup_caller().await;
        // Both a declared 300s card and an indefinite card outlive the relay's 60s default.
        for deadline in [Some(300), None] {
            let call = call_popup(router.clone(), token.clone(), deadline, None);
            let popup = fixture.popup().await;
            let held = std::time::Instant::now();
            sleep(Duration::from_secs(65)).await;
            assert!(
                !call.is_finished(),
                "relay must still wait for the pending home card"
            );
            // A waiting human must not hold the worker's App owner.
            drop(
                tokio::time::timeout(Duration::from_secs(1), fixture.app_worker.lock())
                    .await
                    .expect("worker App must remain available"),
            );
            fixture
                .router
                .runtime_state()
                .resolve_runtime_interaction(&fixture.session_id, popup.id(), "yes", None)
                .await
                .unwrap();
            let result = tokio::time::timeout(Duration::from_secs(5), call)
                .await
                .expect("settled card must promptly cancel relay wait")
                .unwrap()
                .unwrap();
            assert!(result.ok);
            assert_eq!(result.payload["status"], "answered");
            assert_eq!(result.payload["choice_id"], "yes");
            assert_eq!(result.payload["reply"], "answered once");
            assert!(
                fixture
                    .router
                    .runtime_state()
                    .resolve_runtime_interaction(&fixture.session_id, popup.id(), "yes", None,)
                    .await
                    .is_err(),
                "duplicate answer must not be delivered"
            );
            fixture.assert_no_popup().await;
            eprintln!(
                "leased popup deadline={deadline:?}: answered once after {:?}",
                held.elapsed()
            );
        }
        fixture.cleanup_popup().await;
    });
}

#[test]
fn leased_popup_expiry_returns_the_home_outcome() {
    crate::test_support::isolated_env_test!();
    run_async_with_large_test_stack("leased-popup-expiry", || async {
        let _relay_guard = relay_client_test_guard().await;
        let _home = RelayTestHome::new();
        let fixture = RemoteQueueFixture::start("leased-popup-expiry").await;
        let (router, token) = fixture.popup_caller().await;
        for default in [None, Some("no")] {
            let call = call_popup(router.clone(), token.clone(), Some(2), default);
            fixture.popup().await;
            let result = tokio::time::timeout(Duration::from_secs(8), call)
                .await
                .expect("expiry must return the card outcome")
                .unwrap()
                .unwrap();
            assert!(result.ok);
            assert_eq!(
                result.payload["status"],
                if default.is_some() {
                    "answered"
                } else {
                    "timed_out"
                }
            );
            assert_eq!(result.payload["choice_id"], serde_json::json!(default));
            assert_eq!(
                result.payload["reply"],
                serde_json::json!(default.map(|_| "declined"))
            );
            fixture.assert_no_popup().await;
        }
        fixture.cleanup_popup().await;
    });
}

#[test]
fn leased_popup_withdrawal_ends_the_relay_wait() {
    crate::test_support::isolated_env_test!();
    run_async_with_large_test_stack("leased-popup-withdrawal", || async {
        let _relay_guard = relay_client_test_guard().await;
        let _home = RelayTestHome::new();
        let fixture = RemoteQueueFixture::start("leased-popup-withdrawal").await;
        let (router, token) = fixture.popup_caller().await;
        let call = call_popup(router, token, Some(300), None);
        fixture.popup().await;
        fixture
            .dispatch(
                "destroy-popup-agent",
                LocalDaemonRequest::DestroyAgent(crate::local::DestroyAgentRequest {
                    session_id: fixture.session_id.clone(),
                    agent_id: fixture.agent_id.clone(),
                }),
            )
            .await;
        let result = tokio::time::timeout(Duration::from_secs(5), call)
            .await
            .expect("withdrawal must cancel relay wait")
            .unwrap()
            .unwrap();
        assert_eq!(result.payload["status"], "timed_out");
        assert!(result.payload["choice_id"].is_null());
        fixture.assert_no_popup().await;
        fixture.shutdown().await;
    });
}
