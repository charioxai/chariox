//! No-session public-protocol drill with a real fixed App ABI worker.
//! Validation enters at the durable broker boundary; the normal pump and
//! RuntimeInteraction/passkey verifier handle the decision.
use super::*;
use crate::{
    durable_state::app_validations::{
        self, EffectReceipt, ValidationCommand, ValidationOperation, ValidationState,
    },
    local::*,
};
use chariox_app_package::{verify, VerificationPolicy};
use chariox_app_runtime::{
    release_store::{ReleaseStore, StageBudget},
    worker_peer::{Broker, BrokerFuture, BrokerRequest, PeerLimits},
    worker_process::test_fixture::{Fixture, Mode},
};

struct RejectBroker;
impl Broker for RejectBroker {
    fn handle(&self, _: BrokerRequest) -> BrokerFuture {
        Box::pin(async {
            Err(chariox_app_runtime::wire::RemoteError {
                code: "FIXTURE_UNSUPPORTED".into(),
                message: "Fixed fixture".into(),
                retryable: None,
            })
        })
    }
}
struct Scratch(std::path::PathBuf);
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn request<'a>(
    router: &'a CommandRouter,
    user: &'a str,
    request: LocalDaemonRequest,
) -> std::pin::Pin<
    Box<dyn std::future::Future<Output = Result<LocalDaemonResponse, DaemonError>> + Send + 'a>,
> {
    // MD-APP: allocate each dispatch at construction, before composing the drill.
    Box::pin(async move {
        let mut command = remote_command_for_request(&request, Some(user));
        command.command_id = format!("user-view-{:016x}", rand::random::<u64>());
        command.caller.connection_class = Some(KernelConnectionClass::Terminal);
        let dispatch = router.dispatch(command, request);
        assert!(
            std::mem::size_of_val(&dispatch) < 64 * 1024,
            "MD-APP/MD-4: App-view admission must fit an ordinary caller stack"
        );
        Box::pin(dispatch).await
    })
}
fn snapshot() -> LocalDaemonRequest {
    LocalDaemonRequest::SubscribeUserAppViews(SubscribeUserAppViewsRequest {
        after: None,
        wait_ms: 0,
    })
}
fn answer(id: &str, passkey: Option<&str>) -> LocalDaemonRequest {
    LocalDaemonRequest::AnswerUserDomainInteraction(AnswerUserDomainInteractionRequest {
        interaction_id: id.into(),
        choice_id: "approve".into(),
        passkey: passkey.map(ApprovalPasskey::new),
        passkey_remember_minutes: None,
    })
}

#[test]
fn user_app_view_no_session_integration_drill() {
    run_drill(false, false);
}

#[test]
fn user_app_view_close_cancels_a_stalled_channel_call() {
    run_drill(true, false);
}

#[test]
#[ignore = "Requires a normal Linux user, disposable CHARIOX_HOME and sandboxed native Chromium"]
fn user_app_view_kernel_browser_integration_drill() {
    assert_ne!(
        unsafe { libc::geteuid() },
        0,
        "run the host drill as a normal Unix user"
    );
    run_drill(false, true);
}

