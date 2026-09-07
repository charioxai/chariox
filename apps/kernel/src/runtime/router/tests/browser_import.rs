use super::*;
use serde_json::{json, Value};

#[test]
fn browser_import_consent_requires_verified_owner_and_unchanged_selection() {
    std::thread::Builder::new()
        .stack_size(64 * 1024 * 1024)
        .spawn(|| {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap()
                .block_on(consent_round_trip());
        })
        .unwrap()
        .join()
        .unwrap();
}

struct Scratch(std::path::PathBuf);
impl Drop for Scratch {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).unwrap();
    }
}

async fn consent_round_trip() {
    let config = DaemonConfig::for_tests();
    let _scratch = Scratch(
        std::path::PathBuf::from(config.user_config.state.path.as_ref().unwrap())
            .parent()
            .unwrap()
            .to_path_buf(),
    );
    let mut app = DaemonApp::bootstrap(config).unwrap();
    let session = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new("browser-import", "worktree"))
        .unwrap()
        .0;
    let attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(crate::attachment::AttachRequest::new(
            session.id(),
            "import-client",
            ClientCapabilityLevel::FullTerminal,
        ))
        .unwrap();
    let router = CommandRouter::with_interactive_capacity(Arc::new(Mutex::new(app)), 2);
    let state = &router.runtime_state;
    state
        .start_room_environment(
            session.id(),
            crate::session::CanonicalViewport::new(800, 600, 1, 800, 600).unwrap(),
        )
        .unwrap();
    state
        .transition_room_environment(session.id(), crate::session::EnvironmentLifecycle::Ready)
        .unwrap();
    let environment = state
        .reconcile_room_environment_controller_tabs(
            session.id(),
            vec![crate::session::EnvironmentTabObservation {
                runtime_target_id: "target-1".into(),
                document_id: "doc-1".into(),
                url: "https://example.com/".into(),
                title: "fixture".into(),
            }],
            Some("target-1"),
        )
        .unwrap();
    let selection = json!({
        "session_id": session.id(), "attachment_id": attachment.id(),
        "environment_id": environment.environment_id,
        "runtime_generation": environment.runtime_generation,
        "tab_id": environment.tabs[0].tab_id,
        "document_revision": environment.tabs[0].document_revision,
        "source_store_id": "0", "domains": ["example.com"],
        "partition_sites": [], "overwrite": false
    });
    let caller = KernelCaller {
        caller_id: "import-client".into(),
        caller_kind: KernelCallerKind::RemoteClient,
        user_id: Some(DEFAULT_LOCAL_USER_ID.into()),
        client_id: Some("import-client".into()),
        machine_id: None,
        realm_id: Some("test-realm".into()),
        public_key_thumbprint: Some("test-client-key".into()),
        metaagent_id: None,
    };
    let prepare = json!({"PrepareBrowserImport": {"selection": selection}});
    let mut unverified = caller.clone();
    unverified.public_key_thumbprint = None;
    assert!(dispatch(&router, &unverified, prepare.clone())
        .await
        .is_err());
    let response = dispatch(&router, &caller, prepare.clone()).await.unwrap();
    let id = response["BrowserImportConsent"]["request_id"]
        .as_str()
        .unwrap();
    assert_eq!(response["BrowserImportConsent"]["status"], "prepared");
    assert!(
        dispatch(&router, &caller, prepare.clone()).await.is_err(),
        "only one pending import per Environment"
    );
    let approve = json!({"ApproveBrowserImport": {"request_id": id, "selection": selection}});
    let mut rotated = caller.clone();
    rotated.public_key_thumbprint = Some("replacement-key".into());
    assert!(dispatch(&router, &rotated, approve.clone()).await.is_err());
    rotated = caller.clone();
    rotated.realm_id = Some("other-realm".into());
    assert!(dispatch(&router, &rotated, approve.clone()).await.is_err());
    let mut wrong_client = caller.clone();
    wrong_client.client_id = Some("other-client".into());
    assert!(dispatch(&router, &wrong_client, approve.clone())
        .await
        .is_err());
    let mut wrong_scope = approve.clone();
    wrong_scope["ApproveBrowserImport"]["selection"]["overwrite"] = json!(true);
    assert!(dispatch(&router, &caller, wrong_scope).await.is_err());
    let mut agent = caller.clone();
    agent.caller_kind = KernelCallerKind::Metaagent;
    assert!(dispatch(&router, &agent, approve.clone()).await.is_err());
    agent.caller_kind = KernelCallerKind::HostedService;
    assert!(dispatch(&router, &agent, approve.clone()).await.is_err());
    agent = caller.clone();
    agent.user_id = Some("other-user".into());
    assert!(dispatch(&router, &agent, approve.clone()).await.is_err());
    for (field, value) in [
        ("source_store_id", json!("other-store")),
        ("domains", json!(["other.example"])),
        ("partition_sites", json!(["https://other.example"])),
        ("runtime_generation", json!(99)),
        ("environment_id", json!("wrong-environment")),
        ("tab_id", json!("wrong-tab")),
    ] {
        let mut changed = approve.clone();
        changed["ApproveBrowserImport"]["selection"][field] = value;
        assert!(
            dispatch(&router, &caller, changed).await.is_err(),
            "changed {field} cannot reuse consent"
        );
    }
    assert_eq!(
        dispatch(&router, &caller, approve.clone()).await.unwrap()["BrowserImportConsent"]
            ["status"],
        "approved"
    );
    assert!(
        dispatch(&router, &caller, approve).await.is_err(),
        "approval cannot replay"
    );
    let cancel = json!({"CancelBrowserImport": {"session_id": session.id(), "attachment_id": attachment.id(), "request_id": id}});
    assert_eq!(
        dispatch(&router, &caller, cancel).await.unwrap()["BrowserImportConsent"]["status"],
        "cancelled"
    );
    let response = dispatch(&router, &caller, prepare).await.unwrap();
    let next_id = response["BrowserImportConsent"]["request_id"]
        .as_str()
        .unwrap();
    state
        .reconcile_room_environment_controller_tabs(
            session.id(),
            vec![crate::session::EnvironmentTabObservation {
                runtime_target_id: "target-1".into(),
                document_id: "doc-2".into(),
                url: "https://example.com/next".into(),
                title: "next".into(),
            }],
            Some("target-1"),
        )
        .unwrap();
    assert!(
        dispatch(
            &router,
            &caller,
            json!({"ApproveBrowserImport": {"request_id": next_id, "selection": selection}})
        )
        .await
        .is_err(),
        "navigation invalidates consent"
    );
    let cancel = json!({"CancelBrowserImport": {"session_id": session.id(), "attachment_id": attachment.id(), "request_id": next_id}});
    assert!(
        dispatch(&router, &caller, cancel).await.is_ok(),
        "navigation must not prevent cancellation"
    );
    let mut current = selection.clone();
    current["document_revision"] =
        json!(state.room_environment_snapshot(session.id()).unwrap().tabs[0].document_revision);
    let local = KernelCaller::for_source(&KernelCommandSource::LocalIpc);
    let request = json!({"PrepareBrowserImport": {"selection": current}});
    let response = dispatch_source(&router, &local, request, KernelCommandSource::LocalIpc)
        .await
        .unwrap();
    let local_id = response["BrowserImportConsent"]["request_id"]
        .as_str()
        .unwrap();
    let request = json!({"ApproveBrowserImport": {"request_id": local_id, "selection": current}});
    assert!(
        dispatch(&router, &caller, request.clone()).await.is_err(),
        "relay cannot approve local consent"
    );
    assert!(
        dispatch_source(&router, &local, request, KernelCommandSource::LocalIpc)
            .await
            .is_ok()
    );
}

async fn dispatch(
    router: &CommandRouter,
    caller: &KernelCaller,
    value: Value,
) -> Result<Value, DaemonError> {
    dispatch_source(router, caller, value, KernelCommandSource::RelayClient).await
}

async fn dispatch_source(
    router: &CommandRouter,
    caller: &KernelCaller,
    value: Value,
    source: KernelCommandSource,
) -> Result<Value, DaemonError> {
    let request: LocalDaemonRequest =
        serde_json::from_value(value).expect("browser import request must decode");
    let command = KernelCommand::from_local_request_with_caller(
        "browser-import-test",
        source,
        caller.clone(),
        None,
        None,
        &request,
    );
    router
        .dispatch(command, request)
        .await
        .map(|response| serde_json::to_value(response).unwrap())
}
