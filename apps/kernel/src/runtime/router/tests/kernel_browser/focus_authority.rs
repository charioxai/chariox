//! MP-08/MP-11: orchestration focus must never mint human user-domain authority.
use super::*;
use crate::local::KernelConnectionClass;

#[test]
fn mdaccess_agent_focus_attach_cycle_cannot_create_or_revive_grants() {
    run_test(|| Box::pin(check()));
}

async fn check() {
    let workspace = crate::test_support::TestWorktree::new("mdaccess-focus-authority");
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).unwrap();
    let (session, first) = crate::app::KernelSessionService::new(&mut app)
        .create_session(workspace.session_request())
        .unwrap();
    let second = spawn_test_agent(&mut app, session.id(), "second", "dev-stub");
    let meta = spawn_test_agent(&mut app, session.id(), "controller", "dev-stub");
    app.agents_mut()
        .activate_agent_meta_mode(meta.id(), None)
        .unwrap();
    let router = CommandRouter::with_interactive_capacity(Arc::new(Mutex::new(app)), 4);
    for (class, meta_origin) in [
        (KernelConnectionClass::KernelAgent, true),
        (KernelConnectionClass::KernelAgent, false),
        (KernelConnectionClass::ExternalAgent, false),
        (KernelConnectionClass::Terminal, true),
        (KernelConnectionClass::Host, false),
        (KernelConnectionClass::RelayPeer, false),
        (KernelConnectionClass::Unauthenticated, false),
    ] {
        for revoked in [false, true] {
            if revoked {
                focus(&router, session.id(), first.id()).await;
                focus(&router, session.id(), second.id()).await;
                human(
                    &router,
                    KernelBrowserCommand::RevokeGrants { agent_id: None },
                )
                .await;
            }
            for request in [
                LocalDaemonRequest::FocusAgent(FocusAgentRequest {
                    session_id: session.id().into(),
                    agent_id: first.id().into(),
                }),
                LocalDaemonRequest::AttachToSession(crate::local::AttachToSessionRequest {
                    session_id: session.id().into(),
                    client_id: "agent-orchestration".into(),
                    capability_level: crate::attachment::ClientCapabilityLevel::AutomationOnly,
                }),
                LocalDaemonRequest::CycleAgentFocus(crate::local::CycleAgentFocusRequest {
                    session_id: session.id().into(),
                }),
            ] {
                let mut command =
                    KernelCommand::from_local_request("MP-11-agent-focus", None, None, &request);
                command.caller.connection_class = Some(class);
                if meta_origin {
                    command.caller.caller_kind =
                        crate::runtime::command::KernelCallerKind::Metaagent;
                    command.caller.metaagent_id = Some(meta.id().into());
                }
                let result = router.dispatch(command, request).await;
                if class != KernelConnectionClass::ExternalAgent {
                    result.unwrap();
                } else {
                    assert!(result.unwrap_err().to_string().contains("kernel access"));
                }
                assert!(
                    human(&router, KernelBrowserCommand::ListGrants).await["grants"]
                        .as_array()
                        .unwrap()
                        .is_empty(),
                    "MP-11: {class:?} must not create or revive a grant (revoked={revoked})"
                );
            }
        }
    }
    // A human terminal can still grant and restore access on the same session.
    focus(&router, session.id(), first.id()).await;
    assert_eq!(
        human(&router, KernelBrowserCommand::ListGrants).await["grants"][0]["agent_id"],
        first.id()
    );
    focus(&router, session.id(), second.id()).await;
    let request = LocalDaemonRequest::FocusAgent(FocusAgentRequest {
        session_id: session.id().into(),
        agent_id: first.id().into(),
    });
    let mut command =
        KernelCommand::from_local_request("MP-11-retained-focus", None, None, &request);
    command.caller.connection_class = Some(KernelConnectionClass::KernelAgent);
    router.dispatch(command, request).await.unwrap();
    let grants = human(&router, KernelBrowserCommand::ListGrants).await;
    assert_eq!(
        grants["grants"]
            .as_array()
            .unwrap()
            .iter()
            .find(|g| g["agent_id"] == first.id())
            .unwrap()["focused"],
        false,
        "MP-11: agent orchestration cannot refresh sensitive focus authority"
    );
    router.runtime_state().shutdown_cleanup().await.unwrap();
}

#[test]
fn mdaccess_agent_alias_prompt_cannot_mint_grant() {
    run_test(|| {
        Box::pin(async {
            let workspace = crate::test_support::TestWorktree::new("mdaccess-alias-authority");
            let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).unwrap();
            let (session, first) = crate::app::KernelSessionService::new(&mut app)
                .create_session(workspace.session_request())
                .unwrap();
            let second = spawn_test_agent(&mut app, session.id(), "alias-target", "dev-stub");
            launch_test_provider(
                &mut app,
                session.id(),
                second.id(),
                "dev-stub",
                "dev-stub",
                "default",
            );
            let attachment = crate::app::KernelSessionService::new(&mut app)
                .attach(crate::attachment::AttachRequest::new(
                    session.id(),
                    "agent-client",
                    crate::attachment::ClientCapabilityLevel::AutomationOnly,
                ))
                .unwrap();
            let router = CommandRouter::with_interactive_capacity(Arc::new(Mutex::new(app)), 4);
            focus(&router, session.id(), first.id()).await;
            let request = LocalDaemonRequest::SubmitPrompt(crate::local::SubmitPromptRequest {
                session_id: session.id().into(),
                attachment_id: attachment.id().into(),
                target_agent_id: None,
                prompt: "@alias-target ordinary worker prompt".into(),
                attachments: vec![],
            });
            let mut command =
                KernelCommand::from_local_request("MP-11-agent-alias", None, None, &request);
            command.caller.connection_class = Some(KernelConnectionClass::KernelAgent);
            router.dispatch(command, request).await.unwrap();
            let grants = human(&router, KernelBrowserCommand::ListGrants).await;
            assert!(
                !grants["grants"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|g| g["agent_id"] == second.id()),
                "MP-11: alias-routed agent prompt cannot mint a browser/Notes grant"
            );
            router.runtime_state().shutdown_cleanup().await.unwrap();
        })
    });
}