fn run_drill(stall: bool, browser: bool) {
    std::thread::Builder::new()
        .spawn(move || {
            let runtime = tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .enable_all()
                .build()
                .unwrap();
            let _entered = runtime.enter();
            let scratch = Scratch(
                std::env::temp_dir()
                    .join(format!("chariox-user-app-{:016x}", rand::random::<u64>())),
            );
            std::fs::create_dir(&scratch.0).unwrap();
            let root = scratch.0.canonicalize().unwrap();
            let test_vault = root.join("fixture-vault.json");
            crate::secret::create_chariox_encrypted_vault_for_test(
                &test_vault,
                "fixture-passphrase",
            )
            .unwrap();
            let mut config =
                DaemonConfig::for_tests().with_session_history_root(root.join("history"));
            config.user_config_path = root.join("config.toml");
            config.local_socket_path = root.join("kernel.sock");
            config.user_config.state.path = Some(root.join("state.db").display().to_string());
            config.user_config.history.operational.path =
                Some(root.join("events.db").display().to_string());
            config.user_config.artifacts.operational.root =
                Some(root.join("artifacts").display().to_string());
            config.user_config.artifacts.operational.index_path =
                Some(root.join("artifacts.db").display().to_string());
            config.user_config.credential_vault.backend =
                crate::config::CredentialVaultBackend::CharioxEncrypted;
            config.user_config.credential_vault.path = test_vault.display().to_string();
            let app = DaemonApp::bootstrap(config).unwrap();
            let sessions = app.session_state_store();
            assert!(sessions.list_sessions().is_empty());
            let store = app.durable_state_store();
            let (catalog, bytes, publisher) = if browser {
                crate::durable_state::app_state::fixture_browser_tool_package(&store)
            } else {
                let catalog = crate::durable_state::app_state::fixture_tool_catalog(&store);
                let (bytes, publisher) = crate::durable_state::app_state::fixture_tool_package();
                (catalog, bytes, publisher)
            };
            let router = CommandRouter::with_interactive_capacity(Arc::new(Mutex::new(app)), 2);
            let fixture = Fixture::compile().unwrap();
            let package = verify(
                &bytes,
                &VerificationPolicy::new(LOCAL_DAEMON_PROTOCOL_VERSION, vec![publisher]),
            )
            .unwrap();
            ReleaseStore::open_or_create(store.path())
                .unwrap()
                .stage(
                    &package,
                    &bytes,
                    StageBudget {
                        max_stage_bytes: 1024 * 1024,
                        reserved_bytes: 1024 * 1024,
                        host_reserve_bytes: 1024 * 1024,
                    },
                )
                .unwrap();
            let (process, observed) = fixture.spawn_blocking(if stall { Mode::ToolStall } else { Mode::ToolEcho }, &package).unwrap();
            let (starting, _events) = crate::runtime::app_worker::AppWorkerOwner::start_blocking(
                process,
                &package,
                catalog.clone(),
                Arc::new(RejectBroker),
                PeerLimits::default(),
                runtime.handle().clone(),
            )
            .unwrap();
            let mut registered = starting
                .await_registered_blocking(std::time::Duration::from_secs(3))
                .unwrap();
            let proof = store
                .confirm_app_activation(
                    "alice",
                    registered.catalog().clone(),
                    registered.take_activation_budget().unwrap(),
                )
                .unwrap();
            let (worker, handle) = registered.activate_blocking(proof).unwrap();
            router
                .runtime_state
                .app_control()
                .publish_app_worker("alice", handle)
                .unwrap();
            let outcome = runtime.block_on(Box::pin(async {
                use futures_util::FutureExt;
                let outcome = std::panic::AssertUnwindSafe(Box::pin(async {
                let open = LocalDaemonRequest::OpenUserAppView(OpenUserAppViewRequest {
                    installation_id: "installed".into(),
                    host: browser.then_some(UserAppViewHost::KernelBrowser),
                });
                let mut unverified = remote_command_for_request(&open, None);
                unverified.caller.connection_class = Some(KernelConnectionClass::Unauthenticated);
                assert!(matches!(
                    router.dispatch(unverified, open.clone()).await, Err(DaemonError::UserDomainRefused { reason: crate::error::UserDomainRefusalReason::NotGranted })
                ));
                let mut provider = remote_command_for_request(&open, Some("alice"));
                provider.caller.connection_class = Some(KernelConnectionClass::KernelAgent);
                assert!(matches!(
                    router.dispatch(provider, open.clone()).await, Err(DaemonError::UserDomainRefused { reason: crate::error::UserDomainRefusalReason::NotGranted })
                ));
                let LocalDaemonResponse::UserAppViewOpened { view, frontend } =
                    request(&router, "alice", open).await.unwrap()
                else {
                    panic!("open native view");
                };
                if !browser {
                    let capture=LocalDaemonRequest::CaptureVisibleRegion(CaptureVisibleRegionRequest {
                        capture_id:"human-capture".into(), surface:ScreenshotSurface::UserAppView {view_id:view.view_id.clone(),generation:1},
                        region:ScreenshotRegion{x:0,y:0,width:10,height:10,viewport_width:1280,viewport_height:800,frame_width:1280,frame_height:800},
                    });
                    assert!(request(&router,"bob",capture.clone()).await.is_err(),"another owner cannot capture a view");
                    assert!(request(&router,"alice",capture.clone()).await.is_err(),"native views have no kernel pixel surface");
                    let mut provider=remote_command_for_request(&capture,Some("alice"));provider.caller.connection_class=Some(KernelConnectionClass::KernelAgent);
                    assert!(router.dispatch(provider,capture).await.is_err(),"App/agent routes cannot invoke human capture");
                }
                assert_eq!(frontend.entry, "index.html");
                assert!(frontend
                    .assets
                    .iter()
                    .all(|a| !a.path.starts_with("runtime/") && !a.path.contains("..")));
                assert!(frontend
                    .content_security_policy
                    .contains("connect-src 'none'"));
                assert!(!frontend.iframe_sandbox.contains("allow-top-navigation"));
                assert!(
                    sessions.list_sessions().is_empty(),
                    "opening must not create a session"
                );
                let fetch = LocalDaemonRequest::GetUserAppViewFrontend(UserAppViewRequest {
                    view_id: view.view_id.clone(),
                });
                assert!(matches!(
                    request(&router, "bob", fetch.clone()).await, Err(DaemonError::UserDomainRefused { reason: crate::error::UserDomainRefusalReason::NotGranted })
                ));
                assert!(matches!(
                    request(&router, "alice", fetch).await.unwrap(),
                    LocalDaemonResponse::UserAppViewFrontend { .. }
                ));
                let call = LocalDaemonRequest::CallUserAppView(CallUserAppViewRequest {
                    view_id: view.view_id.clone(),
                    method: "echo".into(),
                    input: serde_json::json!({"text":"native"}),
                });
                assert!(matches!(
                    request(&router, "bob", call.clone()).await, Err(DaemonError::UserDomainRefused { reason: crate::error::UserDomainRefusalReason::NotGranted })
                ));
                if stall {
                    let call_router = router.clone();
                    let pending_request = call.clone();
                    let mut pending = tokio::spawn(async move { request(&call_router, "alice", pending_request).await });
                    tokio::time::timeout(std::time::Duration::from_secs(3), async {
                        while observed.tool_invocations() == 0 { tokio::time::sleep(std::time::Duration::from_millis(10)).await; }
                    }).await.unwrap();
                    assert!(!pending.is_finished());
                    request(&router, "alice", LocalDaemonRequest::CloseUserAppView(UserAppViewRequest { view_id: view.view_id.clone() })).await.unwrap();
                    let result = tokio::time::timeout(std::time::Duration::from_secs(3), &mut pending).await.unwrap().unwrap().unwrap();
                    assert!(matches!(result, LocalDaemonResponse::UserAppViewCallResult { error: Some(ref error), .. } if error.code == "CANCELLED"));
                    tokio::time::timeout(std::time::Duration::from_secs(3), async {
                        while observed.tool_cancellations() == 0 { tokio::time::sleep(std::time::Duration::from_millis(10)).await; }
                    }).await.unwrap();
                    assert!(sessions.list_sessions().is_empty());
                    return;
                }
                let LocalDaemonResponse::UserAppViewCallResult {
                    result: Some(result),
                    error: None,
                } = request(&router, "alice", call.clone()).await.unwrap()
                else {
                    panic!("channel call");
                };
                assert_eq!(result, serde_json::json!({"ok":true}));
                if browser { Box::pin(check_browser_page(&router, &view, &observed, 2)).await; }
                else { assert!(view.browser.is_none()); }
                let recorded = observed.tool_requests().unwrap();
                assert_eq!(recorded.len(), if browser { 2 } else { 1 });
                assert_eq!(
                    recorded[0]["context"]["actor"],
                    serde_json::json!({"kind":"human","id":"alice"})
                );
                assert!(recorded[0]["context"].get("room_id").is_none());
                assert!(recorded[0]["context"].get("agent_id").is_none());
                for recorded in &recorded {
                    assert_eq!(recorded["context"]["actor"], serde_json::json!({"kind":"human","id":"alice"}));
                    assert!(recorded["context"].get("room_id").is_none());
                    assert!(recorded["context"].get("agent_id").is_none());
                }
                let LocalDaemonResponse::UserAppViewsChanged {
                    cursor,
                    views,
                    interactions,
                } = request(&router, "alice", snapshot()).await.unwrap()
                else {
                    panic!("subscription");
                };
                assert_eq!(views.len(), 1);
                assert!(interactions.is_empty());
                // Kernel broker's validated request at its durable boundary.
                let now = crate::session::unix_epoch_ms();
                let (parameters, digest) =
                    app_validations::canonical(&serde_json::json!({"text":"native"}));
                store
                    .app_validation(ValidationCommand::Create(ValidationOperation {
                        operation_id: "view-approval".into(),
                        owner: "alice".into(),
                        installation: "installed".into(),
                        generation: 1,
                        action: "fixture_action".into(),
                        parameters,
                        digest: digest.clone(),
                        state: ValidationState::Pending,
                        expires_ms: now + app_validations::PENDING_MS,
                        callers: serde_json::json!([{"kind":"human","id":"alice"}]).to_string(),
                    }))
                    .unwrap();
                router.runtime_state.schedule_app_validation_pump();
                let snapshot = tokio::time::timeout(
                    std::time::Duration::from_secs(3),
                    request(
                        &router,
                        "alice",
                        LocalDaemonRequest::SubscribeUserAppViews(SubscribeUserAppViewsRequest {
                            after: Some(cursor),
                            wait_ms: 25000,
                        }),
                    ),
                )
                .await
                .unwrap()
                .unwrap();
                let LocalDaemonResponse::UserAppViewsChanged { interactions, .. } = snapshot else {
                    panic!("approval snapshot");
                };
                assert_eq!(interactions.len(), 1);
                let id = interactions[0].id();
                assert_eq!(
                    interactions[0].kernel_operation_id(),
                    Some("validation:view-approval")
                );
                let prompts = router.runtime_state.passkey_prompts_for("alice");
                assert_eq!(prompts.len(), 1);
                assert_eq!(prompts[0].session_id, "");
                assert!(router.runtime_state.passkey_prompts_for("bob").is_empty());
                assert!(request(&router, "bob", answer(id, None)).await.is_err());
                assert!(request(&router, "alice", answer(id, None))
                    .await
                    .unwrap_err()
                    .to_string()
                    .contains("PASSKEY_REQUIRED"));
                assert!(request(&router, "alice", answer(id, Some("wrong-fixture")))
                    .await
                    .unwrap_err()
                    .to_string()
                    .contains("PASSKEY_REJECTED"));
                request(&router, "alice", answer(id, Some("fixture-passphrase")))
                    .await
                    .unwrap();
                assert!(
                    request(&router, "alice", answer(id, Some("fixture-passphrase")))
                        .await
                        .is_err()
                );
                assert!(router.runtime_state.passkey_prompts_for("alice").is_empty());
                tokio::time::timeout(std::time::Duration::from_secs(3), async {
                    loop {
                        if store
                            .app_validation_status("alice", "installed", "view-approval")
                            .unwrap()
                            .unwrap()
                            .state
                            == ValidationState::Approved
                        {
                            break;
                        }
                        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
                    }
                })
                .await
                .unwrap();
                let receipt = EffectReceipt {
                    owner: "alice".into(),
                    installation: "installed".into(),
                    generation: 1,
                    action: "fixture_action".into(),
                    operation_id: "view-approval".into(),
                    digest,
                };
                let mut changed = receipt.clone();
                changed.digest = "different-parameters".into();
                assert!(store
                    .app_validation(ValidationCommand::Consume {
                        receipt: changed,
                        now_ms: now
                    })
                    .is_err());
                store
                    .app_validation(ValidationCommand::Consume {
                        receipt: receipt.clone(),
                        now_ms: now,
                    })
                    .unwrap();
                assert!(store
                    .app_validation(ValidationCommand::Consume {
                        receipt,
                        now_ms: now
                    })
                    .is_err());
                if browser {
                    let LocalDaemonResponse::UserAppViewOpened { view: second, .. } = request(&router, "alice", LocalDaemonRequest::OpenUserAppView(OpenUserAppViewRequest {
                        installation_id: "installed".into(), host: Some(UserAppViewHost::KernelBrowser),
                    })).await.unwrap() else { panic!("second App open") };
                    assert_ne!(view.view_id, second.view_id);
                    assert_ne!(view.browser.as_ref().unwrap().tab_id, second.browser.as_ref().unwrap().tab_id);
                    Box::pin(check_browser_page(&router, &second, &observed, 3)).await;
                    Box::pin(check_browser_keyboard(&router, &second, &observed)).await;
                    let second_browser = second.browser.as_ref().unwrap();
                    let stream = browser_request(&router, KernelBrowserCommand::Subscribe {
                        tab_id: second_browser.tab_id.clone(), generation: second_browser.generation,
                    }).await;
                    request(&router, "alice", LocalDaemonRequest::CloseUserAppView(UserAppViewRequest { view_id: second.view_id })).await.unwrap();
                    assert!(request(&router, "alice", LocalDaemonRequest::KernelBrowser(KernelBrowserRequest {
                        command: KernelBrowserCommand::Poll {
                            subscription_id: stream["subscription_id"].as_str().unwrap().into(),
                            generation: stream["generation"].as_u64().unwrap(),
                        },
                    })).await.is_err(), "closed App frame stream is revoked");
                    let state = browser_request(&router, KernelBrowserCommand::State).await;
                    assert_eq!(state["tabs"].as_array().unwrap().len(), 1);
                    assert_eq!(state["tabs"][0]["tab_id"], view.browser.as_ref().unwrap().tab_id);
                }
                let close = LocalDaemonRequest::CloseUserAppView(UserAppViewRequest {
                    view_id: view.view_id.clone(),
                });
                assert!(matches!(
                    request(&router, "bob", close.clone()).await, Err(DaemonError::UserDomainRefused { reason: crate::error::UserDomainRefusalReason::NotGranted })
                ));
                request(&router, "alice", close).await.unwrap();
                assert!(matches!(
                    request(&router, "alice", call).await, Err(DaemonError::UserDomainRefused { reason: crate::error::UserDomainRefusalReason::NotGranted })
                ));
                let LocalDaemonResponse::UserAppViewsListed { views } = request(
                    &router,
                    "alice",
                    LocalDaemonRequest::ListUserAppViews(ListUserAppViewsRequest {}),
                )
                .await
                .unwrap() else {
                    panic!("list");
                };
                assert!(views.is_empty());
                assert!(sessions.list_sessions().is_empty(), "App lifecycle has no sessions");
                if browser {
                    assert!(browser_request(&router, KernelBrowserCommand::State).await["tabs"].as_array().unwrap().is_empty());
                    Box::pin(check_focused_browser_tab(&router, &root, view.browser.as_ref().unwrap().generation)).await;
                }
                })).catch_unwind().await;
                if browser { Box::pin(router.runtime_state.shutdown_cleanup()).await.unwrap(); }
                outcome
            }));
            worker.shutdown_blocking();
            assert!(observed.was_reaped());
            if let Err(error) = outcome { std::panic::resume_unwind(error); }
            if browser {
                let evidence_root = std::path::PathBuf::from(std::env::var_os("CHARIOX_MDINT_DRILL_ROOT").unwrap());
                std::fs::write(evidence_root.join("MDINT-PASS.txt"), "PASS: no-session kernel Chromium App, real signed bundle/page bridge/fixed ABI worker, owner channel context, detached passkey approval and receipt, close, then focused on-demand MCP tab; worker reaped, host shutdown. No model, client/relay projection, Mac or production Vault claim.\n").unwrap();
            }
        })
        .unwrap()
        .join()
        .unwrap();
}

#[tokio::test]
async fn user_app_view_detached_decision_uses_the_sessionless_reply_contract() {
    let app = DaemonApp::bootstrap(DaemonConfig::for_tests()).unwrap();
    let sessions = app.session_state_store();
    let router = CommandRouter::with_interactive_capacity(Arc::new(Mutex::new(app)), 1);
    let receiver = router
        .runtime_state
        .create_kernel_operation_interaction(
            "",
            "alice",
            crate::session::RuntimeInteraction::for_kernel_operation(
                "detached-popup",
                "fixture:detached-popup",
                "Decide",
                "Fixture",
                vec![crate::session::RuntimeInteractionChoice::new(
                    "deny", "Deny", "deny", None,
                )],
            ),
        )
        .await
        .unwrap();
    let reply = LocalDaemonRequest::RespondToInteraction(RespondToInteractionRequest {
        session_id: "".into(),
        interaction_id: "detached-popup".into(),
        choice_id: "deny".into(),
        custom_reply: None,
        passkey: None,
        passkey_remember_minutes: None,
    });
    assert!(request(&router, "bob", reply.clone()).await.is_err());
    let mut host = remote_command_for_request(&reply, Some("alice"));
    host.caller.connection_class = Some(KernelConnectionClass::Host);
    assert!(router.dispatch(host, reply.clone()).await.is_err());
    // The legacy reply promises a Session in its response. It must reject
    // an empty scope before consuming the detached decision.
    assert!(request(&router, "alice", reply).await.is_err());
    let reply =
        LocalDaemonRequest::AnswerUserDomainInteraction(AnswerUserDomainInteractionRequest {
            interaction_id: "detached-popup".into(),
            choice_id: "deny".into(),
            passkey: None,
            passkey_remember_minutes: None,
        });
    let decision_reply = |id: &str, choice: &str| {
        LocalDaemonRequest::AnswerUserDomainInteraction(AnswerUserDomainInteractionRequest {
            interaction_id: id.into(),
            choice_id: choice.into(),
            passkey: None,
            passkey_remember_minutes: None,
        })
    };
    let expected = serde_json::json!({
        "code": "user_domain_not_granted",
        "message": "User-domain request refused",
        "retryable": false,
    });
    for candidate in [reply.clone(), decision_reply("fabricated-decision", "deny")] {
        let error = request(&router, "bob", candidate).await.unwrap_err();
        let envelope = crate::transport::kernel_protocol::map_kernel_error(&error);
        assert_eq!(serde_json::to_value(envelope).unwrap(), expected);
    }
    // A wrong choice on the owner's live decision remains a validation error.
    let invalid = request(
        &router,
        "alice",
        decision_reply("detached-popup", "invalid"),
    )
    .await
    .unwrap_err();
    assert!(!matches!(invalid, DaemonError::UserDomainRefused { .. }));
    let mut host = remote_command_for_request(&reply, Some("alice"));
    host.caller.connection_class = Some(KernelConnectionClass::Host);
    assert!(matches!(
        router.dispatch(host, reply.clone()).await,
        Err(DaemonError::UserDomainRefused {
            reason: crate::error::UserDomainRefusalReason::NotGranted
        })
    ));
    assert!(matches!(
        request(&router, "alice", reply).await.unwrap(),
        LocalDaemonResponse::UserDomainInteractionAnswered { .. }
    ));
    assert_eq!(receiver.await.unwrap().choice_id.as_deref(), Some("deny"));
    for owner in ["alice", "bob"] {
        let error = request(&router, owner, decision_reply("detached-popup", "deny"))
            .await
            .unwrap_err();
        assert_eq!(
            serde_json::to_value(crate::transport::kernel_protocol::map_kernel_error(&error))
                .unwrap(),
            expected
        );
    }
    let mut critical = router
        .runtime_state
        .create_kernel_operation_interaction(
            "",
            "alice",
            crate::session::RuntimeInteraction::for_kernel_operation(
                "detached-critical",
                "fixture:critical",
                "Approve",
                "Fixture",
                vec![
                    crate::session::RuntimeInteractionChoice::new(
                        "approve", "Approve", "allow", None,
                    )
                    .requiring_passkey(),
                    crate::session::RuntimeInteractionChoice::new("deny", "Deny", "deny", None),
                ],
            ),
        )
        .await
        .unwrap();
    let missing_passkey = request(
        &router,
        "alice",
        decision_reply("detached-critical", "approve"),
    )
    .await
    .unwrap_err();
    assert!(!matches!(
        missing_passkey,
        DaemonError::UserDomainRefused { .. }
    ));
    assert!(missing_passkey.to_string().contains("PASSKEY_REQUIRED"));
    assert!(matches!(
        critical.try_recv(),
        Err(tokio::sync::oneshot::error::TryRecvError::Empty)
    ));
    assert!(sessions.list_sessions().is_empty());
}

fn browser_request(
    router: &CommandRouter,
    command: KernelBrowserCommand,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = serde_json::Value> + Send + '_>> {
    Box::pin(async move {
        let LocalDaemonResponse::KernelBrowser { result } = request(
            router,
            "alice",
            LocalDaemonRequest::KernelBrowser(KernelBrowserRequest { command }),
        )
        .await
        .unwrap() else {
            panic!("browser response")
        };
        result
    })
}

async fn check_browser_page(
    router: &CommandRouter,
    view: &UserAppView,
    observed: &chariox_app_runtime::worker_process::test_fixture::Observation,
    expected_calls: u64,
) {
    use base64::Engine;
    let browser = view
        .browser
        .as_ref()
        .expect("Chromium fallback tab reference");
    assert!(browser.tab_id.starts_with("host-tab-"));
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        loop {
            let page = browser_request(
                router,
                KernelBrowserCommand::Snapshot {
                    tab_id: browser.tab_id.clone(),
                    generation: browser.generation,
                },
            )
            .await;
            if page.to_string().contains("App channel {\\\"ok\\\":true}") {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
    })
    .await
    .expect("actual page bridge call/reply");
    assert_eq!(observed.tool_invocations(), expected_calls);
    let frame = browser_request(
        router,
        KernelBrowserCommand::Screenshot {
            tab_id: browser.tab_id.clone(),
            generation: browser.generation,
        },
    )
    .await;
    let png = base64::engine::general_purpose::STANDARD
        .decode(frame["data_base64"].as_str().unwrap())
        .unwrap();
    assert!(png.starts_with(b"\x89PNG\r\n\x1a\n"));
    let root = std::path::PathBuf::from(std::env::var_os("CHARIOX_MDINT_DRILL_ROOT").unwrap());
    std::fs::write(root.join("MDINT-APP.png"), png).unwrap();
}

async fn check_focused_browser_tab(
    router: &CommandRouter,
    root: &std::path::Path,
    app_generation: u64,
) {
    let workspace = root.join("workspace");
    std::fs::create_dir(&workspace).unwrap();
    let (session, agent, run) = {
        let mut app = router.app.lock().await;
        let (session, agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(
                CreateSessionRequest::new(workspace.to_string_lossy(), workspace.to_string_lossy())
                    .with_owner_user_id("alice"),
            )
            .unwrap();
        let run = launch_test_provider(
            &mut app,
            session.id(),
            agent.id(),
            "dev-stub",
            "dev-stub",
            "default",
        );
        (session, agent, run)
    };
    // The provider binding is a dev stub: this exercises admission, not a model.
    let credential = run.runtime_mcp_auth_token().unwrap();
    let focus = LocalDaemonRequest::FocusAgent(FocusAgentRequest {
        session_id: session.id().into(),
        agent_id: agent.id().into(),
    });
    request(router, "alice", focus).await.unwrap();
    // The same owner/profile serves this user tab after its App view closes.
    router
        .dispatch_authenticated_runtime_tool_call(
            credential,
            "chariox.load_kernel_browser",
            serde_json::json!({}),
        )
        .await
        .unwrap();
    let opened = router
        .dispatch_authenticated_runtime_tool_call(
            credential,
            "chariox.kernel_browser",
            serde_json::json!({"command":{"op":"open","url":"about:blank"}}),
        )
        .await
        .unwrap();
    let tab = opened.payload["tab_id"].as_str().unwrap();
    let generation = opened.payload["generation"].as_u64().unwrap();
    assert!(tab.starts_with("host-tab-"));
    assert_eq!(
        generation, app_generation,
        "App and focused tab use the same owner browser"
    );
    let state = router
        .dispatch_authenticated_runtime_tool_call(
            credential,
            "chariox.kernel_browser",
            serde_json::json!({"command":{"op":"state"}}),
        )
        .await
        .unwrap();
    assert!(state.payload["tabs"]
        .as_array()
        .unwrap()
        .iter()
        .any(|t| t["tab_id"] == tab));
    router
        .dispatch_authenticated_runtime_tool_call(
            credential,
            "chariox.kernel_browser",
            serde_json::json!({"command":{"op":"close","tab_id":tab,"generation":generation}}),
        )
        .await
        .unwrap();
}

// The TUI forwards human keys over this shared protocol, never through a Room.
async fn check_browser_keyboard(
    router: &CommandRouter,
    view: &UserAppView,
    observed: &chariox_app_runtime::worker_process::test_fixture::Observation,
) {
    let browser = view.browser.as_ref().unwrap();
    for input in [
        KernelBrowserInput::Key { key: "Tab".into() },
        KernelBrowserInput::Text {
            text: "TUI keyboard fixture".into(),
        },
        KernelBrowserInput::Key {
            key: "Enter".into(),
        },
    ] {
        browser_request(
            router,
            KernelBrowserCommand::Input {
                tab_id: browser.tab_id.clone(),
                generation: browser.generation,
                input,
            },
        )
        .await;
    }
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        loop {
            let page = browser_request(
                router,
                KernelBrowserCommand::Snapshot {
                    tab_id: browser.tab_id.clone(),
                    generation: browser.generation,
                },
            )
            .await;
            if page
                .to_string()
                .contains("Keyboard App channel {\\\"ok\\\":true}")
            {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
    })
    .await
    .expect("human keyboard App channel reply in accessibility projection");
    assert_eq!(observed.tool_invocations(), 4);
    let last = observed.tool_requests().unwrap().last().unwrap().clone();
    assert_eq!(last["params"]["input"]["text"], "TUI keyboard fixture");
    assert_eq!(
        last["context"]["actor"],
        serde_json::json!({"kind":"human","id":"alice"})
    );
}
